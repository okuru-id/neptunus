//! Linux: films can play in the system's `mpv` instead of the webview, which decodes far more formats
//! (MKV, HEVC 10-bit, AC3/DTS) and keeps going on slow streams. mpv runs in its own window with its own
//! controls; the app only starts it and listens on its JSON IPC socket for how far the film got, which
//! comes back as the `mpv-closed` event so the place is kept like with the built-in player.

use super::Sub;
use serde::Serialize;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};

/// The mpv started last; a new film closes it.
static CURRENT: Mutex<Option<Child>> = Mutex::new(None);

#[derive(Serialize, Clone)]
struct Closed {
    path: String,
    pos: f64,
    duration: f64,
    eof: bool,
    /// why mpv gave up on the film (its own words and the last errors of its log), when it did
    error: Option<String>,
}

/// Whether mpv is installed.
#[tauri::command]
pub fn mpv_available() -> bool {
    Command::new("mpv").arg("--version").stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|s| s.success())
}

/// Plays `url` (from `video_url`) in mpv, from `start` seconds, with `subs` to switch between (mpv's
/// `j`): the first one shown when `sub_on`, none otherwise.
/// `headers` are what the stream's host wants (`proxyHeaders`): sent by mpv itself too, for the streams
/// the loopback server passes on (HLS, or a host that didn't answer it). `path` only names the film in
/// the `mpv-closed` event.
#[tauri::command]
pub async fn mpv_play(app: AppHandle, path: String, url: String, title: String, start: f64, subs: Vec<Sub>, sub_on: bool, headers: Option<std::collections::HashMap<String, String>>) -> Result<(), String> {
    let dir = app.path().app_cache_dir().map_err(|e| e.to_string())?.join("mpv");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let (sock, log) = (dir.join("ipc.sock"), dir.join("mpv.log"));
    if let Some(mut old) = CURRENT.lock().unwrap().take() {
        let _ = old.kill();
        let _ = old.wait();
    }
    let _ = std::fs::remove_file(&sock);
    let mut cmd = Command::new("mpv");
    cmd.arg("--force-window=immediate")
        .arg("--keep-open=no")
        // a stream it can't open goes no further (yt-dlp would only add a delay, and get the URL)
        .arg("--ytdl=no")
        .arg(format!("--title={title} — Neptunus"))
        .arg(format!("--force-media-title={title}"))
        .arg(format!("--input-ipc-server={}", sock.display()))
        // the loopback server already keeps the whole film on disk
        .arg("--cache=yes")
        .arg("--demuxer-max-bytes=512MiB")
        .arg(format!("--log-file={}", log.display()));
    for (k, v) in headers.iter().flatten() {
        match k.to_ascii_lowercase().as_str() {
            "user-agent" => cmd.arg(format!("--user-agent={v}")),
            "referer" => cmd.arg(format!("--referrer={v}")),
            _ => cmd.arg(format!("--http-header-fields-append={k}: {v}")),
        };
    }
    if start > 0.0 {
        cmd.arg(format!("--start={start}"));
    }
    vlog!("mpv: {}", super::short(&url));
    let child = cmd.arg("--").arg(&url).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn()
        .map_err(|e| format!("mpv tidak bisa dijalankan ({e})."))?;
    let pid = child.id();
    *CURRENT.lock().unwrap() = Some(child);
    std::thread::spawn(move || {
        let (a, subs_dir) = (app.clone(), dir.join("subs"));
        let (pos, duration, eof, failed) = watch(&sock, move |w| {
            std::thread::spawn(move || add_subs(&a, w, &subs, sub_on, &subs_dir));
        });
        // reap it, unless a newer film already replaced it
        let mut cur = CURRENT.lock().unwrap();
        if cur.as_ref().is_some_and(|c| c.id() == pid) {
            if let Some(mut c) = cur.take() {
                let _ = c.wait();
            }
        }
        drop(cur);
        // closed before anything played: say why, from mpv's own reason and its log
        let error = (failed.is_some() || duration == 0.0).then(|| {
            let log = std::fs::read_to_string(&log).unwrap_or_default();
            let lines: Vec<&str> = log.lines().filter(|l| l.contains("[e]") || l.contains("[w]")).collect();
            let tail = lines[lines.len().saturating_sub(3)..].iter().map(|l| l.splitn(4, ']').nth(3).map_or(*l, str::trim)).collect::<Vec<_>>().join(" · ");
            [failed.unwrap_or_default(), tail].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" — ")
        });
        vlog!("mpv closed at {pos:.0}/{duration:.0}s{}{}", if eof { ", at the end" } else { "" }, error.as_deref().map(|e| format!(": {e}")).unwrap_or_default());
        let _ = app.emit("mpv-closed", Closed { path, pos, duration, eof, error });
    });
    Ok(())
}

/// Follows the playback over mpv's IPC socket until it closes: `(position, duration, reached the end,
/// why it failed)`. `loaded` gets the socket once the film is open, for the subtitles.
fn watch(sock: &std::path::Path, loaded: impl FnOnce(UnixStream)) -> (f64, f64, bool, Option<String>) {
    let mut conn = None;
    for _ in 0..100 {
        if let Ok(c) = UnixStream::connect(sock) {
            conn = Some(c);
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let Some(mut conn) = conn else { return (0.0, 0.0, false, Some("mpv tidak merespons".into())) };
    let _ = conn.write_all(b"{\"command\":[\"observe_property\",1,\"time-pos\"]}\n{\"command\":[\"observe_property\",2,\"duration\"]}\n");
    let (mut pos, mut duration, mut eof, mut failed) = (0.0, 0.0, false, None);
    let mut loaded = conn.try_clone().ok().map(|w| (w, loaded));
    for line in BufReader::new(conn).lines() {
        let Ok(line) = line else { break };
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) else { continue };
        // the film is open (or was before this connection)
        if v["event"] == "file-loaded" || (v["name"] == "duration" && v["data"].is_number()) {
            if let Some((w, f)) = loaded.take() {
                f(w);
            }
        }
        match (v["event"].as_str(), v["name"].as_str()) {
            (Some("property-change"), Some("time-pos")) => pos = v["data"].as_f64().unwrap_or(pos),
            (Some("property-change"), Some("duration")) => duration = v["data"].as_f64().unwrap_or(duration),
            (Some("end-file"), _) => {
                eof = v["reason"] == "eof";
                if v["reason"] == "error" {
                    failed = Some(v["file_error"].as_str().unwrap_or("error").to_owned());
                }
            }
            _ => {}
        }
    }
    (pos, duration, eof, failed)
}

/// `subs` into mpv under their names (`sub-add`, which unlike `--sub-file` takes a title and a language),
/// fetched here first (`fetch_subs`): mpv fetching a remote one itself holds the film up until it has it.
fn add_subs(app: &AppHandle, mut w: UnixStream, subs: &[Sub], sub_on: bool, dir: &std::path::Path) {
    super::fetch_subs(app, subs, sub_on, dir, |i, f| {
        let flag = if i == 0 && sub_on { "select" } else { "auto" };
        let _ = writeln!(w, "{}", serde_json::json!({ "command": ["sub-add", f, flag, subs[i].title, subs[i].lang] }));
    });
}
