//! A remote video downloaded in full to a cache file while it plays, served to the player by the
//! loopback server. On phones the webview's player decides how far ahead it reads and stops soon
//! after the play position (a few minutes for iOS's AVPlayer, under half a minute for Android's
//! Chromium); going through this, the whole file comes down anyway and the player reads it from disk.
//! Only one video downloads at a time: a new one (or closing the player) stops the previous one.
//! What came down stays on disk with a `.meta` beside it (length, type, chunks done), so the same
//! video opened again (an accidental close, or days later) carries on from there; `serve::prune`
//! deletes it after a while unplayed (a week by default, Settings › Lokal).

use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// Download progress is tracked per chunk of this many bytes.
pub const CHUNK: u64 = 1 << 20;
/// Failed requests in a row before giving up on the download.
const TRIES: u32 = 5;

pub struct Fill {
    pub url: String,
    /// headers the source asks for (a Stremio stream's `behaviorHints.proxyHeaders.request`): many hosts
    /// refuse, or never answer, a request without their Referer, Origin or User-Agent
    headers: Vec<(String, String)>,
    path: PathBuf,
    meta: PathBuf,
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
    /// Downloads `url` into `path`, resuming what an earlier play of it left there.
    pub fn start(url: String, headers: Vec<(String, String)>, path: PathBuf) -> Arc<Self> {
        let meta = path.with_extension("meta");
        let mut s = State::default();
        if let Some((len, mime, done)) = std::fs::read_to_string(&meta).ok().as_deref().and_then(load) {
            if path.metadata().is_ok() && done.len() as u64 == len.div_ceil(CHUNK) {
                (s.len, s.mime, s.done) = (Some(len), mime, done);
            }
        }
        let f = Arc::new(Fill { url, headers, path, meta, s: Mutex::new(s), cv: Condvar::new() });
        let t = f.clone();
        std::thread::spawn(move || t.download());
        f
    }

    /// The cache file this download writes.
    pub fn path(&self) -> &Path {
        &self.path
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.s.lock().unwrap()
    }

    /// Stops the download; the file and its progress stay for next time.
    pub fn close(&self) {
        let mut s = self.lock();
        s.closed = true;
        self.save(&s);
        drop(s);
        self.cv.notify_all();
    }

    /// Writes the progress beside the file (its mtime also says when the video was last played).
    fn save(&self, s: &State) {
        let Some(len) = s.len else { return };
        let done: String = s.done.iter().map(|&d| if d { '1' } else { '0' }).collect();
        let _ = std::fs::write(&self.meta, format!("{len}\n{}\n{done}", s.mime));
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
        read_at(file, &mut buf, off)?;
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
        // kept, not truncated: the chunks marked done in `.meta` are in it
        let file = match File::options().read(true).write(true).create(true).truncate(false).open(&self.path) {
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
        let mut req = attohttpc::get(&self.url);
        if !self.headers.iter().any(|(k, _)| k.eq_ignore_ascii_case("user-agent")) {
            req = req.header("User-Agent", "Mozilla/5.0");
        }
        for (k, v) in &self.headers {
            // the range is ours to set, and the rest of these belong to the connection
            if !["range", "host", "content-length", "connection"].iter().any(|h| k.eq_ignore_ascii_case(h)) {
                use attohttpc::header::{HeaderName, HeaderValue};
                // a malformed one is left out rather than failing the video
                if let (Ok(k), Ok(v)) = (HeaderName::from_bytes(k.as_bytes()), HeaderValue::from_str(v)) {
                    req = req.header(k, v);
                }
            }
        }
        let res = req
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
            if let Err(e) = write_at(file, &buf[..n], pos) {
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
                // every 16 MB, so a killed app loses little
                if mark % 16 == 0 || mark == s.chunks() {
                    self.save(&s);
                }
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

/// `.meta`: the length, the type and one `0`/`1` per chunk.
fn load(m: &str) -> Option<(u64, String, Vec<bool>)> {
    let mut l = m.lines();
    let len = l.next()?.parse().ok()?;
    let mime = l.next()?.to_owned();
    Some((len, mime, l.next().unwrap_or("").chars().map(|c| c == '1').collect()))
}

/// Whether anything of the video at `path` came down: its `.meta` marks a chunk done. The file itself
/// appears as soon as a download starts, also for one that fails before its first byte.
pub fn has_data(path: &Path) -> bool {
    std::fs::read_to_string(path.with_extension("meta")).ok().as_deref().and_then(load).is_some_and(|(_, _, done)| done.contains(&true))
        && path.exists()
}

/// Bytes the video at `path` takes on disk. A partly downloaded one already has its full length, but on
/// Linux, macOS, iOS and Android the parts not written yet take no space (a sparse file), so there it's
/// what its `.meta` marks downloaded. Windows writes the gaps out, so there it is the length.
pub fn stored(path: &Path) -> u64 {
    let len = path.metadata().map_or(0, |m| m.len());
    if cfg!(windows) {
        return len;
    }
    match std::fs::read_to_string(path.with_extension("meta")).ok().as_deref().and_then(load) {
        Some((total, _, done)) => (done.iter().filter(|&&d| d).count() as u64 * CHUNK).min(total).min(len),
        None => len,
    }
}

/// How much of the video at `path` came down, 0 to 1 (its `.meta`); 0 without one.
pub fn progress(path: &Path) -> f64 {
    std::fs::read_to_string(path.with_extension("meta")).ok().as_deref().and_then(load)
        .filter(|(_, _, done)| !done.is_empty())
        .map_or(0.0, |(_, _, done)| done.iter().filter(|&&d| d).count() as f64 / done.len() as f64)
}

/// Last time the video at `path` was played, from its `.meta` (or the file itself).
pub fn used(path: &Path) -> Option<std::time::SystemTime> {
    std::fs::metadata(path.with_extension("meta")).or_else(|_| std::fs::metadata(path)).and_then(|m| m.modified()).ok()
}

// positional reads and writes, so the download and the player's reads don't share a cursor
#[cfg(unix)]
fn read_at(f: &File, buf: &mut [u8], off: u64) -> std::io::Result<()> {
    std::os::unix::fs::FileExt::read_exact_at(f, buf, off)
}
#[cfg(unix)]
fn write_at(f: &File, buf: &[u8], off: u64) -> std::io::Result<()> {
    std::os::unix::fs::FileExt::write_all_at(f, buf, off)
}
#[cfg(windows)]
fn read_at(f: &File, mut buf: &mut [u8], mut off: u64) -> std::io::Result<()> {
    while !buf.is_empty() {
        let n = std::os::windows::fs::FileExt::seek_read(f, buf, off)?;
        if n == 0 {
            return Err(std::io::ErrorKind::UnexpectedEof.into());
        }
        buf = &mut buf[n..];
        off += n as u64;
    }
    Ok(())
}
#[cfg(windows)]
fn write_at(f: &File, mut buf: &[u8], mut off: u64) -> std::io::Result<()> {
    while !buf.is_empty() {
        let n = std::os::windows::fs::FileExt::seek_write(f, buf, off)?;
        buf = &buf[n..];
        off += n as u64;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;

    /// A host that only answers with its Referer, like the ones Stremio addons send `proxyHeaders` for.
    fn picky_host(body: &'static [u8]) -> String {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/film.mp4", l.local_addr().unwrap());
        std::thread::spawn(move || {
            for mut s in l.incoming().flatten() {
                let mut head = String::new();
                let mut r = BufReader::new(s.try_clone().unwrap());
                while r.read_line(&mut head).map_or(false, |n| n > 2) {}
                let lower = head.to_lowercase();
                if !lower.contains("referer: https://addon.example/") || !lower.contains("user-agent: addonplayer/1") {
                    let _ = s.write_all(b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                    continue;
                }
                let _ = write!(s, "HTTP/1.1 206 Partial Content\r\nContent-Type: video/mp4\r\nContent-Range: bytes 0-{}/{}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len() - 1, body.len(), body.len());
                let _ = s.write_all(body);
            }
        });
        url
    }

    #[test]
    #[cfg(unix)]
    fn stored_counts_what_came_down() {
        let dir = std::env::temp_dir().join(format!("neptunus-stored-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("film.part");
        // a 100 MB film with three 1 MB chunks downloaded: its file is 100 MB long but sparse
        File::create(&path).unwrap().set_len(100 * CHUNK).unwrap();
        let done: String = (0..100).map(|i| if [0, 1, 50].contains(&i) { '1' } else { '0' }).collect();
        std::fs::write(path.with_extension("meta"), format!("{}\nvideo/mp4\n{done}", 100 * CHUNK)).unwrap();
        assert_eq!(path.metadata().unwrap().len(), 100 * CHUNK);
        assert_eq!(stored(&path), 3 * CHUNK);
        // no .meta: nothing to go by but the length
        std::fs::remove_file(path.with_extension("meta")).unwrap();
        assert_eq!(stored(&path), 100 * CHUNK);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn sends_the_sources_headers() {
        let dir = std::env::temp_dir().join(format!("neptunus-fill-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let url = picky_host(b"0123456789");
        // without them the host refuses: nothing to download, the player is sent to the source
        let bare = Fill::start(url.clone(), vec![], dir.join("bare.part"));
        assert_eq!(bare.head(), None);
        bare.close();
        // with them it answers; their User-Agent replaces ours, and a Range among them is ignored
        let headers = vec![
            ("Referer".into(), "https://addon.example/".into()),
            ("User-Agent".into(), "AddonPlayer/1".into()),
            ("Range".into(), "bytes=5-".into()),
        ];
        let f = Fill::start(url, headers, dir.join("ok.part"));
        assert_eq!(f.head().map(|(len, _)| len), Some(10));
        let file = f.file().unwrap();
        assert_eq!(f.read(&file, 0, 10).unwrap(), b"0123456789");
        f.close();
        let _ = std::fs::remove_dir_all(dir);
    }
}
