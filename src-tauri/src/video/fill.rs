//! A remote video downloaded in full to a cache file while it plays, served to the player by the
//! loopback server. On phones the webview's player decides how far ahead it reads and stops soon
//! after the play position (a few minutes for iOS's AVPlayer, under half a minute for Android's
//! Chromium); going through this, the whole file comes down anyway and the player reads it from disk. Only one video at a time: a new one (or closing the player) drops
//! the previous download and its file.

use std::fs::File;
use std::io::Read;
use std::os::unix::fs::FileExt;
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// Download progress is tracked per chunk of this many bytes.
pub const CHUNK: u64 = 1 << 20;
/// Failed requests in a row before giving up on the download.
const TRIES: u32 = 5;

pub struct Fill {
    pub url: String,
    path: PathBuf,
    s: Mutex<State>,
    cv: Condvar,
}

#[derive(Default)]
struct State {
    len: Option<u64>,
    mime: String,
    /// not worth proxying (HLS, no length, unreachable): the player is sent to `url` itself
    passthrough: bool,
    /// the download stopped for good; what's on disk is still served
    failed: bool,
    closed: bool,
    done: Vec<bool>,
    /// the request in flight: its first byte, how far it has written, and when it started
    run: (u64, u64, Option<Instant>),
    /// offsets the player is blocked on
    waiting: Vec<u64>,
    /// last offset the player read, where the download carries on from
    play: u64,
}

impl State {
    /// Bytes readable at `off` without waiting.
    fn ready(&self, off: u64) -> u64 {
        let Some(len) = self.len else { return 0 };
        if off >= len {
            return 0;
        }
        let mut end = off;
        let mut i = off / CHUNK;
        while self.done.get(i as usize) == Some(&true) {
            i += 1;
            end = (i * CHUNK).min(len);
        }
        if (self.run.0..self.run.1).contains(&end) {
            end = self.run.1;
        }
        end - off
    }

    fn chunks(&self) -> u64 {
        self.done.len() as u64
    }

    /// Where the next request should start: a blocked read first, then the first missing chunk
    /// from the play position on, wrapping around to the start.
    fn next(&self) -> Option<u64> {
        if let Some(&w) = self.waiting.iter().find(|&&w| self.ready(w) == 0) {
            return Some(w / CHUNK);
        }
        let from = self.play / CHUNK;
        (from..self.chunks()).chain(0..from).find(|&i| !self.done[i as usize])
    }
}

impl Fill {
    pub fn start(url: String, path: PathBuf) -> Arc<Self> {
        let f = Arc::new(Fill { url, path, s: Mutex::default(), cv: Condvar::new() });
        let t = f.clone();
        std::thread::spawn(move || t.download());
        f
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.s.lock().unwrap()
    }

    pub fn close(&self) {
        self.lock().closed = true;
        self.cv.notify_all();
        let _ = std::fs::remove_file(&self.path);
    }

    /// Waits for the first response: `(length, mime)`, or `None` to send the player to the source.
    pub fn head(&self) -> Option<(u64, String)> {
        let deadline = Instant::now() + Duration::from_secs(30);
        let mut s = self.lock();
        loop {
            if s.passthrough || s.closed {
                return None;
            }
            if let Some(len) = s.len {
                return Some((len, s.mime.clone()));
            }
            let left = deadline.checked_duration_since(Instant::now())?;
            s = self.cv.wait_timeout(s, left).unwrap().0;
        }
    }

    /// Reads up to `max` bytes at `off`, waiting for the download to get there.
    pub fn read(&self, file: &File, off: u64, max: u64) -> std::io::Result<Vec<u8>> {
        let mut s = self.lock();
        s.play = off;
        let n = loop {
            let n = s.ready(off);
            if n > 0 {
                break n;
            }
            if s.closed || s.failed {
                return Err(std::io::Error::other("download stopped"));
            }
            s.waiting.push(off);
            self.cv.notify_all();
            s = self.cv.wait_timeout(s, Duration::from_secs(1)).unwrap().0;
            let i = s.waiting.iter().position(|&w| w == off).unwrap();
            s.waiting.swap_remove(i);
        };
        drop(s);
        let mut buf = vec![0; n.min(max) as usize];
        file.read_exact_at(&mut buf, off)?;
        Ok(buf)
    }

    pub fn file(&self) -> std::io::Result<File> {
        File::open(&self.path)
    }

    /// Fraction of the file downloaded without a gap from `at` (a fraction of the file) on.
    pub fn buffered(&self, at: f64) -> Option<f64> {
        let s = self.lock();
        let len = s.len.filter(|&l| l > 0)?;
        let off = ((at.clamp(0.0, 1.0) * len as f64) as u64).min(len - 1);
        Some((off + s.ready(off)) as f64 / len as f64)
    }

    fn download(&self) {
        let file = match File::create(&self.path) {
            Ok(f) => f,
            Err(_) => return self.give_up(),
        };
        let mut fails = 0;
        loop {
            let start = {
                let mut s = self.lock();
                loop {
                    if s.closed {
                        return;
                    }
                    if s.len.is_none() {
                        break 0;
                    }
                    if let Some(i) = s.next() {
                        break i * CHUNK;
                    }
                    s = self.cv.wait(s).unwrap();
                }
            };
            match self.fetch(&file, start) {
                Ok(true) => fails = 0,
                Ok(false) => return,
                Err(_) => {
                    fails += 1;
                    if fails >= TRIES {
                        return self.give_up();
                    }
                    std::thread::sleep(Duration::from_secs(fails as u64));
                }
            }
        }
    }

    fn give_up(&self) {
        let mut s = self.lock();
        if s.len.is_none() {
            s.passthrough = true;
        }
        s.failed = true;
        drop(s);
        self.cv.notify_all();
    }

    /// One request from `start` on, written to the file until the player needs another part of it.
    /// `Ok(false)` when there's nothing more to do.
    fn fetch(&self, file: &File, start: u64) -> Result<bool, String> {
        let res = attohttpc::get(&self.url)
            .header("User-Agent", "Mozilla/5.0")
            .header("Range", format!("bytes={start}-"))
            .connect_timeout(Duration::from_secs(15))
            .read_timeout(Duration::from_secs(30))
            .send()
            .map_err(|e| e.to_string())?;
        let (status, headers, mut body) = res.split();
        if !status.is_success() {
            // refused outright: let the player try it and show its own error
            if self.lock().len.is_none() && status.is_client_error() {
                self.give_up();
                return Ok(false);
            }
            return Err(format!("HTTP {status}"));
        }
        let header = |k: &str| headers.get(k).and_then(|v| v.to_str().ok()).unwrap_or("");
        // a server without ranges sends the whole file
        let ranged = status.as_u16() == 206;
        let start = if ranged { start } else { 0 };
        let total = if ranged {
            header("content-range").rsplit('/').next().and_then(|t| t.parse::<u64>().ok())
        } else {
            header("content-length").parse::<u64>().ok()
        };
        let mut s = self.lock();
        if s.len.is_none() {
            let mime = header("content-type").to_ascii_lowercase();
            match total {
                Some(len) if len > 0 && !mime.contains("mpegurl") => {
                    s.len = Some(len);
                    s.mime = if mime.starts_with("video/") { mime } else { super::mime(&self.url).into() };
                    s.done = vec![false; len.div_ceil(CHUNK) as usize];
                }
                _ => {
                    s.passthrough = true;
                    drop(s);
                    self.cv.notify_all();
                    return Ok(false);
                }
            }
        }
        let len = s.len.unwrap();
        s.run = (start, start, Some(Instant::now()));
        drop(s);
        self.cv.notify_all();
        let (mut pos, mut mark) = (start, start / CHUNK);
        let mut buf = vec![0; 64 << 10];
        loop {
            let n = body.read(&mut buf).map_err(|e| e.to_string())?;
            if n == 0 {
                return if pos >= len { Ok(true) } else { Err("connection closed early".into()) };
            }
            let n = n.min(len.saturating_sub(pos) as usize);
            if let Err(e) = file.write_all_at(&buf[..n], pos) {
                // out of space: keep serving what's there rather than retrying
                self.lock().failed = true;
                self.cv.notify_all();
                return Err(e.to_string());
            }
            pos += n as u64;
            let mut s = self.lock();
            s.run.1 = pos;
            while mark < s.chunks() && ((mark + 1) * CHUNK).min(len) <= pos {
                s.done[mark as usize] = true;
                mark += 1;
            }
            let moved = s.closed || pos >= len || (ranged && self.should_move(&s, mark, pos));
            if moved {
                s.run = (0, 0, None);
            }
            let closed = s.closed;
            drop(s);
            self.cv.notify_all();
            if closed {
                return Ok(false);
            }
            if moved {
                return Ok(true);
            }
        }
    }

    /// Whether to drop this request for another one: the player is blocked somewhere it won't
    /// reach soon, or it has caught up with a part already downloaded.
    fn should_move(&self, s: &State, mark: u64, pos: u64) -> bool {
        if s.done.get(mark as usize) == Some(&true) {
            return true;
        }
        let (from, _, since) = s.run;
        // let every request make some progress, or two reads far apart would take turns forever
        let settled = pos - from >= CHUNK || since.is_some_and(|t| t.elapsed() > Duration::from_secs(3));
        settled && s.waiting.iter().any(|&w| s.ready(w) == 0 && (w < from || w > pos + 2 * CHUNK))
    }
}
