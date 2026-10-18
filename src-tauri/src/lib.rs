mod db;
mod ext;
mod local;
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

/// Runs blocking work (disk, S3) off the async runtime.
pub(crate) async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T, String> + Send + 'static) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(f).await.map_err(|e| e.to_string())?
}
fn e(err: rusqlite::Error) -> String { err.to_string() }

#[tauri::command]
async fn scan_folder(db: State<'_, Db>, path: String) -> R<Vec<Track>> {
    db.add_folder(&path).map_err(e)?;
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
fn list_folders(db: State<Db>) -> R<Vec<String>> { db.folders().map_err(e) }

#[tauri::command]
fn remove_folder(db: State<Db>, path: String) -> R<Vec<Track>> {
    db.remove_folder(&path).map_err(e)?;
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

/// Covers up to this size are stored as they are; bigger ones are shrunk first.
const SMALL_COVER: usize = 150_000;
/// Uncached covers are read and decoded at most 3 at a time: decoding a big cover takes tens of MB, and an
/// S3 cover needs several range requests, so a fast scroll would otherwise start dozens at once.
static COVER_SLOTS: [std::sync::Mutex<()>; 3] = [const { std::sync::Mutex::new(()) }; 3];
static COVER_NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

fn data_url(mime: &str, data: &[u8]) -> String {
    format!("data:{mime};base64,{}", STANDARD.encode(data))
}

/// Reads the cover from the file's tags: (mime, bytes), shrunk to a JPEG of at most `max` px when large.
/// Thumbnails (`max` <= 256) are always re-encoded: the webview keeps the decoded bitmap of every image in
/// the list, so a 512px cover costs 1 MB there even when it is drawn at 48px.
fn load_cover(path: &str, max: u32) -> Option<(String, Vec<u8>)> {
    let _slot = COVER_SLOTS[COVER_NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed) % 3]
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let (mime, data) = cover_bytes(path)?;
    if max > 256 && data.len() <= SMALL_COVER {
        return Some((mime, data));
    }
    let shrunk = image::load_from_memory(&data).ok().and_then(|img| {
        let img = if img.width().max(img.height()) > max { img.resize(max, max, image::imageops::FilterType::Triangle) } else { img };
        let mut out = Vec::new();
        img.to_rgb8().write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Jpeg).ok()?;
        Some(out)
    });
    Some(match shrunk {
        Some(jpeg) => ("image/jpeg".to_owned(), jpeg),
        None => (mime, data),
    })
}

/// Returns the embedded cover as a data URL. Covers are cached on disk (app cache dir, `covers/`), so a
/// song's file (or S3 object) is only read once; an empty cache file means "no cover".
#[tauri::command]
async fn get_cover(app: tauri::AppHandle, path: String, thumb: Option<bool>) -> Option<String> {
    let (max, tag) = if thumb == Some(true) { (192, "t") } else { (512, "") };
    tauri::async_runtime::spawn_blocking(move || {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        std::hash::Hash::hash(&path, &mut h);
        let name = format!("{:016x}{tag}", std::hash::Hasher::finish(&h));
        let dir = app.path().app_cache_dir().ok().map(|d| d.join("covers"));
        let (img, none) = match &dir {
            Some(d) => (Some(d.join(format!("{name}.img"))), Some(d.join(format!("{name}.none")))),
            None => (None, None),
        };
        if none.as_ref().is_some_and(|f| f.exists()) {
            return None;
        }
        if let Some(bytes) = img.as_ref().and_then(|f| std::fs::read(f).ok()) {
            let mime = if bytes.starts_with(&[0x89, b'P']) { "image/png" } else { "image/jpeg" };
            return Some(data_url(mime, &bytes));
        }
        let cover = load_cover(&path, max);
        if let Some(d) = &dir {
            let _ = std::fs::create_dir_all(d);
            match &cover {
                Some((_, bytes)) => drop(std::fs::write(img.as_ref().unwrap(), bytes)),
                None => drop(std::fs::write(none.as_ref().unwrap(), b"")),
            }
        }
        cover.map(|(m, bytes)| data_url(&m, &bytes))
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
        .setup(|app| {
            migrate_old_dirs(app.handle());
            let dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&dir)?;
            let _ = DATA_DIR.set(dir.clone());
            app.manage(Db::open(&dir.join("library.db"))?);
            app.manage(media::init(app.handle()));
            app.manage(ext::init(app.handle())?);
            tray::init(app.handle())?;
            // WebKitGTK turns trackpad pinch into page zoom; drop pinch events before GTK dispatches them.
            #[cfg(target_os = "linux")]
            gtk::gdk::Event::set_handler(Some(|e: &mut gtk::gdk::Event| {
                if e.event_type() != gtk::gdk::EventType::TouchpadPinch {
                    gtk::main_do_event(e);
                }
            }));
            Ok(())
        })
        .manage(player::AudioState::new().expect("no audio output device"))
        .invoke_handler(tauri::generate_handler![
            scan_folder,
            s3::s3_get_config,
            s3::s3_save_config,
            s3::s3_sync,
            s3::s3_download,
            s3::s3_list,
            list_folders,
            remove_folder,
            local::local_list,
            local::local_mkdir,
            local::local_upload,
            local::local_delete,
            s3::s3_mkdir,
            s3::s3_upload,
            s3::s3_delete,
            load_library,
            list_playlists,
            create_playlist,
            delete_playlist,
            playlist_tracks,
            add_to_playlist,
            remove_from_playlist,
            get_cover,
            lyrics::get_lyrics,
            lyrics::search_lyrics,
            lyrics::save_lyrics,
            ext::ext_list,
            ext::ext_install_file,
            ext::ext_install_url,
            ext::ext_repo,
            ext::ext_remove,
            ext::ext_set_enabled,
            ext::ext_save_settings,
            ext::ext_call,
            ext::ext_download,
            ext::session::ext_session_verify,
            ext::session::ext_session_poll,
            player::play,
            player::set_paused,
            player::queue_next,
            player::seek,
            player::set_volume,
            player::status,
            media::media_metadata,
            media::media_playback,
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
            _ => {}
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
