// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // Native Wayland is the default: under XWayland the window sometimes stays unclickable (stuck pointer
    // grab). Wayland lets apps position nothing, so the tray mini player can't open under the tray icon
    // there; NEPTUNUS_X11=1 runs through XWayland to get that back. GDK_BACKEND is overridden on purpose:
    // some terminals export it to every shell.
    #[cfg(target_os = "linux")]
    if std::env::var_os("WAYLAND_DISPLAY").is_some() {
        let backend = if std::env::var_os("NEPTUNUS_X11").is_some() { "x11" } else { "wayland" };
        // SAFETY: single-threaded at this point, before Tauri/GTK start.
        unsafe { std::env::set_var("GDK_BACKEND", backend) };
    }
    neptunus_lib::run()
}
