use crate::{Db, EXTS, Track, read_track};
use std::io::{self, Read, Seek, SeekFrom};
use s3::{Bucket, Region, creds::Credentials};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Emitter, Manager, State};

#[derive(Serialize, Deserialize, Default)]
pub struct S3Config {
    endpoint: String,
    region: String,
    bucket: String,
    prefix: String,
    access_key: String,
    secret_key: String,
    /// Set by `s3_disconnect`: the settings are kept for reconnecting, but the bucket is not in use.
    #[serde(default)]
    disconnected: bool,
}

fn config_path(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(app.path().app_data_dir().map_err(|e| e.to_string())?.join("s3.json"))
}

#[tauri::command]
pub fn s3_get_config(app: AppHandle) -> Result<S3Config, String> {
    Ok(std::fs::read_to_string(config_path(&app)?)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default())
}

#[tauri::command]
pub fn s3_save_config(app: AppHandle, config: S3Config) -> Result<(), String> {
    let json = serde_json::to_string_pretty(&config).map_err(|e| e.to_string())?;
    std::fs::write(config_path(&app)?, json).map_err(|e| e.to_string())
}

/// Stops using the bucket but keeps the settings (marked disconnected) and the songs' metadata, so
/// reconnecting is one click and reads nothing again; the app hides cloud songs meanwhile.
/// Nothing in the bucket is touched.
#[tauri::command]
pub fn s3_disconnect(app: AppHandle) -> Result<(), String> {
    let mut config = s3_get_config(app.clone())?;
    config.disconnected = true;
    s3_save_config(app, config)
}

fn open_bucket(c: &S3Config) -> Result<Box<Bucket>, String> {
    let region = if c.endpoint.is_empty() {
        c.region.parse::<Region>().map_err(|e| e.to_string())?
    } else {
        Region::Custom { region: c.region.clone(), endpoint: c.endpoint.clone() }
    };
    let creds = Credentials::new(Some(&c.access_key), Some(&c.secret_key), None, None, None)
        .map_err(|e| e.to_string())?;
    let bucket = Bucket::new(&c.bucket, region, creds).map_err(|e| e.to_string())?;
    // custom endpoints (MinIO, R2, ...) generally need path-style addressing
    Ok(if c.endpoint.is_empty() { bucket } else { bucket.with_path_style() })
}

const SCHEME: &str = "s3://";
/// Bytes fetched per range request while playing; the decoder reads on the audio thread, so fewer is better.
const CHUNK: u64 = 1 << 20;

/// Track path for an object: `s3://bucket/key`.
fn track_path(bucket: &str, key: &str) -> String {
    format!("{SCHEME}{bucket}/{key}")
}

fn wanted(key: &str) -> bool {
    Path::new(key)
        .extension()
        .and_then(|x| x.to_str())
        .map(str::to_lowercase)
        .is_some_and(|x| x == "lrc" || EXTS.contains(&x.as_str()))
}

/// One range request (inclusive end), retried once: a dropped connection mid-song shouldn't end it.
fn fetch_range(bucket: &Bucket, key: &str, start: u64, len: u64) -> io::Result<Vec<u8>> {
    let end = (start + CHUNK).min(len) - 1;
    let mut last = None;
    for _ in 0..2 {
        match bucket.get_object_range(key, start, Some(end)) {
            Ok(r) if r.status_code() < 300 => return Ok(r.bytes().to_vec()),
            Ok(r) => last = Some(io::Error::other(format!("{key}: HTTP {}", r.status_code()))),
            Err(e) => last = Some(io::Error::other(e)),
        }
    }
    Err(last.unwrap())
}

/// An S3 object as `Read + Seek`, fetched in chunks with range requests (nothing is written to disk).
/// With read-ahead on (playback), the next chunk is fetched on another thread while the current one plays,
/// so the audio thread doesn't wait for the network at every chunk boundary.
pub struct S3Reader {
    bucket: Box<Bucket>,
    key: String,
    len: u64,
    pos: u64,
    buf: Vec<u8>,
    buf_start: u64,
    readahead: bool,
    // in a Mutex only because the decoder needs the reader to be `Sync`
    ahead: Option<(u64, std::sync::Mutex<std::sync::mpsc::Receiver<io::Result<Vec<u8>>>>)>,
}

impl S3Reader {
    pub fn len(&self) -> u64 {
        self.len
    }

    pub fn with_readahead(mut self) -> Self {
        self.readahead = true;
        self
    }

    fn prefetch(&mut self, start: u64) {
        if start >= self.len {
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let (bucket, key, len) = (self.bucket.clone(), self.key.clone(), self.len);
        std::thread::spawn(move || {
            let _ = tx.send(fetch_range(&bucket, &key, start, len));
        });
        self.ahead = Some((start, std::sync::Mutex::new(rx)));
    }
}

impl Read for S3Reader {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if self.pos >= self.len || out.is_empty() {
            return Ok(0);
        }
        if self.pos < self.buf_start || self.pos >= self.buf_start + self.buf.len() as u64 {
            let pos = self.pos;
            self.buf = match self.ahead.take() {
                Some((start, rx)) if start == pos => rx.into_inner().unwrap_or_else(|e| e.into_inner()).recv().map_err(io::Error::other)??,
                _ => fetch_range(&self.bucket, &self.key, pos, self.len)?,
            };
            self.buf_start = pos;
            if self.readahead {
                self.prefetch(pos + self.buf.len() as u64);
            }
        }
        let off = (self.pos - self.buf_start) as usize;
        let n = out.len().min(self.buf.len() - off);
        out[..n].copy_from_slice(&self.buf[off..off + n]);
        self.pos += n as u64;
        Ok(n)
    }
}

impl Seek for S3Reader {
    fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
        let new = match to {
            SeekFrom::Start(n) => Some(n),
            SeekFrom::End(d) => self.len.checked_add_signed(d),
            SeekFrom::Current(d) => self.pos.checked_add_signed(d),
        };
        self.pos = new.ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "seek before start"))?;
        Ok(self.pos)
    }
}

fn open_object(bucket: Box<Bucket>, key: &str) -> Result<S3Reader, String> {
    let (head, _) = bucket.head_object(key).map_err(|e| format!("{key}: {e}"))?;
    let len = head.content_length.unwrap_or(0).max(0) as u64;
    Ok(S3Reader { bucket, key: key.to_owned(), len, pos: 0, buf: Vec::new(), buf_start: 0, readahead: false, ahead: None })
}

/// `Some` when `path` is an `s3://bucket/key` track; the credentials come from the saved config.
pub fn reader(path: &str) -> Option<Result<S3Reader, String>> {
    let (_, key) = path.strip_prefix(SCHEME)?.split_once('/')?;
    // the bucket in the path is the one the track was synced from; credentials are the saved ones
    let open = || -> Result<S3Reader, String> {
        let dir = crate::DATA_DIR.get().ok_or("no app data dir")?;
        let cfg: S3Config = std::fs::read_to_string(dir.join("s3.json"))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .ok_or("S3 belum dikonfigurasi")?;
        open_object(open_bucket(&cfg)?, key)
    };
    Some(open())
}

/// Text of the `.lrc` next to an S3 track. Outer `None` = not an S3 path; inner `None` = no sidecar.
pub fn sidecar(path: &str) -> Option<Option<String>> {
    let mut r = reader(path)?.ok()?;
    r.key = Path::new(&r.key).with_extension("lrc").to_string_lossy().into_owned();
    let got = r.bucket.get_object(&r.key).ok().filter(|o| o.status_code() < 300);
    Some(got.and_then(|o| String::from_utf8(o.bytes().to_vec()).ok()))
}

/// Indexes audio files under the configured prefix (tags are read with range requests, nothing is
/// downloaded) and returns the full library.
#[tauri::command]
pub async fn s3_sync(app: AppHandle, db: State<'_, Db>) -> Result<Vec<Track>, String> {
    let cfg = s3_get_config(app.clone())?;
    let bucket = open_bucket(&cfg)?;

    let objects: Vec<_> = bucket
        .list(cfg.prefix.clone(), None)
        .map_err(|e| e.to_string())?
        .into_iter()
        .flat_map(|r| r.contents)
        .filter(|o| wanted(&o.key) && !o.key.to_lowercase().ends_with(".lrc"))
        .collect();

    // Skip songs already in the library; read the rest 8 at a time and save after each batch so an
    // interrupted sync keeps its progress.
    let known: std::collections::HashSet<String> = db.all().map_err(|e| e.to_string())?.into_iter().map(|t| t.path).collect();
    let todo: Vec<String> = objects.iter().map(|o| track_path(&cfg.bucket, &o.key)).filter(|p| !known.contains(p)).collect();
    let total = todo.len();
    for (b, batch) in todo.chunks(8).enumerate() {
        let found: Vec<Track> = std::thread::scope(|s| {
            let handles: Vec<_> = batch.iter().map(|p| s.spawn(|| read_track(p))).collect();
            handles.into_iter().filter_map(|h| h.join().ok().flatten()).collect()
        });
        db.upsert(&found).map_err(|e| e.to_string())?;
        let _ = app.emit("s3-progress", ((b * 8 + batch.len()).min(total), total));
    }
    db.all().map_err(|e| e.to_string())
}

/// Saves an S3 track (and its `.lrc`, if any) to `~/Music/Neptunus`, adds the local file to the library
/// and returns the library. Emits `s3-download` (path, bytes done, bytes total) while copying.
#[tauri::command]
pub async fn s3_download(app: AppHandle, db: State<'_, Db>, path: String) -> Result<Vec<Track>, String> {
    use std::io::Write;
    let mut r = reader(&path).ok_or("bukan lagu S3")??;
    let dir = app.path().audio_dir().map_err(|e| e.to_string())?.join("Neptunus");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let name = Path::new(&r.key).file_name().ok_or("nama file tidak valid")?.to_owned();
    let dest = dir.join(name);
    let part = dest.with_extension("part");
    let mut f = std::fs::File::create(&part).map_err(|e| e.to_string())?;
    let (total, mut done) = (r.len(), 0u64);
    let mut buf = vec![0u8; 256 * 1024];
    loop {
        let n = r.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        f.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        done += n as u64;
        let _ = app.emit("s3-download", (&path, done, total));
    }
    drop(f);
    std::fs::rename(&part, &dest).map_err(|e| e.to_string())?;
    if let Some(Some(lrc)) = sidecar(&path) {
        let _ = std::fs::write(dest.with_extension("lrc"), lrc);
    }
    if let Some(t) = read_track(&dest.to_string_lossy()) {
        db.upsert(&[t]).map_err(|e| e.to_string())?;
    }
    db.all().map_err(|e| e.to_string())
}

// ---------- file manager ----------
// The bucket calls block, so each command runs on a blocking thread (`crate::blocking`).

#[derive(Serialize)]
pub struct Entry {
    key: String,
    name: String,
    dir: bool,
    size: u64,
    modified: String,
}

fn saved_bucket(app: &AppHandle) -> Result<(S3Config, Box<Bucket>), String> {
    let cfg = s3_get_config(app.clone())?;
    if cfg.bucket.is_empty() {
        return Err("S3 belum dikonfigurasi".into());
    }
    let b = open_bucket(&cfg)?;
    Ok((cfg, b))
}

/// Folders and files directly under `prefix` (a key ending in `/`, or empty for the bucket root).
#[tauri::command]
pub async fn s3_list(app: AppHandle, prefix: String) -> Result<Vec<Entry>, String> {
    crate::blocking(move || {
        let (_, bucket) = saved_bucket(&app)?;
        let pages = bucket.list(prefix.clone(), Some("/".into())).map_err(|e| e.to_string())?;
        let name = |k: &str| k[prefix.len()..].trim_end_matches('/').to_owned();
        let mut out: Vec<Entry> = Vec::new();
        for p in pages {
            for c in p.common_prefixes.unwrap_or_default() {
                out.push(Entry { name: name(&c.prefix), key: c.prefix, dir: true, size: 0, modified: String::new() });
            }
            // `prefix` itself shows up as an empty "folder marker" object
            for o in p.contents.into_iter().filter(|o| o.key != prefix) {
                out.push(Entry { name: name(&o.key), key: o.key, dir: false, size: o.size, modified: o.last_modified });
            }
        }
        out.sort_by(|a, b| b.dir.cmp(&a.dir).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
        Ok(out)
    })
    .await
}

/// Creates an empty folder marker (`prefix` ends in `/`).
#[tauri::command]
pub async fn s3_mkdir(app: AppHandle, prefix: String) -> Result<(), String> {
    crate::blocking(move || {
        let (_, bucket) = saved_bucket(&app)?;
        bucket.put_object(&prefix, b"").map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
}

fn content_type(key: &str) -> &'static str {
    match Path::new(key).extension().and_then(|x| x.to_str()).map(str::to_lowercase).as_deref() {
        Some("mp3") => "audio/mpeg",
        Some("flac") => "audio/flac",
        Some("m4a" | "aac") => "audio/mp4",
        Some("ogg" | "opus") => "audio/ogg",
        Some("wav") => "audio/wav",
        Some("lrc" | "txt") => "text/plain; charset=utf-8",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("png") => "image/png",
        _ => "application/octet-stream",
    }
}

/// Smallest part S3 accepts; also how often upload progress moves.
const PART: u64 = 5 * 1024 * 1024;
/// Parts (or small files) in flight at once. One request at a time over a fresh connection each
/// leaves most of the uplink idle between requests; a few at once fill it.
const LANES: usize = 4;

/// Upload/copy progress, emitted as `<kind>-upload`: per file 0 = waiting, 1 = sending, 2 = done;
/// `name` is the file that just moved, and `done` of `total` bytes (all files together) are through.
#[derive(Serialize, Clone)]
pub struct Progress<'a> {
    pub states: Vec<u8>,
    pub name: &'a str,
    pub done: u64,
    pub total: u64,
}

/// Total size of `files`, for progress.
pub fn total_size(files: &[String]) -> u64 {
    files.iter().filter_map(|f| std::fs::metadata(f).ok()).map(|m| m.len()).sum()
}

struct Up {
    name: String,
    key: String,
    path: String,
    /// multipart upload, for files bigger than one part
    upload_id: Option<String>,
    parts: std::sync::Mutex<Vec<Option<s3::serde_types::Part>>>,
    left: std::sync::atomic::AtomicUsize,
}

/// Uploads local files into `prefix`, `LANES` parts/files at a time; audio files join the library
/// right away (streamed from S3). Emits `s3-upload` progress. Returns the library.
#[tauri::command]
pub async fn s3_upload(app: AppHandle, prefix: String, files: Vec<String>) -> Result<Vec<Track>, String> {
    use std::sync::{Mutex, atomic::{AtomicU64, AtomicUsize, Ordering::SeqCst}};
    crate::blocking(move || {
        let (cfg, bucket) = saved_bucket(&app)?;
        let total = total_size(&files);
        let mut ups: Vec<Up> = Vec::new();
        let mut jobs = std::collections::VecDeque::new();
        let abort = |ups: &[Up], states: &[u8]| {
            for (u, _) in ups.iter().zip(states.iter().chain(std::iter::repeat(&0))).filter(|(_, s)| **s != 2) {
                if let Some(id) = &u.upload_id {
                    let _ = bucket.abort_upload(&u.key, id);
                }
            }
        };
        for (i, f) in files.iter().enumerate() {
            let name = Path::new(f).file_name().ok_or("nama file tidak valid")?.to_string_lossy().into_owned();
            let key = format!("{prefix}{name}");
            let len = std::fs::metadata(f).map_err(|e| format!("{name}: {e}"))?.len();
            let n = if len > PART { len.div_ceil(PART) as usize } else { 1 };
            let upload_id = if n > 1 {
                match bucket.initiate_multipart_upload(&key, content_type(&key)) {
                    Ok(r) => Some(r.upload_id),
                    Err(e) => {
                        abort(&ups, &[]);
                        return Err(format!("{name}: {e}"));
                    }
                }
            } else {
                None
            };
            jobs.extend((0..n).map(|p| (i, p)));
            ups.push(Up { name, key, path: f.clone(), upload_id, parts: Mutex::new((0..n).map(|_| None).collect()), left: AtomicUsize::new(n) });
        }

        let jobs = Mutex::new(jobs);
        let states = Mutex::new(vec![0u8; ups.len()]);
        let done = AtomicU64::new(0);
        let failed = Mutex::new(None::<String>);
        let emit = |name: &str| {
            let states = states.lock().unwrap().clone();
            let _ = app.emit("s3-upload", Progress { states, name, done: done.load(SeqCst), total });
        };
        // one part (or a whole small file): read it, send it, and finish the file after its last part
        let send = |i: usize, p: usize| -> Result<(), String> {
            let u = &ups[i];
            let ct = content_type(&u.key);
            let mut f = std::fs::File::open(&u.path).map_err(|e| e.to_string())?;
            f.seek(SeekFrom::Start(p as u64 * PART)).map_err(|e| e.to_string())?;
            let mut buf = Vec::with_capacity(PART as usize);
            f.take(PART).read_to_end(&mut buf).map_err(|e| e.to_string())?;
            match &u.upload_id {
                None => drop(bucket.put_object_with_content_type(&u.key, &buf, ct).map_err(|e| e.to_string())?),
                Some(id) => {
                    let part = bucket.put_multipart_chunk(&buf, &u.key, p as u32 + 1, id, ct).map_err(|e| e.to_string())?;
                    u.parts.lock().unwrap()[p] = Some(part);
                }
            }
            done.fetch_add(buf.len() as u64, SeqCst);
            if u.left.fetch_sub(1, SeqCst) == 1 {
                if let Some(id) = &u.upload_id {
                    let parts = u.parts.lock().unwrap().drain(..).flatten().collect();
                    bucket.complete_multipart_upload(&u.key, id, parts).map_err(|e| e.to_string())?;
                }
                states.lock().unwrap()[i] = 2;
            }
            Ok(())
        };

        if let Some(u) = ups.first() {
            emit(&u.name);
        }
        std::thread::scope(|s| {
            for _ in 0..LANES {
                s.spawn(|| loop {
                    if failed.lock().unwrap().is_some() {
                        break;
                    }
                    let Some((i, p)) = jobs.lock().unwrap().pop_front() else { break };
                    {
                        let mut st = states.lock().unwrap();
                        st[i] = st[i].max(1);
                    }
                    emit(&ups[i].name);
                    if let Err(e) = send(i, p) {
                        failed.lock().unwrap().get_or_insert(format!("{}: {e}", ups[i].name));
                        break;
                    }
                    emit(&ups[i].name);
                });
            }
        });
        if let Some(e) = failed.into_inner().unwrap() {
            abort(&ups, &states.into_inner().unwrap());
            return Err(e);
        }

        // read the new songs' tags (range requests), a few at a time
        let songs: Vec<String> = ups.iter().filter(|u| wanted(&u.key) && !u.key.to_lowercase().ends_with(".lrc")).map(|u| track_path(&cfg.bucket, &u.key)).collect();
        let mut added = Vec::new();
        for batch in songs.chunks(8) {
            added.extend(std::thread::scope(|s| {
                let hs: Vec<_> = batch.iter().map(|p| s.spawn(|| read_track(p))).collect();
                hs.into_iter().filter_map(|h| h.join().ok().flatten()).collect::<Vec<_>>()
            }));
        }
        let db = app.state::<Db>();
        db.upsert(&added).map_err(|e| e.to_string())?;
        db.all().map_err(|e| e.to_string())
    })
    .await
}

/// Deletes objects; a key ending in `/` deletes the whole folder. Songs among them leave the library.
#[tauri::command]
pub async fn s3_delete(app: AppHandle, keys: Vec<String>) -> Result<Vec<Track>, String> {
    crate::blocking(move || {
        let (cfg, bucket) = saved_bucket(&app)?;
        let mut all = Vec::new();
        for k in keys {
            if k.ends_with('/') {
                let pages = bucket.list(k.clone(), None).map_err(|e| e.to_string())?;
                all.extend(pages.into_iter().flat_map(|p| p.contents).map(|o| o.key));
            }
            all.push(k);
        }
        all.sort();
        all.dedup();
        for k in &all {
            bucket.delete_object(k).map_err(|e| format!("{k}: {e}"))?;
        }
        let db = app.state::<Db>();
        db.remove(&all.iter().map(|k| track_path(&cfg.bucket, k)).collect::<Vec<_>>()).map_err(|e| e.to_string())?;
        db.all().map_err(|e| e.to_string())
    })
    .await
}
