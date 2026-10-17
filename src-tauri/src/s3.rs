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

/// An S3 object as `Read + Seek`, fetched in chunks with range requests (nothing is written to disk).
pub struct S3Reader {
    bucket: Box<Bucket>,
    key: String,
    len: u64,
    pos: u64,
    buf: Vec<u8>,
    buf_start: u64,
}

impl S3Reader {
    pub fn len(&self) -> u64 {
        self.len
    }
}

impl Read for S3Reader {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if self.pos >= self.len || out.is_empty() {
            return Ok(0);
        }
        if self.pos < self.buf_start || self.pos >= self.buf_start + self.buf.len() as u64 {
            let end = (self.pos + CHUNK).min(self.len) - 1;
            let r = self.bucket.get_object_range(&self.key, self.pos, Some(end)).map_err(io::Error::other)?;
            if r.status_code() >= 300 {
                return Err(io::Error::other(format!("{}: HTTP {}", self.key, r.status_code())));
            }
            self.buf = r.bytes().to_vec();
            self.buf_start = self.pos;
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
    Ok(S3Reader { bucket, key: key.to_owned(), len, pos: 0, buf: Vec::new(), buf_start: 0 })
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
