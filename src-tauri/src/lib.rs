mod db;
mod ext;
mod local;
mod lyrics;
mod media;
#[cfg(target_os = "ios")]
mod media_ios;
mod player;
mod s3;
mod spotify;
mod tray;
use db::{Db, Playlist, Track};
use tauri::{Manager, State};
use base64::{Engine, engine::general_purpose::STANDARD};
use lofty::{file::{AudioFile, TaggedFileExt}, probe::Probe, tag::Accessor};
use walkdir::WalkDir;

/// App data dir, for code that has no `AppHandle` (the audio thread reading S3 objects).
pub(crate) static DATA_DIR: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();

/// Where downloads go when no library folder is set: `Music/Neptunus`, or on iOS `Documents/Neptunus`,
/// the folder the Files app shows (Info.ios.plist) and the one "Tambah folder" adds there.
pub(crate) fn download_dir(app: &tauri::AppHandle) -> Result<std::path::PathBuf, String> {
    #[cfg(target_os = "ios")]
    let base = app.path().document_dir();
    #[cfg(not(target_os = "ios"))]
    let base = app.path().audio_dir();
    Ok(base.map_err(|e| e.to_string())?.join("Neptunus"))
}

/// Moves a path to the system trash (permanent delete on mobile, which has none).
pub(crate) fn trash(path: impl AsRef<std::path::Path>) -> Result<(), String> {
    #[cfg(desktop)]
    return trash::delete(path).map_err(|e| e.to_string());
    #[cfg(mobile)]
    {
        let p = path.as_ref();
        return if p.is_dir() { std::fs::remove_dir_all(p) } else { std::fs::remove_file(p) }.map_err(|e| e.to_string());
    }
}

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

/// The newest GitHub release when it is newer than this build: (version, release page URL).
#[tauri::command]
async fn check_update() -> R<Option<(String, String)>> {
    blocking(|| {
        let r: serde_json::Value = attohttpc::get("https://api.github.com/repos/okuru-id/neptunus/releases/latest")
            .header("User-Agent", "neptunus")
            .header("Accept", "application/vnd.github+json")
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .and_then(|r| r.error_for_status())
            .and_then(|r| r.json())
            .map_err(|e| e.to_string())?;
        let tag = r["tag_name"].as_str().unwrap_or_default();
        let ver = |s: &str| -> Vec<u64> { s.trim_start_matches('v').split('.').map(|p| p.parse().unwrap_or(0)).collect() };
        Ok((ver(tag) > ver(env!("CARGO_PKG_VERSION")))
            .then(|| (tag.trim_start_matches('v').to_owned(), r["html_url"].as_str().unwrap_or_default().to_owned())))
    })
    .await
}

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

/// Cache files of a song's cover: (`<hash><tag>.img`, `<hash><tag>.none`); `tag` is "t" for thumbnails.
fn cover_cache(app: &tauri::AppHandle, path: &str, tag: &str) -> Option<(std::path::PathBuf, std::path::PathBuf)> {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    std::hash::Hash::hash(path, &mut h);
    let name = format!("{:016x}{tag}", std::hash::Hasher::finish(&h));
    let dir = app.path().app_cache_dir().ok()?.join("covers");
    Some((dir.join(format!("{name}.img")), dir.join(format!("{name}.none"))))
}

/// A playing song without cover art: looks it up on the iTunes Search API by title and artist and
/// caches it as the song's cover (the file itself is left alone). None when nothing matches well.
#[tauri::command]
async fn find_cover(app: tauri::AppHandle, db: State<'_, Db>, path: String) -> R<Option<String>> {
    let Some(t) = db.track(&path).map_err(e)? else { return Ok(None) };
    let (title, artist) = (spotify::norm(&t.title), spotify::norm(t.artist.split([',', '&', ';']).next().unwrap_or("")));
    if title.is_empty() {
        return Ok(None);
    }
    blocking(move || {
        let hit = itunes(&format!("{} {}", t.artist, t.title), |name, a| name == title && (artist.is_empty() || a.contains(&artist)))?;
        let Some(url) = hit.as_ref().and_then(|h| h["artworkUrl100"].as_str()) else { return Ok(None) };
        let get = |url: &str| attohttpc::get(url).header("User-Agent", "Mozilla/5.0").timeout(std::time::Duration::from_secs(10));
        let bytes = get(&url.replace("100x100bb", "600x600bb")).send().and_then(|r| r.bytes()).map_err(|e| e.to_string())?;
        let img = image::load_from_memory(&bytes).map_err(|e| e.to_string())?;
        let jpeg = |max: u32| {
            let mut out = Vec::new();
            let _ = img.resize(max, max, image::imageops::FilterType::Triangle).to_rgb8()
                .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Jpeg);
            out
        };
        let (full, thumb) = (jpeg(512), jpeg(192));
        for (tag, data) in [("", &full), ("t", &thumb)] {
            if let Some((img, none)) = cover_cache(&app, &path, tag) {
                let _ = std::fs::create_dir_all(img.parent().unwrap());
                let _ = std::fs::remove_file(none);
                let _ = std::fs::write(img, data);
            }
        }
        Ok(Some(data_url("image/jpeg", &full)))
    })
    .await
}

/// First iTunes Search song whose normalized (title, artist) passes `ok`.
fn itunes(term: &str, ok: impl Fn(&str, &str) -> bool) -> R<Option<serde_json::Value>> {
    let res: serde_json::Value = attohttpc::get("https://itunes.apple.com/search")
        .header("User-Agent", "Mozilla/5.0").timeout(std::time::Duration::from_secs(10))
        .params([("term", term), ("entity", "song"), ("limit", "10")])
        .send().and_then(|r| r.json()).map_err(|e| e.to_string())?;
    Ok(res["results"].as_array().into_iter().flatten()
        .find(|r| ok(&spotify::norm(r["trackName"].as_str().unwrap_or("")), &spotify::norm(r["artistName"].as_str().unwrap_or(""))))
        .cloned())
}

/// A playing song missing its artist or album: looks it up on iTunes (titles like "Song - Artist"
/// count too; a bare title with no artist is too ambiguous), saves what was missing to the library and, for local files, the file's tags.
#[tauri::command]
async fn find_meta(db: State<'_, Db>, path: String) -> R<Option<db::Track>> {
    let Some(mut t) = db.track(&path).map_err(e)? else { return Ok(None) };
    if !t.artist.is_empty() && !t.album.is_empty() {
        return Ok(None);
    }
    let found = blocking({
        let t = t.clone();
        move || {
            let artist = spotify::norm(t.artist.split([',', '&', ';']).next().unwrap_or(""));
            let (x, y) = t.title.split_once(" - ").map(|(x, y)| (spotify::norm(x), spotify::norm(y))).unwrap_or_default();
            itunes(&format!("{} {}", t.artist, t.title), |name, a| {
                (name == spotify::norm(&t.title) && !artist.is_empty() && a.contains(&artist))
                    || (t.artist.is_empty() && !x.is_empty() && ((name == x && a.contains(&y)) || (name == y && a.contains(&x))))
            })
        }
    })
    .await?;
    let Some(hit) = found else { return Ok(None) };
    let text = |k: &str| hit[k].as_str().unwrap_or("").to_owned();
    if t.artist.is_empty() {
        (t.title, t.artist) = (text("trackName"), text("artistName"));
    }
    if t.album.is_empty() {
        t.album = text("collectionName");
    }
    db.upsert(std::slice::from_ref(&t)).map_err(e)?;
    if !path.starts_with("s3://") {
        let tags = serde_json::json!({ "name": t.title, "artists": t.artist, "album_name": t.album });
        let p = std::path::PathBuf::from(&path);
        let _ = tauri::async_runtime::spawn_blocking(move || ext::tag_file(&p, &tags)).await;
    }
    Ok(Some(t))
}

/// Returns the embedded cover as a data URL. Covers are cached on disk (app cache dir, `covers/`), so a
/// song's file (or S3 object) is only read once; an empty cache file means "no cover".
#[tauri::command]
async fn get_cover(app: tauri::AppHandle, path: String, thumb: Option<bool>) -> Option<String> {
    let (max, tag) = if thumb == Some(true) { (192, "t") } else { (512, "") };
    tauri::async_runtime::spawn_blocking(move || {
        let dir = app.path().app_cache_dir().ok().map(|d| d.join("covers"));
        let (img, none) = match cover_cache(&app, &path, tag) {
            Some((i, n)) => (Some(i), Some(n)),
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

/// What a song file is encoded as, read from its header (tags and cover art are skipped).
#[derive(serde::Serialize)]
struct Quality {
    /// FLAC, ALAC, AAC, MP3, Opus, Vorbis, WAV, ...
    format: &'static str,
    bits: Option<u8>,
    /// Hz
    rate: Option<u32>,
    /// kbps
    kbps: Option<u32>,
    lossless: bool,
}

/// Audio quality of a local song file; `None` for cloud (S3) songs and unreadable files.
#[tauri::command]
async fn audio_quality(path: String) -> Option<Quality> {
    if path.starts_with("s3://") {
        return None;
    }
    tauri::async_runtime::spawn_blocking(move || quality_of(&path)).await.ok().flatten()
}

fn quality_of(path: &str) -> Option<Quality> {
    use lofty::{config::ParseOptions, file::FileType};
    {
        // by content, not extension: a download may be named .flac while holding an MP4 stream
        let file = Probe::open(path).ok()?.guess_file_type().ok()?.options(ParseOptions::new().read_tags(false).read_cover_art(false)).read().ok()?;
        let p = file.properties();
        let bits = p.bit_depth();
        let (format, lossless) = match file.file_type() {
            FileType::Flac => ("FLAC", true),
            FileType::Mp4 => {
                use lofty::{file::AudioFile, mp4::{Mp4Codec, Mp4File}};
                let mut f = std::fs::File::open(path).ok()?;
                let mp4 = Mp4File::read_from(&mut f, ParseOptions::new().read_tags(false).read_cover_art(false)).ok()?;
                match mp4.properties().codec() {
                    Some(Mp4Codec::ALAC) => ("ALAC", true),
                    Some(Mp4Codec::FLAC) => ("FLAC", true),
                    Some(Mp4Codec::MP3) => ("MP3", false),
                    Some(Mp4Codec::AAC) => ("AAC", false),
                    // e.g. an encrypted stream that was never decrypted
                    _ => ("MP4", false),
                }
            }
            FileType::Aac => ("AAC", false),
            FileType::Mpeg => ("MP3", false),
            FileType::Opus => ("Opus", false),
            FileType::Vorbis => ("Vorbis", false),
            FileType::Wav => ("WAV", true),
            FileType::Aiff => ("AIFF", true),
            FileType::WavPack => ("WavPack", true),
            FileType::Ape => ("APE", true),
            _ => ("Audio", false),
        };
        Some(Quality { format, bits: bits.filter(|_| lossless), rate: p.sample_rate(), kbps: p.audio_bitrate(), lossless })
    }
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
    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init());
    #[cfg(mobile)]
    let builder = builder.plugin(ext::session::grant_catcher());
    builder
        .setup(|app| {
            migrate_old_dirs(app.handle());
            let dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&dir)?;
            let _ = DATA_DIR.set(dir.clone());
            app.manage(Db::open(&dir.join("library.db"))?);
            // iOS mutes the default (solo ambient) session when the app leaves the foreground.
            #[cfg(target_os = "ios")]
            unsafe {
                use objc2_avf_audio::{AVAudioSession, AVAudioSessionCategoryPlayback};
                let session = AVAudioSession::sharedInstance();
                if let Some(playback) = AVAudioSessionCategoryPlayback {
                    let _ = session.setCategory_error(playback);
                }
                let _ = session.setActive_error(true);
            }
            app.manage(media::init(app.handle()));
            // Drives the player poll in the UI: webview timers get throttled while the window is
            // hidden (Android in the background, tray), which would stall the move to the next song.
            let ticker = app.handle().clone();
            std::thread::spawn(move || loop {
                std::thread::sleep(std::time::Duration::from_millis(500));
                let _ = tauri::Emitter::emit(&ticker, "player-tick", ());
            });
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
            spotify::spotify_import,
            spotify::playlist_pending,
            spotify::pending_remove,
            spotify::pending_download,
            s3::s3_get_config,
            s3::s3_save_config,
            s3::s3_disconnect,
            s3::s3_buffered,
            ext::ext_cancel,
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
            find_cover,
            find_meta,
            audio_quality,
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
            ext::ext_redownload,
            ext::session::ext_session_verify,
            ext::session::ext_session_poll,
            check_update,
            player::play,
            player::set_paused,
            player::queue_next,
            player::seek,
            player::set_volume,
            player::set_eq,
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
