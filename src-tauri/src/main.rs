// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // Wayland doesn't let apps place their own windows, so the tray mini player couldn't open under
    // the tray icon. Run through XWayland; PIXELPLAY_WAYLAND=1 opts out. GDK_BACKEND is overridden on
    // purpose: some terminals export GDK_BACKEND=wayland to every shell.
    #[cfg(target_os = "linux")]
    if std::env::var_os("WAYLAND_DISPLAY").is_some() && std::env::var_os("PIXELPLAY_WAYLAND").is_none() {
        // SAFETY: single-threaded at this point, before Tauri/GTK start.
        unsafe { std::env::set_var("GDK_BACKEND", "x11") };
    }
    pixelplay_desktop_lib::run()
}
