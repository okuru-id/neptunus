mod db;
mod lyrics;
mod media;
mod player;
mod s3;
mod tray;
use db::{Db, Playlist, Track};
use tauri::{Manager, State};
use base64::{Engine, engine::general_purpose::STANDARD};
use lofty::{file::{AudioFile, TaggedFileExt}, probe::Probe, tag::Accessor};
use walkdir::WalkDir;

/// App data dir, for code that has no `AppHandle` (the audio thread reading S3 objects).
pub(crate) static DATA_DIR: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();

pub(crate) const EXTS: &[&str] = &["mp3", "flac", "m4a", "aac", "ogg", "opus", "wav"];

/// Opens the tags of a local file or an `s3://bucket/key` object (read with range requests).
pub(crate) fn probe(path: &str) -> Option<lofty::file::TaggedFile> {
    match s3::reader(path) {
        Some(r) => Probe::new(r.ok()?).guess_file_type().ok()?.read().ok(),
        None => Probe::open(path).ok()?.read().ok(),
    }
}

pub(crate) fn read_track(path: &str) -> Option<Track> {
    let file = probe(path)?;
    let tag = file.primary_tag().or_else(|| file.first_tag());
    let stem = std::path::Path::new(path).file_stem()?.to_string_lossy().into_owned();
    Some(Track {
        path: path.to_owned(),
        title: tag.and_then(|t| t.title().map(|s| s.into_owned())).unwrap_or(stem),
        artist: tag.and_then(|t| t.artist().map(|s| s.into_owned())).unwrap_or_default(),
        album: tag.and_then(|t| t.album().map(|s| s.into_owned())).unwrap_or_default(),
        duration_secs: file.properties().duration().as_secs(),
    })
}

type R<T> = Result<T, String>;
fn e(err: rusqlite::Error) -> String { err.to_string() }

#[tauri::command]
async fn scan_folder(db: State<'_, Db>, path: String) -> R<Vec<Track>> {
    let found: Vec<Track> = WalkDir::new(path)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| {
            e.path().extension().and_then(|x| x.to_str())
                .is_some_and(|x| EXTS.contains(&x.to_lowercase().as_str()))
        })
        .filter_map(|e| read_track(&e.path().to_string_lossy()))
        .collect();
    db.upsert(&found).map_err(e)?;
    db.all().map_err(e)
}

#[tauri::command]
fn load_library(db: State<Db>) -> R<Vec<Track>> { db.all().map_err(e) }

#[tauri::command]
fn list_playlists(db: State<Db>) -> R<Vec<Playlist>> { db.playlists().map_err(e) }

#[tauri::command]
fn create_playlist(db: State<Db>, name: String) -> R<i64> { db.create_playlist(&name).map_err(e) }

#[tauri::command]
fn delete_playlist(db: State<Db>, id: i64) -> R<()> { db.delete_playlist(id).map_err(e) }

#[tauri::command]
fn playlist_tracks(db: State<Db>, id: i64) -> R<Vec<Track>> { db.playlist_tracks(id).map_err(e) }

#[tauri::command]
fn add_to_playlist(db: State<Db>, id: i64, path: String) -> R<()> { db.add_to_playlist(id, &path).map_err(e) }

#[tauri::command]
fn remove_from_playlist(db: State<Db>, id: i64, path: String) -> R<()> { db.remove_from_playlist(id, &path).map_err(e) }

/// Returns the embedded cover as a data URL, if any.
#[tauri::command]
async fn get_cover(path: String) -> Option<String> {
    tauri::async_runtime::spawn_blocking(move || {
        let (mime, data) = cover_bytes(&path)?;
        Some(format!("data:{mime};base64,{}", STANDARD.encode(data)))
    })
    .await
    .ok()
    .flatten()
}

/// Embedded cover art as (mime type, raw image bytes).
pub(crate) fn cover_bytes(path: &str) -> Option<(String, Vec<u8>)> {
    let file = probe(path)?;
    let tag = file.primary_tag().or_else(|| file.first_tag())?;
    let pic = tag.pictures().first()?;
    let mime = pic.mime_type().map(|m| m.as_str()).unwrap_or("image/jpeg");
    Some((mime.to_owned(), pic.data().to_vec()))
}

/// The app was called PixelPlay (identifier `com.pixelplay.desktop`); move its library, settings and
/// caches over under the new identifier. Per entry, because the webview has already created the new
/// data dir by the time setup runs; what is left of the old dir is webview cache and is dropped.
fn migrate_old_dirs(app: &tauri::AppHandle) {
    let p = app.path();
    for dir in [p.app_data_dir(), p.app_cache_dir()].into_iter().flatten() {
        let Some(old) = dir.parent().map(|d| d.join("com.pixelplay.desktop")) else { continue };
        let Ok(entries) = std::fs::read_dir(&old) else { continue };
        let _ = std::fs::create_dir_all(&dir);
        for e in entries.flatten() {
            let to = dir.join(e.file_name());
            if !to.exists() {
                let _ = std::fs::rename(e.path(), to);
            }
        }
        let _ = std::fs::remove_dir_all(&old);
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_positioner::init())
        .setup(|app| {
            migrate_old_dirs(app.handle());
            let dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&dir)?;
            let _ = DATA_DIR.set(dir.clone());
            app.manage(Db::open(&dir.join("library.db"))?);
            app.manage(media::init(app.handle()));
            tray::init(app.handle())?;
            Ok(())
        })
        .manage(player::AudioState::new().expect("no audio output device"))
        .invoke_handler(tauri::generate_handler![
            scan_folder,
            s3::s3_get_config,
            s3::s3_save_config,
            s3::s3_sync,
            load_library,
            list_playlists,
            create_playlist,
            delete_playlist,
            playlist_tracks,
            add_to_playlist,
            remove_from_playlist,
            get_cover,
            lyrics::get_lyrics,
            player::play,
            player::set_paused,
            player::queue_next,
            player::seek,
            player::set_volume,
            player::status,
            media::media_metadata,
            media::media_playback,
            tray::open_main
        ])
        .on_window_event(|window, event| match (window.label(), event) {
            // While music plays, closing the main window keeps it running in the tray (quit from the
            // tray menu). Otherwise close quits, so the app can't get stuck hidden on desktops
            // without a tray (e.g. GNOME without the AppIndicator extension).
            ("main", tauri::WindowEvent::CloseRequested { api, .. }) => {
                if window.state::<player::AudioState>().is_playing() {
                    api.prevent_close();
                    let _ = window.hide();
                } else {
                    window.app_handle().exit(0);
                }
            }
            ("mini", tauri::WindowEvent::Focused(false)) => tray::hide_mini_on_blur(window),
            _ => {}
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
