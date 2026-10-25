#![allow(clippy::type_complexity)]

mod common;
mod cursor_icon;
mod drag;
mod focus;
mod keyboard;
mod mute;
mod navigation;
mod profile;
mod title;
mod webview;
mod zoom;

use crate::common::{
    CefRootCachePath, CefStatus, LocalHostPlugin, MessageLoopPlugin, SandboxMode,
    WebviewCoreComponentsPlugin, WebviewDpiPlugin, add_failed_startup, resolve_no_sandbox,
};
use crate::cursor_icon::SystemCursorIconPlugin;
use crate::drag::DragPlugin;
use crate::focus::FocusPlugin;
use crate::keyboard::KeyboardPlugin;
use crate::mute::AudioMutePlugin;
use crate::prelude::{IpcPlugin, NavigationPlugin, WebviewPlugin};
use crate::profile::ProfilePlugin;
use crate::title::TitlePlugin;
use crate::zoom::ZoomPlugin;
use bevy::prelude::*;
use bevy_cef_core::prelude::{
    CefCustomScheme, CefExtensions, CommandLineConfig, effective_command_line_config, switches,
};
#[cfg(feature = "brp")]
use bevy::remote::RemotePlugin;
use std::time::Duration;

pub mod prelude {
    pub use crate::common::{CefStartupError, CefStatus};
    pub use crate::focus::FocusedWebview;
    pub use crate::keyboard::{CefKeyboardFilter, KeyboardDeliverSet, ModifiersState};
    pub use crate::profile::{
        ProfileName, ProfileNameError, WebviewProfile, WebviewProfileError,
        WebviewProfileErrorReason,
    };
    pub use crate::{CefPlugin, RunOnMainThread, navigation::*, title::*, webview::prelude::*};
    pub use bevy_cef_core::prelude::{
        Browsers, RenderTextureMessage, WebviewPainted, CefCustomScheme, create_cef_key_events,
        keyboard_modifiers, CefExtensions, CefSchemeBody, CefSchemeHandler, CefSchemeOptions,
        CefSchemeRequest, CefSchemeResponse, CommandLineConfig, switches,
    };
    #[cfg(target_os = "linux")]
    pub use bevy_cef_core::prelude::DmabufPaint;
}

pub struct RunOnMainThread;

pub struct CefPlugin {
    pub command_line_config: CommandLineConfig,
    pub extensions: CefExtensions,
    /// Root directory for CEF runtime data (cache, profiles, etc.).
    /// If empty, defaults to the executable's directory.
    /// Should be set to a user-writable path (e.g. `~/.myapp/cef_data`).
    pub root_cache_path: Option<String>,
    /// Custom URL schemes to register in addition to the built-in
    /// `cef://localhost/`. Each carries a handler that services requests.
    pub custom_schemes: Vec<CefCustomScheme>,
    /// Controls Chromium's OS-level sandbox. Defaults to the current per-platform
    /// behavior; see [`SandboxMode`].
    pub sandbox: SandboxMode,
    /// How often the cookie stores of profile contexts are flushed to disk, measured in
    /// wall-clock time. A flush also follows every main-frame load end in a profile webview.
    /// Defaults to 5 seconds.
    pub cookie_flush_interval: Duration,
    /// Linux: CEF composites pages on the GPU and hands each frame over as a dmabuf
    /// (`WebviewGpuPainted`) instead of copying its pixels to the CPU, for an app that shows
    /// it straight from GPU memory. Adds the switches CEF's GPU path needs (X11 ozone, ANGLE
    /// on Vulkan), so not with `ozone-platform=headless`. Off by default.
    pub gpu_paint: bool,
}

impl Default for CefPlugin {
    fn default() -> Self {
        Self {
            command_line_config: Default::default(),
            extensions: Default::default(),
            root_cache_path: Default::default(),
            custom_schemes: Default::default(),
            sandbox: Default::default(),
            cookie_flush_interval: Duration::from_secs(5),
            gpu_paint: false,
        }
    }
}

impl Plugin for CefPlugin {
    fn build(&self, app: &mut App) {
        // NOTE: Must run before MessageLoopPlugin::start, which calls cef_initialize.
        // CEF's OnRegisterCustomSchemes fires during initialize; schemes registered
        // afterward are silently ignored.
        bevy_cef_core::prelude::init_registered_schemes(self.custom_schemes.clone());

        // Resolve the sandbox decision and compute the effective command line once.
        let no_sandbox = resolve_no_sandbox(self.sandbox);
        let strip_no_zygote = cfg!(target_os = "linux") && !no_sandbox;
        let mut command_line_config = self.command_line_config.clone();
        if cfg!(target_os = "linux") && self.gpu_paint {
            // Chromium's GPU process on Wayland (or GL) cannot hand a native pixmap to its
            // compositor on NVIDIA; X11 with ANGLE on Vulkan can.
            // Chromium keeps only the last value of a repeated switch, so these replace the
            // app's own `ozone-platform` / `use-angle` and join its `enable-features`.
            let values = &mut command_line_config.switch_values;
            for (name, value) in [("ozone-platform", "x11"), ("use-angle", "vulkan")] {
                if let Some(old) = values.iter().find(|(n, v)| *n == name && *v != value) {
                    warn!("gpu_paint: {name}={} replaced by {name}={value}", old.1);
                }
                values.retain(|(n, _)| *n != name);
                values.push((name, value));
            }
            let mut features = vec!["Vulkan,DefaultANGLEVulkan,VulkanFromANGLE"];
            features.extend(
                values
                    .iter()
                    .filter(|(n, _)| *n == "enable-features")
                    .map(|(_, v)| *v),
            );
            values.retain(|(n, _)| *n != "enable-features");
            values.push(("enable-features", features.join(",").leak()));
        }
        let effective_config =
            effective_command_line_config(&command_line_config, strip_no_zygote);

        // Warn when any security-relaxing switch is active (based on the EFFECTIVE set).
        let risky = switches::risky_present(&effective_config.switches);
        if !risky.is_empty() {
            warn!(
                "bevy_cef: web-security relaxations active: {risky:?}. \
                 Only enable these when loading fully trusted content."
            );
        }

        // Warn when the sandbox is requested but its prerequisites are absent on macOS
        // (the render process is not linked against cef_sandbox).
        #[cfg(target_os = "macos")]
        if self.sandbox == SandboxMode::Enabled {
            warn!(
                "bevy_cef: SandboxMode::Enabled requested on macOS, but the render \
                 process is not linked against cef_sandbox / does not call \
                 cef_sandbox_initialize(); the sandbox will not function and may abort. \
                 See the plugin-configuration docs."
            );
        }

        // Warn when the sandbox is requested on Linux, which requires a SUID-root
        // `chrome-sandbox` helper installed alongside the render process.
        #[cfg(target_os = "linux")]
        if self.sandbox == SandboxMode::Enabled {
            warn!(
                "bevy_cef: SandboxMode::Enabled requested on Linux; the sandbox requires a \
                 correctly-installed SUID-root chrome-sandbox helper alongside the render \
                 process. Without it, Chromium aborts the renderer with no bevy_cef-side \
                 diagnostic. See the plugin-configuration docs."
            );
        }

        let message_loop = MessageLoopPlugin {
            config: effective_config,
            extensions: self.extensions.clone(),
            root_cache_path: self.root_cache_path.clone(),
            no_sandbox,
        };
        if let Err(e) = message_loop.start(app) {
            error!("bevy_cef: CEF failed to start: {e}");
            add_failed_startup(app, e, self.root_cache_path.clone());
            return;
        }
        app.insert_resource(CefStatus::Running)
            .insert_resource(CefRootCachePath(self.root_cache_path.clone()))
            .add_plugins((
                LocalHostPlugin,
                message_loop,
                WebviewCoreComponentsPlugin,
                WebviewDpiPlugin,
                WebviewPlugin,
                IpcPlugin,
                KeyboardPlugin,
                FocusPlugin,
                SystemCursorIconPlugin,
                DragPlugin,
                NavigationPlugin,
                TitlePlugin,
                ZoomPlugin,
                AudioMutePlugin,
            ))
            .add_plugins(ProfilePlugin {
                cookie_flush_interval: self.cookie_flush_interval,
            });
        #[cfg(feature = "brp")]
        if !app.is_plugin_added::<RemotePlugin>() {
            app.add_plugins(RemotePlugin::default());
        }
    }

    fn finish(&self, app: &mut App) {
        #[cfg(target_os = "linux")]
        if let Some(mut browsers) = app
            .world_mut()
            .get_non_send_mut::<bevy_cef_core::prelude::Browsers>() {
            browsers.gpu_paint = self.gpu_paint;
        }
    }
}
