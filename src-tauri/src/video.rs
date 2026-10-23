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

#[cfg(target_os = "ios")]
mod fill;

/// A URL the webview can stream: presigned for S3, the loopback server for local files on desktop.
/// `None` for local files on phones: the UI uses `convertFileSrc` (the asset protocol) there.
/// On iOS remote videos go through the loopback server too, which downloads them in full (`fill`).
#[tauri::command]
pub async fn video_url(app: AppHandle, path: String) -> Result<Option<String>, String> {
    let remote = path.starts_with("http://") || path.starts_with("https://");
    let url = if path.starts_with("s3://") {
        let app = app.clone();
        crate::blocking(move || crate::s3::presigned(&app, &path)).await?
    } else if remote {
        path
    } else {
        #[cfg(desktop)]
        return serve::url(&path).map(Some);
        #[cfg(mobile)]
        return Ok(None);
    };
    #[cfg(target_os = "ios")]
    if !url.split(['?', '#']).next().unwrap_or("").to_lowercase().ends_with(".m3u8") && !url.contains(char::is_whitespace) {
        let dir = app.path().app_cache_dir().map_err(|e| e.to_string())?.join("video");
        return serve::fill(url, &dir).map(Some);
    }
    let _ = app;
    Ok(Some(url))
}

/// The content type the loopback server sends for a video, by its extension.
#[cfg(any(desktop, target_os = "ios"))]
fn mime(path: &str) -> &'static str {
    match path.split(['?', '#']).next().unwrap_or("").rsplit('.').next().map(str::to_lowercase).as_deref() {
        Some("webm") => "video/webm",
        Some("mkv") => "video/x-matroska",
        Some("mov") => "video/quicktime",
        _ => "video/mp4",
    }
}

/// How far the playing video is downloaded ahead of `at` (a fraction of it), as a fraction of it.
/// `None` when it isn't being downloaded here.
#[tauri::command]
pub fn video_buffered(at: f64) -> Option<f64> {
    #[cfg(target_os = "ios")]
    return serve::FILL.lock().unwrap().as_ref()?.1.buffered(at);
    #[cfg(not(target_os = "ios"))]
    { let _ = at; None }
}

/// The player of `url` closed: stops its download and deletes what it wrote. Another video may have
/// started since, so only that one's.
#[tauri::command]
pub fn video_stop(url: String) {
    #[cfg(target_os = "ios")]
    {
        let mut cur = serve::FILL.lock().unwrap();
        if cur.as_ref().is_some_and(|(id, _)| url.ends_with(&format!("/~fill/{id}"))) {
            cur.take().unwrap().1.close();
        }
    }
    #[cfg(not(target_os = "ios"))]
    let _ = url;
}

/// WebKitGTK can't play media from a custom scheme (`asset://`), so desktop serves local videos over
/// `http://127.0.0.1` with Range support. Only files handed out by `url` are served, under a random
/// per-run token. On iOS it serves the remote video being downloaded (`fill`) instead.
#[cfg(any(desktop, target_os = "ios"))]
#[cfg_attr(target_os = "ios", allow(dead_code))]
mod serve {
    use std::collections::HashSet;
    use std::fs::File;
    use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::{Mutex, OnceLock};

    static SERVER: OnceLock<(u16, String)> = OnceLock::new();
    static ALLOWED: Mutex<Option<HashSet<String>>> = Mutex::new(None);
    /// The remote video being downloaded, by a number that changes with each one.
    #[cfg(target_os = "ios")]
    pub static FILL: Mutex<Option<(u64, std::sync::Arc<super::fill::Fill>)>> = Mutex::new(None);

    pub fn url(path: &str) -> Result<String, String> {
        let (port, token) = server()?;
        ALLOWED.lock().unwrap().get_or_insert_with(HashSet::new).insert(path.to_owned());
        Ok(format!("http://127.0.0.1:{port}/{token}/{}", encode(path)))
    }

    /// Starts downloading `url` into `dir`, dropping the previous video's download.
    #[cfg(target_os = "ios")]
    pub fn fill(url: String, dir: &std::path::Path) -> Result<String, String> {
        let (port, token) = server()?;
        let mut cur = FILL.lock().unwrap();
        let id = match cur.take() {
            Some((id, f)) => {
                f.close();
                id + 1
            }
            None => {
                // leftovers from the last run
                let _ = std::fs::remove_dir_all(dir);
                0
            }
        };
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        *cur = Some((id, super::fill::Fill::start(url, dir.join(format!("{id}.part")))));
        Ok(format!("http://127.0.0.1:{port}/{token}/~fill/{id}"))
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
        #[cfg(target_os = "ios")]
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
            "HTTP/1.1 {}\r\nContent-Type: {}\r\nAccept-Ranges: bytes\r\nContent-Length: {}\r\n{}Connection: close\r\n\r\n",
            if partial { "206 Partial Content" } else { "200 OK" },
            mime,
            end + 1 - start,
            if partial { format!("Content-Range: bytes {start}-{end}/{len}\r\n") } else { String::new() },
        )?;
        Ok((start, end))
    }

    /// Serves the video being downloaded, waiting for the parts not there yet; sends the player to
    /// the source itself when it can't be downloaded here (HLS, unknown length, unreachable).
    #[cfg(target_os = "ios")]
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
