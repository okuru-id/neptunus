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

/// A URL the webview can stream: presigned for S3, the loopback server for local files on desktop.
/// `None` on phones: the UI uses `convertFileSrc` (the asset protocol) there.
#[tauri::command]
pub async fn video_url(app: AppHandle, path: String) -> Result<Option<String>, String> {
    if path.starts_with("s3://") {
        return crate::blocking(move || crate::s3::presigned(&app, &path)).await.map(Some);
    }
    #[cfg(desktop)]
    return serve::url(&path).map(Some);
    #[cfg(mobile)]
    Ok(None)
}

/// WebKitGTK can't play media from a custom scheme (`asset://`), so desktop serves local videos over
/// `http://127.0.0.1` with Range support. Only files handed out by `url` are served, under a random
/// per-run token.
#[cfg(desktop)]
mod serve {
    use std::collections::HashSet;
    use std::fs::File;
    use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::{Mutex, OnceLock};

    static SERVER: OnceLock<(u16, String)> = OnceLock::new();
    static ALLOWED: Mutex<Option<HashSet<String>>> = Mutex::new(None);

    pub fn url(path: &str) -> Result<String, String> {
        let (port, token) = match SERVER.get() {
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
        };
        ALLOWED.lock().unwrap().get_or_insert_with(HashSet::new).insert(path.to_owned());
        Ok(format!("http://127.0.0.1:{port}/{token}/{}", encode(path)))
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

    fn mime(path: &str) -> &'static str {
        match path.rsplit('.').next().map(str::to_lowercase).as_deref() {
            Some("webm") => "video/webm",
            Some("mkv") => "video/x-matroska",
            Some("mov") => "video/quicktime",
            _ => "video/mp4",
        }
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
            mime(&path),
            end + 1 - start,
            if partial { format!("Content-Range: bytes {start}-{end}/{len}\r\n") } else { String::new() },
        )?;
        if method == "GET" {
            file.seek(SeekFrom::Start(start))?;
            std::io::copy(&mut file.take(end + 1 - start), &mut stream)?;
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
