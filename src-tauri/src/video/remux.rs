//! Desktop: a film the webview's player refuses (MKV it won't take, AC3/DTS/E-AC3 audio) is remuxed by
//! the system's `ffmpeg` while it plays: the video is copied as is when it is 8-bit H.264 (else, HEVC or
//! 10-bit say, made H.264), the first audio track (players take one per segment) becomes stereo AAC, cut into an HLS playlist of fMP4 segments that the loopback
//! server hands to the same player (`~remux/<id>/index.m3u8`). Only one runs at a time; a new one, or the
//! player closing, stops it.

use std::path::PathBuf;
use std::process::{Child, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager};

/// The remux running: its number (in the URL), ffmpeg, and the folder it writes.
static CURRENT: Mutex<Option<(u64, Child, PathBuf)>> = Mutex::new(None);

/// `None` when ffmpeg (and ffprobe, for the length) are installed; else how to install them.
#[tauri::command]
pub async fn remux_missing() -> Option<&'static str> {
    crate::blocking(|| Ok::<_, String>(crate::ffmpeg::missing())).await.ok().flatten()
}

/// Remuxes `url` (what the player was given: the loopback server's address of the film) into HLS and
/// returns the playlist's address with the film's length in seconds, once the first segment is there.
#[tauri::command]
pub async fn video_remux(app: AppHandle, url: String) -> Result<(String, f64), String> {
    let dir = app.path().app_cache_dir().map_err(|e| e.to_string())?.join("remux");
    crate::blocking(move || start(url, dir)).await
}

fn start(url: String, dir: PathBuf) -> Result<(String, f64), String> {
    let id = stop().map_or(0, |id| id + 1);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    // HLS still being written has no length of its own; the player is told this one
    let probe = crate::ffmpeg::command("ffprobe")
        .args(["-v", "error", "-select_streams", "v:0", "-show_entries", "stream=codec_name,pix_fmt:format=duration", "-of", "default=nw=1", &url])
        .output().map_err(|e| format!("ffprobe tidak bisa dijalankan ({e})."))?;
    let probe = String::from_utf8_lossy(&probe.stdout);
    let field = |k: &str| probe.lines().find_map(|l| l.strip_prefix(k)?.strip_prefix('=')).unwrap_or("");
    let duration = field("duration").parse::<f64>().unwrap_or(0.0);
    let video: &[&str] = if field("codec_name") == "h264" && field("pix_fmt") == "yuv420p" {
        &["-c:v", "copy"]
    } else {
        &["-c:v", "libx264", "-preset", "veryfast", "-crf", "21", "-pix_fmt", "yuv420p"]
    };
    vlog!("remux #{id}: {} {}, {duration:.0}s -> {}", field("codec_name"), field("pix_fmt"), if video[1] == "copy" { "copied" } else { "H.264" });
    let began = Instant::now();
    let (playlist, log) = (dir.join("index.m3u8"), dir.join("ffmpeg.log"));
    let log_file = std::fs::File::create(&log).map_err(|e| e.to_string())?;
    let mut child = crate::ffmpeg::command("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-nostdin", "-i", &url])
        .args(["-map", "0:v:0", "-map", "0:a:0?"])
        .args(video)
        .args(["-c:a", "aac", "-ac", "2", "-b:a", "192k", "-sn", "-dn"])
        .args(["-f", "hls", "-hls_time", "6", "-hls_list_size", "0", "-hls_playlist_type", "event"])
        .args(["-hls_segment_type", "fmp4", "-hls_fmp4_init_filename", "init.mp4"])
        .arg("-hls_segment_filename").arg(dir.join("seg%05d.m4s"))
        .arg(&playlist)
        .stdin(Stdio::null()).stdout(Stdio::null()).stderr(log_file)
        .spawn().map_err(|e| format!("ffmpeg tidak bisa dijalankan ({e})."))?;
    // the player can start once a segment is listed
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        if std::fs::read_to_string(&playlist).is_ok_and(|p| p.contains("#EXTINF")) {
            vlog!("remux #{id}: first segment in {:.1}s", began.elapsed().as_secs_f32());
            break;
        }
        if let Ok(Some(_)) = child.try_wait() {
            let err = std::fs::read_to_string(&log).unwrap_or_default();
            let why = err.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("ffmpeg berhenti").trim().to_owned();
            vlog!("remux #{id}: ffmpeg stopped:\n{}", err.trim_end());
            return Err(format!("Film ini tidak bisa diubah formatnya ({why})."));
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            return Err("Film ini terlalu lama disiapkan untuk diputar.".into());
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    *CURRENT.lock().unwrap() = Some((id, child, dir));
    Ok((format!("{}{id}/index.m3u8", super::serve::remux_base()?), duration))
}

/// Stops the running remux; returns its number.
fn stop() -> Option<u64> {
    let (id, mut child, _) = CURRENT.lock().unwrap().take()?;
    vlog!("remux #{id}: stopped");
    let _ = child.kill();
    let _ = child.wait();
    Some(id)
}

/// The player closed: ffmpeg stops (its folder goes with the next remux).
#[tauri::command]
pub fn video_remux_stop() {
    stop();
}

/// A file of remux `id`, for the loopback server: only plain names in its folder.
pub fn file(id: &str, name: &str) -> Option<PathBuf> {
    let cur = CURRENT.lock().unwrap();
    let (n, _, dir) = cur.as_ref()?;
    (id == n.to_string() && !name.is_empty() && !name.contains(['/', '\\']) && !name.starts_with('.')).then(|| dir.join(name))
}
