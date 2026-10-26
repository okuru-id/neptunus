//! Video library: files under the music folders and on S3. Nothing is indexed; the list is read
//! fresh each time (a directory walk and one bucket listing), and the webview plays the files itself.

use crate::Db;
use serde::Serialize;
use std::path::Path;
use tauri::{AppHandle, Manager, State};
use walkdir::WalkDir;

const EXTS: &[&str] = &["mp4", "m4v", "mkv", "webm", "mov"];

#[derive(Serialize)]
pub struct Video {
    path: String,
    title: String,
    size: u64,
}

fn title(path: &str) -> String {
    Path::new(path).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
}

#[tauri::command]
pub async fn video_list(app: AppHandle, db: State<'_, Db>) -> Result<Vec<Video>, String> {
    let folders = db.folders().map_err(|e| e.to_string())?;
    crate::blocking(move || {
        let scope = app.asset_protocol_scope();
        let mut out = Vec::new();
        for dir in folders {
            // the webview loads local videos through the asset protocol, limited to these folders
            let _ = scope.allow_directory(&dir, true);
            for e in WalkDir::new(&dir).into_iter().filter_map(Result::ok).filter(|e| e.file_type().is_file()) {
                let ext = e.path().extension().and_then(|x| x.to_str()).map(str::to_lowercase);
                if ext.is_some_and(|x| EXTS.contains(&x.as_str())) {
                    let path = e.path().to_string_lossy().into_owned();
                    out.push(Video { title: title(&path), size: e.metadata().map(|m| m.len()).unwrap_or(0), path });
                }
            }
        }
        // a broken S3 connection shouldn't hide the local videos
        for (path, size) in crate::s3::videos(&app, EXTS).unwrap_or_default() {
            out.push(Video { title: title(&path), size, path });
        }
        out.sort_by_key(|v| v.title.to_lowercase());
        Ok(out)
    })
    .await
}

/// Dev builds: what happens to the video playing, on the terminal (`[video] …`). URLs without their
/// query, headers by name only: both may carry a token.
macro_rules! vlog {
    ($($t:tt)*) => { if cfg!(debug_assertions) { println!("[video] {}", format_args!($($t)*)); } };
}
/// A URL for the log: no query.
fn short(url: &str) -> &str {
    url.split(['?', '#']).next().unwrap_or(url)
}

/// A subtitle for an outside player's list (mpv, iOS's VLCKit screen): its file, and the name and
/// language it shows under there (the built-in player's menu label).
#[derive(serde::Deserialize)]
#[cfg_attr(not(any(target_os = "linux", target_os = "ios")), allow(dead_code))]
pub struct Sub {
    url: String,
    title: String,
    lang: String,
}

/// `subs` as WebVTT files in `dir`, named by their titles (players without a title of their own show the
/// file's name), fetched side by side (`video_subtitle`: cached). `got` gets each one in their order; the
/// shown one (`first_now`: the first) as soon as it is there, before the rest.
#[cfg(any(target_os = "linux", target_os = "ios"))]
fn fetch_subs(app: &AppHandle, subs: &[Sub], first_now: bool, dir: &Path, mut got: impl FnMut(usize, &Path)) {
    let _ = std::fs::remove_dir_all(dir);
    if subs.is_empty() || std::fs::create_dir_all(dir).is_err() {
        return;
    }
    let fetch = |i: usize| -> Option<std::path::PathBuf> {
        let vtt = video_subtitle(app.state(), subs[i].url.clone()).ok()?;
        let name: String = subs[i].title.chars().map(|c| if matches!(c, '/' | '\\' | ':') { '-' } else { c }).collect();
        let f = dir.join(format!("{name}.vtt"));
        std::fs::write(&f, vtt).ok().map(|_| f)
    };
    let first = usize::from(first_now);
    if first_now {
        if let Some(f) = fetch(0) {
            got(0, &f);
        }
    }
    let idx: Vec<usize> = (first..subs.len()).collect();
    for chunk in idx.chunks(8) {
        let done: Vec<_> = std::thread::scope(|sc| {
            let jobs: Vec<_> = chunk.iter().map(|&i| sc.spawn(move || (i, fetch(i)))).collect();
            jobs.into_iter().filter_map(|j| j.join().ok()).collect()
        });
        for (i, f) in done {
            if let Some(f) = f {
                got(i, &f);
            }
        }
    }
}

/// Dev builds: the player's own events (VideoPlayer.vue) next to the rest.
#[tauri::command]
pub fn video_log(msg: String) {
    vlog!("player: {msg}");
}

mod fill;
#[cfg(target_os = "linux")]
pub mod mpv;
#[cfg(desktop)]
pub mod remux;
mod posters;

/// The file name a remote video is kept under in the cache folder (without `.part`).
fn cache_key(name: &str) -> String {
    use sha2::Digest;
    sha2::Sha256::digest(name.as_bytes()).iter().take(16).map(|b| format!("{b:02x}")).collect()
}

fn cache_dir(app: &AppHandle) -> Result<std::path::PathBuf, String> {
    Ok(app.path().app_cache_dir().map_err(|e| e.to_string())?.join("video"))
}

/// Bytes the video disk cache takes.
#[tauri::command]
pub fn video_cache_size(app: AppHandle) -> Result<u64, String> {
    // a film downloaded in part counts what came down, not its whole length (`fill::stored`)
    let films: u64 = std::fs::read_dir(cache_dir(&app)?).into_iter().flatten().flatten().map(|e| e.path())
        .map(|p| if p.extension().is_some_and(|x| x == "part") { fill::stored(&p) } else { p.metadata().map_or(0, |m| m.len()) })
        .sum();
    Ok(films + posters::size())
}

/// Empties the video disk cache (Settings › Lokal), stopping the last video's download first.
#[tauri::command]
pub fn video_cache_clear(app: AppHandle) -> Result<(), String> {
    serve::stop();
    for e in std::fs::read_dir(cache_dir(&app)?).into_iter().flatten().flatten() {
        let _ = std::fs::remove_file(e.path());
    }
    posters::clear();
    Ok(())
}

#[derive(Serialize)]
pub struct CachedFilm {
    /// the cache files' name (a hash of the video's URL or S3 path)
    key: String,
    /// the player's title for it; none for videos cached before titles were kept
    title: Option<String>,
    /// bytes it takes (`fill::stored`)
    size: u64,
    /// how much of it came down, 0 to 1
    done: f64,
    /// when it was last played, unix seconds
    used: u64,
}

/// The films in the disk cache, last played first (Settings › Lokal, to delete some by hand).
#[tauri::command]
pub fn video_cache_list(app: AppHandle) -> Result<Vec<CachedFilm>, String> {
    let dir = cache_dir(&app)?;
    let mut out: Vec<CachedFilm> = std::fs::read_dir(&dir).into_iter().flatten().flatten().map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "part"))
        .filter_map(|p| {
            let key = p.file_stem()?.to_str()?.to_owned();
            let title = std::fs::read_to_string(p.with_extension("json")).ok()
                .and_then(|j| serde_json::from_str::<serde_json::Value>(&j).ok())
                .and_then(|j| j["title"].as_str().map(str::to_owned));
            let used = fill::used(&p).and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs());
            Some(CachedFilm { key, title, size: fill::stored(&p), done: fill::progress(&p), used })
        })
        .collect();
    out.sort_by(|a, b| b.used.cmp(&a.used));
    Ok(out)
}

/// Deletes the cached films `keys` (from `video_cache_list`), stopping the download of one still playing.
#[tauri::command]
pub fn video_cache_remove(app: AppHandle, keys: Vec<String>) -> Result<(), String> {
    let dir = cache_dir(&app)?;
    for key in keys {
        // a hash, so nothing outside the cache folder can be named
        if key.is_empty() || !key.bytes().all(|b| b.is_ascii_hexdigit()) {
            continue;
        }
        let part = dir.join(format!("{key}.part"));
        if serve::FILL.lock().unwrap().as_ref().is_some_and(|(_, f)| f.path() == part) {
            serve::stop();
        }
        for ext in ["part", "meta", "json"] {
            let _ = std::fs::remove_file(part.with_extension(ext));
        }
    }
    Ok(())
}

/// Which of the remote video `urls` have something downloaded in the disk cache.
#[tauri::command]
pub fn video_cached(app: AppHandle, urls: Vec<String>) -> Result<Vec<String>, String> {
    let dir = cache_dir(&app)?;
    Ok(urls.into_iter().filter(|u| fill::has_data(&dir.join(format!("{}.part", cache_key(u))))).collect())
}

/// Where the catalogs load posters from: the loopback server, which keeps them on disk (`posters`);
/// what the picture is of and the image URL go after it, each percent-encoded and split by a slash. `None` when the server can't start: posters then
/// come straight from their hosts.
#[tauri::command]
pub fn poster_base(app: AppHandle) -> Option<String> {
    posters::set_dir(app.path().app_cache_dir().ok()?.join("posters"));
    serve::poster_base().ok()
}

/// A URL the webview can stream: presigned for S3, the loopback server for local files (desktop and
/// iOS). `None` for local files on Android: the UI uses `convertFileSrc` (the asset protocol) there.
/// Remote videos go through the loopback server too, which downloads them in full and keeps them
/// on disk for the days set in Settings › Lokal, a week by default (`fill`).
#[tauri::command]
pub async fn video_url(app: AppHandle, path: String, headers: Option<std::collections::HashMap<String, String>>, title: Option<String>) -> Result<Option<String>, String> {
    let remote = path.starts_with("http://") || path.starts_with("https://");
    vlog!("open {} (headers: {:?})", short(&path), headers.as_ref().map(|h| h.keys().collect::<Vec<_>>()).unwrap_or_default());
    let url = if path.starts_with("s3://") {
        let (app, path) = (app.clone(), path.clone());
        crate::blocking(move || crate::s3::presigned(&app, &path)).await?
    } else if remote {
        path.clone()
    } else {
        // iOS too: WKWebView's player can't read video through the asset protocol's custom scheme
        #[cfg(not(target_os = "android"))]
        return serve::url(&path).map(Some);
        #[cfg(target_os = "android")]
        return Ok(None);
    };
    if !url.split(['?', '#']).next().unwrap_or("").to_lowercase().ends_with(".m3u8") && !url.contains(char::is_whitespace) {
        let dir = cache_dir(&app)?;
        // kept under the S3 path, not the presigned URL, which changes every time
        // Settings › Lokal: days a video stays in the cache after it was last played
        let days = app.state::<Db>().setting("video_cache_days").ok().flatten().and_then(|d| d.parse().ok()).unwrap_or(7);
        return serve::fill(url, headers.unwrap_or_default().into_iter().collect(), &path, title.as_deref(), &dir, days).map(Some);
    }
    Ok(Some(url))
}

/// The content type the loopback server sends for a video, by its extension.
fn mime(path: &str) -> &'static str {
    match path.split(['?', '#']).next().unwrap_or("").rsplit('.').next().map(str::to_lowercase).as_deref() {
        Some("webm") => "video/webm",
        Some("mkv") => "video/x-matroska",
        Some("mov") => "video/quicktime",
        _ => "video/mp4",
    }
}

/// iOS: the player apps a film can go to that are installed, of VLC and Infuse. Their URL schemes are
/// in `LSApplicationQueriesSchemes` (Info.ios.plist), or iOS says no for both.
#[cfg(target_os = "ios")]
#[tauri::command]
pub async fn ios_players(app: AppHandle) -> Vec<&'static str> {
    let (tx, rx) = std::sync::mpsc::channel();
    let _ = app.run_on_main_thread(move || {
        use objc2::runtime::AnyObject;
        use objc2::{class, msg_send};
        let ui: *mut AnyObject = unsafe { msg_send![class!(UIApplication), sharedApplication] };
        let found = [("vlc", "vlc-x-callback://"), ("infuse", "infuse://")].into_iter().filter(|(_, scheme)| {
            let s = objc2_foundation::NSString::from_str(scheme);
            let url: *mut AnyObject = unsafe { msg_send![class!(NSURL), URLWithString: &*s] };
            !url.is_null() && unsafe { msg_send![ui, canOpenURL: url] }
        });
        let _ = tx.send(found.map(|(name, _)| name).collect::<Vec<_>>());
    });
    crate::blocking(move || Ok::<_, String>(rx.recv().unwrap_or_default())).await.unwrap_or_default()
}

/// iOS: the film screen built on VLCKit (`src-tauri/ios/NeptunusPlayer.swift`), there when the build
/// linked it in (scripts/ios-vlckit.sh).
#[cfg(target_os = "ios")]
fn ios_player() -> Option<&'static objc2::runtime::AnyClass> {
    objc2::runtime::AnyClass::get(c"NeptunusPlayer")
}

#[cfg(target_os = "ios")]
static IOS_APP: std::sync::OnceLock<AppHandle> = std::sync::OnceLock::new();

/// iOS: whether films can play in the VLCKit screen rather than the WebView's player.
#[cfg(target_os = "ios")]
#[tauri::command]
pub fn ios_native_available() -> bool {
    ios_player().is_some()
}

/// iOS: plays `film` (`{url, title, headers, position}`, ms) in the VLCKit screen. The subtitles follow
/// as files once fetched, named for the menu (VLCKit shows a file's name); the first shown when
/// `sub_on`. Where it got to comes back as the `native-video-closed` event.
#[cfg(target_os = "ios")]
#[tauri::command]
pub fn ios_play_native(app: AppHandle, film: String, subs: Vec<Sub>, sub_on: bool) -> Result<(), String> {
    use objc2::msg_send;
    use objc2_foundation::NSString;
    let _ = IOS_APP.set(app.clone());
    let cls = ios_player().ok_or("Pemutar VLC tidak ada di build ini.")?;
    let dir = app.path().app_cache_dir().map_err(|e| e.to_string())?.join("vlc-subs");
    app.run_on_main_thread(move || {
        let s = NSString::from_str(&film);
        let _: () = unsafe { msg_send![cls, play: &*s] };
    })
    .map_err(|e| e.to_string())?;
    std::thread::spawn(move || {
        fetch_subs(&app, &subs, sub_on, &dir, |i, f| {
            let (path, select) = (f.to_string_lossy().into_owned(), i == 0 && sub_on);
            let _ = app.run_on_main_thread(move || {
                let s = NSString::from_str(&path);
                let _: () = unsafe { msg_send![cls, addSubtitle: &*s, select: select] };
            });
        });
    });
    Ok(())
}

/// Called by NeptunusPlayer.swift as its screen closes, with `{pos, duration, eof, error}`.
#[cfg(target_os = "ios")]
#[unsafe(no_mangle)]
pub extern "C" fn neptunus_video_closed(json: *const std::ffi::c_char) {
    use tauri::Emitter;
    let json = unsafe { std::ffi::CStr::from_ptr(json) }.to_string_lossy();
    let v: serde_json::Value = serde_json::from_str(&json).unwrap_or_default();
    if let Some(app) = IOS_APP.get() {
        let _ = app.emit("native-video-closed", v);
    }
}

/// iOS: a film handed to VLC or Infuse (`ios_players`), by their x-callback URL.
#[cfg(target_os = "ios")]
#[tauri::command]
pub fn ios_play_in(app: AppHandle, url: String) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    if !url.starts_with("vlc-x-callback://x-callback-url/") && !url.starts_with("infuse://x-callback-url/") {
        return Err("Pemutar tidak dikenal.".into());
    }
    app.opener().open_url(url, None::<&str>).map_err(|e| e.to_string())
}

/// Why the source wouldn't give the video being downloaded (`fill`), if it wouldn't.
#[tauri::command]
pub fn video_source_error() -> Option<String> {
    serve::FILL.lock().unwrap().as_ref()?.1.error()
}

/// How far the playing video is downloaded ahead of `at` (a fraction of it), as a fraction of it.
/// `None` when it isn't being downloaded here.
#[tauri::command]
pub fn video_buffered(at: f64) -> Option<f64> {
    serve::FILL.lock().unwrap().as_ref()?.1.buffered(at)
}

/// The player of `url` closed: stops its download (what came down is kept). Another video may have
/// started since, so only that one's.
#[tauri::command]
pub fn video_stop(url: String) {
    let mut cur = serve::FILL.lock().unwrap();
    if cur.as_ref().is_some_and(|(id, _)| url.ends_with(&format!("/~fill/{id}"))) {
        cur.take().unwrap().1.close();
    }
}

/// WebKitGTK and iOS WKWebView can't play media from a custom scheme (`asset://`), so local videos go over
/// `http://127.0.0.1` with Range support. Only files handed out by `url` are served, under a random
/// per-run token. It also serves the remote video being downloaded (`fill`).
#[cfg_attr(target_os = "android", allow(dead_code))]
mod serve {
    use std::collections::HashSet;
    use std::fs::File;
    use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::{Mutex, OnceLock};

    static SERVER: OnceLock<(u16, String)> = OnceLock::new();
    static ALLOWED: Mutex<Option<HashSet<String>>> = Mutex::new(None);
    /// The remote video being downloaded, by a number that changes with each one.
    pub static FILL: Mutex<Option<(u64, std::sync::Arc<super::fill::Fill>)>> = Mutex::new(None);

    /// Stops downloading the last video, so its files can go.
    pub fn stop() {
        if let Some((_, f)) = FILL.lock().unwrap().take() {
            f.close();
        }
    }

    pub fn url(path: &str) -> Result<String, String> {
        let (port, token) = server()?;
        ALLOWED.lock().unwrap().get_or_insert_with(HashSet::new).insert(path.to_owned());
        Ok(format!("http://127.0.0.1:{port}/{token}/{}", encode(path)))
    }

    /// Starts (or resumes) downloading `url` into `dir`, stopping the previous video's download.
    pub fn fill(url: String, headers: Vec<(String, String)>, name: &str, title: Option<&str>, dir: &std::path::Path, days: u64) -> Result<String, String> {
        let (port, token) = server()?;
        let mut cur = FILL.lock().unwrap();
        let id = cur.take().map_or(0, |(id, f)| { f.close(); id + 1 });
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        // one file per video, so playing it again finds what came down before
        let key = super::cache_key(name);
        let path = dir.join(format!("{key}.part"));
        let _ = std::fs::File::options().append(true).open(path.with_extension("meta")).and_then(|f| f.set_modified(std::time::SystemTime::now()));
        // what it is, for the list in Settings › Lokal (the files are named by a hash)
        if let Some(title) = title.filter(|t| !t.is_empty()) {
            let _ = std::fs::write(path.with_extension("json"), serde_json::json!({ "title": title }).to_string());
        }
        let (d, keep) = (dir.to_owned(), path.clone());
        std::thread::spawn(move || prune(&d, &keep, days));
        vlog!("fill #{id}: {} -> {}", super::short(&url), path.display());
        *cur = Some((id, super::fill::Fill::start(url, headers, path)));
        Ok(format!("http://127.0.0.1:{port}/{token}/~fill/{id}"))
    }

    /// Deletes the videos not played for `days`, then the oldest while they take more than 10 GB.
    fn prune(dir: &std::path::Path, keep: &std::path::Path, days: u64) {
        let max_age = std::time::Duration::from_secs(days * 24 * 3600);
        const CAP: u64 = 10 << 30;
        let mut all: Vec<_> = std::fs::read_dir(dir).into_iter().flatten().flatten().map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "part") && p != keep)
            .map(|p| (super::fill::used(&p).unwrap_or(std::time::UNIX_EPOCH), super::fill::stored(&p), p))
            .collect();
        all.sort_by_key(|a| a.0);
        let mut total: u64 = all.iter().map(|a| a.1).sum::<u64>() + super::fill::stored(keep);
        for (used, size, p) in all {
            if total > CAP || used.elapsed().unwrap_or_default() > max_age {
                let _ = std::fs::remove_file(p.with_extension("meta"));
                let _ = std::fs::remove_file(p.with_extension("json"));
                let _ = std::fs::remove_file(&p);
                total -= size;
            }
        }
    }

    #[cfg(desktop)]
    pub fn remux_base() -> Result<String, String> {
        let (port, token) = server()?;
        Ok(format!("http://127.0.0.1:{port}/{token}/~remux/"))
    }

    pub fn poster_base() -> Result<String, String> {
        let (port, token) = server()?;
        Ok(format!("http://127.0.0.1:{port}/{token}/~poster/"))
    }

    fn server() -> Result<(u16, String), String> {
        Ok(match SERVER.get() {
            Some(s) => s.clone(),
            None => {
                let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
                let port = listener.local_addr().map_err(|e| e.to_string())?.port();
                let token = token();
                let t = token.clone();
                std::thread::spawn(move || {
                    for stream in listener.incoming().flatten() {
                        let t = t.clone();
                        std::thread::spawn(move || { let _ = handle(stream, &t); });
                    }
                });
                SERVER.get_or_init(|| (port, token)).clone()
            }
        })
    }

    fn token() -> String {
        use std::hash::BuildHasher;
        let h = std::collections::hash_map::RandomState::new();
        format!("{:016x}{:016x}", h.hash_one(std::time::SystemTime::now()), h.hash_one(std::process::id()))
    }

    fn encode(s: &str) -> String {
        s.bytes()
            .map(|b| if b.is_ascii_alphanumeric() || b"-._~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") })
            .collect()
    }

    fn decode(s: &str) -> Option<String> {
        let b = s.as_bytes();
        let mut out = Vec::with_capacity(b.len());
        let mut i = 0;
        while i < b.len() {
            if b[i] == b'%' {
                out.push(u8::from_str_radix(s.get(i + 1..i + 3)?, 16).ok()?);
                i += 3;
            } else {
                out.push(b[i]);
                i += 1;
            }
        }
        String::from_utf8(out).ok()
    }

    fn handle(mut stream: TcpStream, token: &str) -> std::io::Result<()> {
        let mut reader = BufReader::new(stream.try_clone()?);
        let mut line = String::new();
        reader.read_line(&mut line)?;
        let mut parts = line.split_whitespace();
        let (method, target) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
        let mut range = None;
        loop {
            let mut h = String::new();
            if reader.read_line(&mut h)? == 0 || h.trim().is_empty() {
                break;
            }
            if let Some((k, v)) = h.split_once(':') {
                if k.eq_ignore_ascii_case("range") {
                    range = v.trim().strip_prefix("bytes=").map(str::to_owned);
                }
            }
        }
        // `~poster/<title>/<url>`, both percent-encoded (see `posters::get`)
        if let Some(rest) = target.strip_prefix(&format!("/{token}/~poster/")) {
            let (title, url) = rest.split_once('/').unwrap_or(("", rest));
            return match decode(title).zip(decode(url)).and_then(|(t, u)| super::posters::get(&u, Some(&t))) {
                Some((bytes, mime)) => {
                    // the same URL is always the same picture: let the webview keep it too
                    write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nCache-Control: max-age=604800\r\nConnection: close\r\n\r\n", bytes.len())?;
                    if method == "GET" {
                        stream.write_all(&bytes)?;
                    }
                    Ok(())
                }
                None => stream.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"),
            };
        }
        // `~remux/<id>/<file>`: the HLS playlist and segments ffmpeg writes (`remux`)
        #[cfg(desktop)]
        if let Some(rest) = target.strip_prefix(&format!("/{token}/~remux/")) {
            let file = rest.split_once('/').and_then(|(id, name)| super::remux::file(id, name)).filter(|_| method == "GET" || method == "HEAD");
            let Some(file) = file else {
                return stream.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
            };
            if file.extension().is_some_and(|x| x == "m3u8") {
                // still growing: from the start, not the live edge
                let list = std::fs::read_to_string(&file)?.replacen("#EXTM3U\n", "#EXTM3U\n#EXT-X-START:TIME-OFFSET=0\n", 1);
                write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/vnd.apple.mpegurl\r\nContent-Length: {}\r\nCache-Control: no-cache\r\nAccess-Control-Allow-Origin: *\r\nConnection: close\r\n\r\n", list.len())?;
                if method == "GET" {
                    stream.write_all(list.as_bytes())?;
                }
                return Ok(());
            }
            let mut f = File::open(&file)?;
            let len = f.metadata()?.len();
            let (start, end) = head(&mut stream, range, len, "video/mp4")?;
            if method == "GET" {
                f.seek(SeekFrom::Start(start))?;
                std::io::copy(&mut f.take(end + 1 - start), &mut stream)?;
            }
            return Ok(());
        }
        if let Some(id) = target.strip_prefix(&format!("/{token}/~fill/")) {
            let f = FILL.lock().unwrap().as_ref().filter(|(n, _)| id == n.to_string()).map(|(_, f)| f.clone());
            return match f {
                Some(f) if method == "GET" || method == "HEAD" => send_fill(stream, &f, method, range),
                _ => stream.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"),
            };
        }
        let path = target
            .strip_prefix('/')
            .and_then(|t| t.split_once('/'))
            .filter(|(t, _)| *t == token)
            .and_then(|(_, p)| decode(p))
            .filter(|p| ALLOWED.lock().unwrap().as_ref().is_some_and(|a| a.contains(p)));
        let Some(path) = path.filter(|_| method == "GET" || method == "HEAD") else {
            return stream.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
        };
        let mut file = File::open(&path)?;
        let len = file.metadata()?.len();
        let (start, end) = head(&mut stream, range, len, super::mime(&path))?;
        if method == "GET" {
            file.seek(SeekFrom::Start(start))?;
            std::io::copy(&mut file.take(end + 1 - start), &mut stream)?;
        }
        Ok(())
    }

    /// Writes the response head for `range` of a `len`-byte file; the byte range to send follows.
    fn head(stream: &mut TcpStream, range: Option<String>, len: u64, mime: &str) -> std::io::Result<(u64, u64)> {
        let (start, end, partial) = range
            .and_then(|r| {
                let (a, b) = r.split_once('-')?;
                let start = a.parse::<u64>().ok()?;
                let end = b.parse::<u64>().unwrap_or(len.saturating_sub(1)).min(len.saturating_sub(1));
                (start <= end).then_some((start, end))
            })
            .map_or((0, len.saturating_sub(1), false), |(s, e)| (s, e, true));
        write!(
            stream,
            "HTTP/1.1 {}\r\nContent-Type: {}\r\nAccept-Ranges: bytes\r\nContent-Length: {}\r\n{}Access-Control-Allow-Origin: *\r\nConnection: close\r\n\r\n",
            if partial { "206 Partial Content" } else { "200 OK" },
            mime,
            end + 1 - start,
            if partial { format!("Content-Range: bytes {start}-{end}/{len}\r\n") } else { String::new() },
        )?;
        Ok((start, end))
    }

    /// Serves the video being downloaded, waiting for the parts not there yet; sends the player to
    /// the source itself when it can't be downloaded here (HLS, unknown length, unreachable).
    fn send_fill(mut stream: TcpStream, f: &super::fill::Fill, method: &str, range: Option<String>) -> std::io::Result<()> {
        let Some((len, mime)) = f.head() else {
            return write!(stream, "HTTP/1.1 302 Found\r\nLocation: {}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n", f.url);
        };
        let (start, end) = head(&mut stream, range, len, &mime)?;
        if method == "GET" {
            let file = f.file()?;
            let mut off = start;
            while off <= end {
                let buf = f.read(&file, off, (end + 1 - off).min(256 << 10))?;
                stream.write_all(&buf)?;
                off += buf.len() as u64;
            }
        }
        Ok(())
    }
}

/// Fetches a subtitle file for the player as WebVTT. Done here rather than in the webview: subtitle
/// hosts rarely send CORS headers, and SRT has to become VTT for the track element anyway.
#[tauri::command]
pub fn video_subtitle(db: tauri::State<crate::db::Db>, url: String) -> Result<String, String> {
    if !url.starts_with("https://") && !url.starts_with("http://") {
        return Err("Alamat subtitle tidak valid.".into());
    }
    // fetched once, then served from the database: works offline and spares the addon's host
    if let Ok(Some(vtt)) = db.subtitle(&url) {
        return Ok(vtt);
    }
    let vtt = fetch_subtitle(&url)?;
    let _ = db.save_subtitle(&url, &vtt);
    Ok(vtt)
}

fn fetch_subtitle(url: &str) -> Result<String, String> {
    let res = attohttpc::get(url).header("User-Agent", "Mozilla/5.0").timeout(std::time::Duration::from_secs(20))
        .send().map_err(|e| e.to_string())?;
    if !res.is_success() {
        return Err(format!("HTTP {}", res.status()));
    }
    let bytes = res.bytes().map_err(|e| e.to_string())?;
    if bytes.len() > 5 << 20 {
        return Err("File subtitle terlalu besar.".into());
    }
    // most subtitle files are UTF-8; old ones are Latin-1, where every byte maps to the same code point
    let text = String::from_utf8(bytes.clone()).unwrap_or_else(|_| bytes.iter().map(|&b| b as char).collect());
    let text = text.trim_start_matches('\u{feff}').replace("\r\n", "\n").replace('\r', "\n");
    if text.starts_with("WEBVTT") {
        return Ok(text);
    }
    // SRT → VTT: same cues, but a dot before the milliseconds and a header line
    let body: Vec<String> = text.lines()
        .map(|l| if l.contains("-->") { l.replace(',', ".") } else { l.to_string() })
        .collect();
    Ok(format!("WEBVTT\n\n{}", body.join("\n")))
}
