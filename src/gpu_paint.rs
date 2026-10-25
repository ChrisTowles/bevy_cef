//! Whether CEF's GPU process can paint pages on the GPU here (Linux `CefPlugin::gpu_paint`),
//! checked before CEF starts: when it cannot, Chromium restarts the GPU process until it gives
//! up and exits the whole app ("GPU process isn't usable. Goodbye."), stalling the app's main
//! thread on every restart until then.
//!
//! The checks follow what the GPU process does with `gpu_paint`'s switches: X11 ozone opens the
//! display, and ANGLE on Vulkan loads `libvulkan.so.1` from libcef.so's own directory only
//! (`ANGLE_USE_CUSTOM_LIBVULKAN`), creates a Vulkan 1.1 instance and exports each frame as a
//! dmabuf.

use ash::{ext, khr, vk};
use std::ffi::{CStr, c_char, c_int, c_void};
use std::path::{Path, PathBuf};

/// The device extensions ANGLE needs to hand a frame over as a dmabuf.
const DEVICE_EXTENSIONS: [&CStr; 3] = [
    khr::external_memory_fd::NAME,
    ext::external_memory_dma_buf::NAME,
    ext::image_drm_format_modifier::NAME,
];

/// Why the GPU process could not paint here, if it could not.
pub(crate) fn preflight() -> Result<(), String> {
    x11_display()?;
    let dir = libcef_dir().ok_or("libcef.so is not mapped into this process")?;
    let loader = dir.join("libvulkan.so.1");
    if !loader.exists() {
        return Err(format!(
            "{} is missing: CEF's ANGLE loads Vulkan only from libcef.so's directory",
            loader.display()
        ));
    }
    vulkan(&loader)
}

/// libcef.so's directory, from this process's mappings (it is linked, so already loaded).
fn libcef_dir() -> Option<PathBuf> {
    let maps = std::fs::read_to_string("/proc/self/maps").ok()?;
    maps.lines()
        .filter_map(|line| line.find('/').map(|i| Path::new(&line[i..])))
        .find(|path| path.file_name().is_some_and(|name| name == "libcef.so"))
        .and_then(Path::parent)
        .map(Path::to_path_buf)
}

/// CEF's own loader brings up a Vulkan 1.1 instance with a device that exports dmabufs. The
/// loader is unloaded again when this returns (the app's own Vulkan keeps the system one).
fn vulkan(loader: &Path) -> Result<(), String> {
    // SAFETY: the library is a Vulkan loader (CEF ships it); nothing created here outlives it.
    let entry = unsafe { ash::Entry::load_from(loader) }
        .map_err(|e| format!("{}: {e}", loader.display()))?;
    let app_info = vk::ApplicationInfo::default().api_version(vk::API_VERSION_1_1);
    let create_info = vk::InstanceCreateInfo::default().application_info(&app_info);
    // SAFETY: valid create info; the instance is destroyed below.
    let instance = unsafe { entry.create_instance(&create_info, None) }
        .map_err(|e| format!("vkCreateInstance (Vulkan 1.1, {}): {e}", loader.display()))?;
    // SAFETY: the devices belong to `instance`, which is alive.
    let found = unsafe { instance.enumerate_physical_devices() }
        .map_err(|e| format!("vkEnumeratePhysicalDevices: {e}"))
        .map(|devices| {
            devices.into_iter().any(|device| {
                let extensions =
                    unsafe { instance.enumerate_device_extension_properties(device) }
                        .unwrap_or_default();
                DEVICE_EXTENSIONS.iter().all(|wanted| {
                    extensions
                        .iter()
                        .any(|e| e.extension_name_as_c_str().is_ok_and(|name| name == *wanted))
                })
            })
        });
    // SAFETY: nothing created from the instance is alive.
    unsafe { instance.destroy_instance(None) };
    match found? {
        true => Ok(()),
        false => Err(format!("no Vulkan device has {DEVICE_EXTENSIONS:?}")),
    }
}

/// `gpu_paint` runs the GPU process on X11 ozone, which needs the X display.
fn x11_display() -> Result<(), String> {
    type XOpenDisplay = unsafe extern "C" fn(*const c_char) -> *mut c_void;
    type XCloseDisplay = unsafe extern "C" fn(*mut c_void) -> c_int;
    // SAFETY: libX11's documented entry points, called with their C signatures.
    unsafe {
        let lib = libloading::Library::new("libX11.so.6").map_err(|e| format!("libX11: {e}"))?;
        let open = lib
            .get::<XOpenDisplay>(b"XOpenDisplay\0")
            .map_err(|e| format!("libX11: {e}"))?;
        let close = lib
            .get::<XCloseDisplay>(b"XCloseDisplay\0")
            .map_err(|e| format!("libX11: {e}"))?;
        let display = open(std::ptr::null());
        if display.is_null() {
            return Err(format!(
                "cannot open the X display (DISPLAY={:?}), which gpu_paint's X11 ozone needs",
                std::env::var("DISPLAY").unwrap_or_default()
            ));
        }
        close(display);
    }
    Ok(())
}
