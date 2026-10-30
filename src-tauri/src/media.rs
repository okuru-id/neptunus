#[cfg(desktop)]
use souvlaki::{
    MediaControlEvent, MediaControls, MediaMetadata, MediaPlayback, MediaPosition, PlatformConfig,
};
use std::sync::mpsc;
#[cfg(desktop)]
use std::{ffi::c_void, time::Duration};
#[cfg(windows)]
use tauri::Manager;
use tauri::AppHandle;
#[cfg(any(desktop, target_os = "ios"))]
use tauri::Emitter;

enum Msg {
    Meta { path: String, title: String, artist: String, album: String, secs: f64, cover: Option<String> },
    /// The song's cover got cached after its metadata was sent (`file://` URL).
    Cover { path: String, url: String },
    Playback(Option<(bool, f64)>),
}

/// OS media controls (MPRIS / SMTC / MediaRemote). The controls live on their own thread
/// because the platform handles are not guaranteed to be `Send`.
pub struct Media(mpsc::Sender<Msg>);

pub fn init(app: &AppHandle) -> Media {
    let (tx, rx) = mpsc::channel();
    #[cfg(windows)]
    let hwnd = app.get_webview_window("main").and_then(|w| w.hwnd().ok()).map(|h| h.0 as usize);
    #[cfg(all(desktop, not(windows)))]
    let hwnd: Option<usize> = None;
    #[cfg(desktop)]
    let handle = app.clone();

    // Android: the UI reports to MediaService (MainActivity) itself
    #[cfg(target_os = "android")]
    {
        let _ = app;
        std::thread::spawn(move || for _ in rx {});
    }
    #[cfg(target_os = "ios")]
    {
        let (key, seek) = (app.clone(), app.clone());
        crate::media_ios::commands(
            move |k| { let _ = key.emit("media-key", k); },
            move |secs| { let _ = seek.emit("media-seek", secs); },
        );
        std::thread::spawn(move || {
            let (mut meta, mut playback) = ((String::new(), String::new(), String::new(), 0.0), None);
            for msg in rx {
                match msg {
                    Msg::Meta { title, artist, album, secs, .. } => {
                        meta = (title, artist, album, secs);
                        playback = playback.map(|(p, _)| (p, 0.0));
                    }
                    Msg::Cover { .. } => continue,
                    Msg::Playback(p) => playback = p,
                }
                crate::media_ios::now_playing(&meta.0, &meta.1, &meta.2, meta.3, playback);
            }
        });
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
        // last metadata, resent when the cover arrives later
        let mut cur: Option<(String, String, String, String, f64, Option<String>)> = None;
        for msg in rx {
            let _ = match msg {
                Msg::Meta { path, title, artist, album, secs, cover } => {
                    cur = Some((path, title, artist, album, secs, cover));
                    set_meta(&mut controls, cur.as_ref().unwrap())
                }
                Msg::Cover { path, url } => match cur.as_mut().filter(|c| c.0 == path) {
                    Some(c) => {
                        c.5 = Some(url);
                        set_meta(&mut controls, c)
                    }
                    None => Ok(()),
                },
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

#[cfg(desktop)]
fn set_meta(controls: &mut MediaControls, (_, title, artist, album, secs, cover): &(String, String, String, String, f64, Option<String>)) -> Result<(), souvlaki::Error> {
    controls.set_metadata(MediaMetadata {
        title: Some(title),
        artist: Some(artist),
        album: Some(album),
        duration: Some(Duration::from_secs_f64(*secs)),
        cover_url: cover.as_deref(),
    })
}

/// `file://` URL of the song's cached full-size cover (written by `get_cover` / `find_cover`), if any.
fn cover_url(app: &AppHandle, path: &str) -> Option<String> {
    let (img, _) = crate::cover_cache(app, path, "")?;
    img.is_file().then(|| tauri::Url::from_file_path(&img).ok()).flatten().map(|u| u.to_string())
}

#[tauri::command]
pub fn media_metadata(app: tauri::AppHandle, m: tauri::State<Media>, path: String, title: String, artist: String, album: String, duration_secs: f64) {
    crate::tray::set_track(&app, &path, &title, &artist);
    let cover = cover_url(&app, &path);
    let _ = m.0.send(Msg::Meta { path, title, artist, album, secs: duration_secs, cover });
}

/// Called once the UI has the song's cover (it is cached on disk by then).
#[tauri::command]
pub fn media_cover(app: tauri::AppHandle, m: tauri::State<Media>, path: String) {
    if let Some(url) = cover_url(&app, &path) {
        let _ = m.0.send(Msg::Cover { path, url });
    }
}

#[tauri::command]
pub fn media_playback(app: tauri::AppHandle, m: tauri::State<Media>, playing: Option<bool>, position: f64) {
    crate::tray::set_playing(&app, playing);
    let _ = m.0.send(Msg::Playback(playing.map(|p| (p, position))));
}
