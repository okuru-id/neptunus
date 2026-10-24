//! Posters and backdrops of the video catalogs, kept on disk so the catalogs show them again without
//! the network (offline, a slow addon host). The loopback server hands them to the webview: a poster
//! comes from the disk when it is there, else it is fetched once and saved. The folder is held under
//! `MAX` bytes, the least recently shown going first.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, SystemTime};

static DIR: OnceLock<PathBuf> = OnceLock::new();
/// The folder stops growing past this; pruning takes it back down to three quarters of it.
const MAX: u64 = 200 << 20;
/// Anything bigger isn't a poster.
const MAX_FILE: u64 = 8 << 20;

pub fn set_dir(dir: PathBuf) {
    if DIR.set(dir.clone()).is_ok() {
        let _ = std::fs::create_dir_all(&dir);
        std::thread::spawn(move || prune(&dir));
    }
}

/// The image at `url` and its type: from the disk, else fetched and saved there. `title` names what the
/// picture is of ("tt0111161:poster"): extensions listing the same film each give their own URL for
/// its poster, and under the title they share one copy, whichever of them saved it first.
pub fn get(url: &str, title: Option<&str>) -> Option<(Vec<u8>, &'static str)> {
    if !url.starts_with("https://") && !url.starts_with("http://") {
        return None;
    }
    let path = DIR.get()?.join(super::cache_key(title.filter(|t| !t.is_empty()).unwrap_or(url)));
    if let Ok(bytes) = std::fs::read(&path) {
        // its mtime says when it was last shown, for pruning
        let _ = std::fs::File::options().append(true).open(&path).and_then(|f| f.set_modified(SystemTime::now()));
        let m = mime(&bytes)?;
        return Some((bytes, m));
    }
    let res = attohttpc::get(url)
        .header("User-Agent", "Mozilla/5.0")
        .connect_timeout(Duration::from_secs(10))
        .read_timeout(Duration::from_secs(20))
        .send()
        .ok()
        .filter(|r| r.is_success())?;
    let mut bytes = Vec::new();
    std::io::Read::read_to_end(&mut std::io::Read::take(res, MAX_FILE + 1), &mut bytes).ok()?;
    let m = mime(&bytes).filter(|_| bytes.len() as u64 <= MAX_FILE)?;
    // written beside, then renamed: a poster shown while it was being saved is never half a file
    let tmp = path.with_extension("tmp");
    if std::fs::write(&tmp, &bytes).is_ok() && std::fs::rename(&tmp, &path).is_ok() && now_and_then() {
        if let Some(dir) = DIR.get() {
            let dir = dir.clone();
            std::thread::spawn(move || prune(&dir));
        }
    }
    Some((bytes, m))
}

/// Image types by their first bytes; anything else (an error page) isn't kept.
fn mime(b: &[u8]) -> Option<&'static str> {
    Some(match b {
        [0xff, 0xd8, 0xff, ..] => "image/jpeg",
        [0x89, b'P', b'N', b'G', ..] => "image/png",
        [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'E', b'B', b'P', ..] => "image/webp",
        [b'G', b'I', b'F', b'8', ..] => "image/gif",
        [_, _, _, _, b'f', b't', b'y', b'p', b'a', b'v', b'i', b'f', ..] => "image/avif",
        _ => return None,
    })
}

/// Prunes now and then rather than after every poster saved.
fn now_and_then() -> bool {
    use std::sync::atomic::{AtomicU32, Ordering};
    static N: AtomicU32 = AtomicU32::new(0);
    N.fetch_add(1, Ordering::Relaxed) % 50 == 49
}

fn prune(dir: &Path) {
    let mut all: Vec<_> = std::fs::read_dir(dir).into_iter().flatten().flatten()
        .filter_map(|e| { let m = e.metadata().ok()?; Some((m.modified().ok()?, m.len(), e.path())) })
        .collect();
    let mut total: u64 = all.iter().map(|a| a.1).sum();
    if total <= MAX {
        return;
    }
    all.sort_by_key(|a| a.0);
    for (_, size, path) in all {
        if total <= MAX / 4 * 3 {
            break;
        }
        if std::fs::remove_file(&path).is_ok() {
            total -= size;
        }
    }
}

pub fn size() -> u64 {
    DIR.get().map_or(0, |d| std::fs::read_dir(d).into_iter().flatten().flatten().filter_map(|e| e.metadata().ok()).map(|m| m.len()).sum())
}

pub fn clear() {
    if let Some(d) = DIR.get() {
        for e in std::fs::read_dir(d).into_iter().flatten().flatten() {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    /// Serves `body` to `n` requests, then stops (the host going offline).
    fn host(body: &'static [u8], n: usize) -> String {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/p.jpg", l.local_addr().unwrap());
        std::thread::spawn(move || {
            for mut s in l.incoming().flatten().take(n) {
                let _ = s.read(&mut [0; 1024]);
                let _ = write!(s, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
                let _ = s.write_all(body);
            }
        });
        url
    }

    #[test]
    fn kept_on_disk() {
        let dir = std::env::temp_dir().join(format!("neptunus-posters-{}", std::process::id()));
        set_dir(dir.clone());
        const JPEG: &[u8] = &[0xff, 0xd8, 0xff, 0xe0, 1, 2, 3, 4];
        let url = host(JPEG, 1);
        assert_eq!(get(&url, None), Some((JPEG.to_vec(), "image/jpeg")));
        // the host is gone now: the poster still comes, from the disk
        assert_eq!(get(&url, None), Some((JPEG.to_vec(), "image/jpeg")));
        // an error page isn't an image and isn't kept
        let page = host(b"<html>not found</html>", 2);
        assert_eq!(get(&page, None), None);
        assert!(!dir.join(crate::video::cache_key(&page)).exists());
        assert_eq!(get("file:///etc/passwd", None), None);
        // the same film from a second extension, at another URL: one copy, nothing fetched again
        let first = host(JPEG, 1);
        assert_eq!(get(&first, Some("tt1:poster")), Some((JPEG.to_vec(), "image/jpeg")));
        let files = std::fs::read_dir(&dir).unwrap().count();
        assert_eq!(get("http://127.0.0.1:9/elsewhere.jpg", Some("tt1:poster")), Some((JPEG.to_vec(), "image/jpeg")));
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), files);
        let _ = std::fs::remove_dir_all(dir);
    }
}
