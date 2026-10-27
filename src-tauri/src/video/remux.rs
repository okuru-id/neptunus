//! Desktop: a film the webview's player refuses (MKV it won't take, AC3/DTS/E-AC3 audio) is remuxed by
//! the system's `ffmpeg` while it plays: the video is copied as is when it is 8-bit H.264 (else, HEVC or
//! 10-bit say, made H.264) and every audio track becomes stereo AAC, cut into HLS of fMP4 segments, each
//! audio track its own rendition (players take one per segment) so a dub can be picked, that the
//! loopback server hands to the same player (`~remux/<id>/index.m3u8`). Only one runs at a time; a new one, or the
//! player closing, stops it.

use std::collections::HashMap;
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

/// Remuxes `url` into HLS and returns the playlist's address with the film's length in seconds, once
/// the first segment is there. A DASH or HLS source (`index_web.mpd`) comes with the headers its host
/// demands (`Cookie`, `Referer`): ffmpeg must read that one straight from the origin, not the loopback
/// server, or the manifest's relative fragment addresses would not resolve.
#[tauri::command]
pub async fn video_remux(app: AppHandle, url: String, headers: Option<HashMap<String, String>>) -> Result<(String, f64), String> {
    let dir = app.path().app_cache_dir().map_err(|e| e.to_string())?.join("remux");
    crate::blocking(move || start(url, dir, headers.unwrap_or_default())).await
}

/// Why the first try of `start_once` gives up: the second one finds the file's index already cached.
const SLOW: &str = "Film ini terlalu lama disiapkan untuk diputar.";

fn start(url: String, dir: PathBuf, headers: HashMap<String, String>) -> Result<(String, f64), String> {
    // a cold cache makes ffprobe and ffmpeg walk the film's index (a MKV's Cues sit at the end) over
    // the network in small pieces, which can outlast the wait for the first segment; the retry starts
    // with those pieces on disk, and only a second slow one is reported
    match start_once(url.clone(), dir.clone(), &headers) {
        Err(e) if e == SLOW => {
            vlog!("remux: lambat disiapkan, dicoba lagi dengan cache yang sudah terisi");
            start_once(url, dir, &headers)
        }
        r => r,
    }
}

/// `-headers` for ffmpeg and ffprobe: `Cookie`, `Referer` and friends the source's host demands.
fn header_arg(headers: &HashMap<String, String>) -> Vec<String> {
    if headers.is_empty() { return vec![] }
    // a line break inside a value would end the header block early
    // ponytail: no full URL-encode here; hosts that need exotic headers can also pass them raw
    let value = headers.iter().map(|(k, v)| format!("{k}: {}\r\n", v.replace(['\r', '\n'], " "))).collect::<String>();
    vec!["-headers".into(), value]
}

fn start_once(url: String, dir: PathBuf, headers: &HashMap<String, String>) -> Result<(String, f64), String> {
    let id = stop().map_or(0, |id| id + 1);
    let auth = header_arg(headers);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    // HLS still being written has no length of its own; the player is told this one
    let probe = crate::ffmpeg::command("ffprobe")
        .args(&auth)
        .args(["-v", "error", "-select_streams", "v:0", "-show_entries", "stream=codec_name,pix_fmt:format=duration", "-of", "default=nw=1", &url])
        .output().map_err(|e| format!("ffprobe tidak bisa dijalankan ({e})."))?;
    let probe = String::from_utf8_lossy(&probe.stdout);
    let field = |k: &str| probe.lines().find_map(|l| l.strip_prefix(k)?.strip_prefix('=')).unwrap_or("");
    let duration = field("duration").parse::<f64>().unwrap_or(0.0);
    let video: &[&str] = if field("codec_name") == "h264" && field("pix_fmt") == "yuv420p" {
        &["-c:v", "copy"]
    } else {
        &["-c:v", "libx264", "-preset", "veryfast", "-crf", "21", "-pix_fmt", "yuv420p", "-x264-params", "threads=4"]
    };
    let audio = audio_tracks(&url, headers);
    vlog!("remux #{id}: {} {}, {duration:.0}s, {} audio -> {}", field("codec_name"), field("pix_fmt"), audio.len(), if video[1] == "copy" { "copied" } else { "H.264" });
    // the video, and each audio track in one group: `language:` for the player's menu
    let mut map = vec!["-map".to_owned(), "0:v:0".to_owned()];
    let mut streams = vec![if audio.is_empty() { "v:0,name:video".to_owned() } else { "v:0,agroup:aud,name:video".to_owned() }];
    for (i, (lang, _)) in audio.iter().enumerate() {
        map.extend(["-map".to_owned(), format!("0:a:{i}")]);
        let lang = if lang.is_empty() { String::new() } else { format!(",language:{lang}") };
        streams.push(format!("a:{i},agroup:aud,name:a{i}{lang}{}", if i == 0 { ",default:yes" } else { "" }));
    }
    let began = Instant::now();
    let (master, playlist, log) = (dir.join("index.m3u8"), dir.join("video.m3u8"), dir.join("ffmpeg.log"));
    let log_file = std::fs::File::create(&log).map_err(|e| e.to_string())?;
    let mut child = crate::ffmpeg::command("ffmpeg")
        // decode with 2 threads and encode with 4: together a fraction of the cores, still a few
        // times faster than playback, so the transcode stays ahead of the player (raise when it
        // stalls at its edge, or delete for full speed)
        .args(&auth)
        .args(["-hide_banner", "-loglevel", "error", "-nostdin", "-threads", "2", "-i", &url])
        .args(&map)
        .args(video)
        .args(["-c:a", "aac", "-ac", "2", "-b:a", "192k", "-sn", "-dn"])
        .args(["-f", "hls", "-hls_time", "6", "-hls_list_size", "0", "-hls_playlist_type", "event"])
        .args(["-hls_segment_type", "fmp4", "-hls_fmp4_init_filename", "init.mp4"])
        .arg("-hls_segment_filename").arg(dir.join("%v_seg%05d.m4s"))
        .args(["-master_pl_name", "index.m3u8", "-var_stream_map", &streams.join(" ")])
        .arg(dir.join("%v.m3u8"))
        .stdin(Stdio::null()).stdout(Stdio::null()).stderr(log_file)
        .spawn().map_err(|e| format!("ffmpeg tidak bisa dijalankan ({e})."))?;
        // the encode takes every core it is given (over 1000% here); below the app's own priority it
        // still runs at full speed on an idle machine but yields to it, so the interface stays smooth
        #[cfg(unix)]
        unsafe { libc::setpriority(libc::PRIO_PROCESS, child.id(), 10); }
    // the player can start once a segment is listed
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        if master.is_file() && std::fs::read_to_string(&playlist).is_ok_and(|p| p.contains("#EXTINF")) {
            name_tracks(&master, &audio);
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
            let _ = child.wait();
            return Err(SLOW.into());
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    *CURRENT.lock().unwrap() = Some((id, child, dir));
    Ok((format!("{}{id}/index.m3u8", super::serve::remux_base()?), duration))
}

/// The film's audio tracks, in order: language code (ISO 639-2, as tagged) and title, either may be empty.
fn audio_tracks(url: &str, headers: &HashMap<String, String>) -> Vec<(String, String)> {
    let out = crate::ffmpeg::command("ffprobe")
        .args(&header_arg(headers))
        .args(["-v", "error", "-select_streams", "a", "-show_entries", "stream_tags=language,title", "-of", "json", url])
        .output();
    let json: serde_json::Value = out.ok().and_then(|o| serde_json::from_slice(&o.stdout).ok()).unwrap_or_default();
    let tag = |s: &serde_json::Value, k: &str| s["tags"][k].as_str().unwrap_or("").to_owned();
    json["streams"].as_array().map_or(vec![], |l| {
        l.iter().map(|s| (tag(s, "language").chars().filter(|c| c.is_ascii_alphanumeric()).collect(), tag(s, "title"))).collect()
    })
}

/// ffmpeg names the renditions `audio_N`: the track's own title instead, when it has one ("English Dub").
fn name_tracks(master: &std::path::Path, audio: &[(String, String)]) {
    let Ok(list) = std::fs::read_to_string(master) else { return };
    let mut titles = audio.iter().map(|(_, t)| t.replace('"', "'"));
    let list: Vec<String> = list.lines().map(|l| {
        let title = if l.starts_with("#EXT-X-MEDIA:TYPE=AUDIO") { titles.next().unwrap_or_default() } else { String::new() };
        if title.is_empty() { return l.to_owned() }
        l.split(',').map(|kv| if kv.starts_with("NAME=") { format!("NAME=\"{title}\"") } else { kv.to_owned() }).collect::<Vec<_>>().join(",")
    }).collect();
    let _ = std::fs::write(master, list.join("\n") + "\n");
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
