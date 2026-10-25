//! Whether CEF's GPU process can paint pages on the GPU here (Linux `CefPlugin::gpu_paint`),
//! checked before CEF starts: when it cannot, Chromium restarts the GPU process until it gives
//! up and exits the whole app ("GPU process isn't usable. Goodbye."), stalling the app's main
//! thread on every restart until then.
//!
//! `gpu_paint` runs the GPU process on Wayland ozone (ANGLE on GL/EGL), so it needs the
//! compositor's socket.

use std::os::unix::net::UnixStream;
use std::path::PathBuf;

/// Why the GPU process could not paint here, if it could not.
pub(crate) fn preflight() -> Result<(), String> {
    let display = std::env::var("WAYLAND_DISPLAY")
        .map_err(|_| "WAYLAND_DISPLAY is not set; gpu_paint's Wayland ozone needs it".to_string())?;
    let socket = if display.starts_with('/') {
        PathBuf::from(&display)
    } else {
        let runtime = std::env::var("XDG_RUNTIME_DIR")
            .map_err(|_| "XDG_RUNTIME_DIR is not set; the Wayland socket lives there".to_string())?;
        PathBuf::from(runtime).join(&display)
    };
    UnixStream::connect(&socket)
        .map(drop)
        .map_err(|e| format!("cannot reach the Wayland compositor at {}: {e}", socket.display()))
}
