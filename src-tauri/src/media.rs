#[cfg(desktop)]
use souvlaki::{
    MediaControlEvent, MediaControls, MediaMetadata, MediaPlayback, MediaPosition, PlatformConfig,
};
use std::sync::mpsc;
#[cfg(desktop)]
use std::{ffi::c_void, time::Duration};
#[cfg(windows)]
use tauri::Manager;
use tauri::{AppHandle, Emitter};

enum Msg {
    Meta { title: String, artist: String, album: String, secs: f64 },
    Playback(Option<(bool, f64)>),
}

/// OS media controls (MPRIS / SMTC / MediaRemote). The controls live on their own thread
/// because the platform handles are not guaranteed to be `Send`.
pub struct Media(mpsc::Sender<Msg>);

pub fn init(app: &AppHandle) -> Media {
    let (tx, rx) = mpsc::channel();
    #[cfg(windows)]
    let hwnd = app.get_webview_window("main").and_then(|w| w.hwnd().ok()).map(|h| h.0 as usize);
    #[cfg(not(windows))]
    let hwnd: Option<usize> = None;
    let handle = app.clone();

    #[cfg(mobile)]
    {
        let _ = (app, &rx);
        std::thread::spawn(move || for _ in rx {});
    }
    #[cfg(desktop)]
    std::thread::spawn(move || {
        // Unique per process (MPRIS "instance" convention): a taken name makes souvlaki panic,
        // which aborts the whole app in release builds (panic = "abort").
        let dbus_name = format!("neptunus.instance{}", std::process::id());
        let cfg = PlatformConfig {
            display_name: "Neptunus",
            dbus_name: &dbus_name,
            hwnd: hwnd.map(|h| h as *mut c_void),
        };
        let Ok(mut controls) = MediaControls::new(cfg) else { return };
        let _ = controls.attach(move |e| match e {
            MediaControlEvent::Play => { let _ = handle.emit("media-key", "play"); }
            MediaControlEvent::Pause => { let _ = handle.emit("media-key", "pause"); }
            MediaControlEvent::Toggle => { let _ = handle.emit("media-key", "toggle"); }
            MediaControlEvent::Next => { let _ = handle.emit("media-key", "next"); }
            MediaControlEvent::Previous => { let _ = handle.emit("media-key", "prev"); }
            MediaControlEvent::SetPosition(p) => { let _ = handle.emit("media-seek", p.0.as_secs_f64()); }
            _ => {}
        });
        for msg in rx {
            let _ = match msg {
                Msg::Meta { title, artist, album, secs } => controls.set_metadata(MediaMetadata {
                    title: Some(&title),
                    artist: Some(&artist),
                    album: Some(&album),
                    duration: Some(Duration::from_secs_f64(secs)),
                    cover_url: None,
                }),
                Msg::Playback(p) => controls.set_playback(match p {
                    None => MediaPlayback::Stopped,
                    Some((playing, pos)) => {
                        let progress = Some(MediaPosition(Duration::from_secs_f64(pos)));
                        if playing { MediaPlayback::Playing { progress } } else { MediaPlayback::Paused { progress } }
                    }
                }),
            };
        }
    });
    Media(tx)
}

#[tauri::command]
pub fn media_metadata(app: tauri::AppHandle, m: tauri::State<Media>, path: String, title: String, artist: String, album: String, duration_secs: f64) {
    crate::tray::set_track(&app, &path, &title, &artist);
    let _ = m.0.send(Msg::Meta { title, artist, album, secs: duration_secs });
}

#[tauri::command]
pub fn media_playback(app: tauri::AppHandle, m: tauri::State<Media>, playing: Option<bool>, position: f64) {
    crate::tray::set_playing(&app, playing);
    let _ = m.0.send(Msg::Playback(playing.map(|p| (p, position))));
}
