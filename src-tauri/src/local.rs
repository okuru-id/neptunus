//! File manager commands for local library folders; same shape as the `s3_*` ones. Directory keys end in `/`.
use crate::{Db, EXTS, Track, read_track};
use serde::Serialize;
use std::path::Path;
use crate::s3::Progress;
use tauri::{AppHandle, Emitter, Manager};

#[derive(Serialize)]
pub struct Entry {
    key: String,
    name: String,
    dir: bool,
    size: u64,
    /// milliseconds since the epoch
    modified: String,
}

type R<T> = Result<T, String>;
fn err(e: impl ToString) -> String {
    e.to_string()
}

fn is_song(p: &Path) -> bool {
    p.extension().and_then(|x| x.to_str()).is_some_and(|x| EXTS.contains(&x.to_lowercase().as_str()))
}

#[tauri::command]
pub async fn local_list(prefix: String) -> R<Vec<Entry>> {
    crate::blocking(move || {
        let mut out: Vec<Entry> = Vec::new();
        for d in std::fs::read_dir(&prefix).map_err(err)?.filter_map(Result::ok) {
            let name = d.file_name().to_string_lossy().into_owned();
            let Ok(meta) = d.metadata() else { continue };
            if name.starts_with('.') {
                continue;
            }
            let modified = meta.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok());
            let dir = meta.is_dir();
            out.push(Entry {
                key: format!("{prefix}{name}{}", if dir { "/" } else { "" }),
                name,
                dir,
                size: if dir { 0 } else { meta.len() },
                modified: modified.map(|m| m.as_millis().to_string()).unwrap_or_default(),
            });
        }
        out.sort_by(|a, b| b.dir.cmp(&a.dir).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
        Ok(out)
    })
    .await
}

#[tauri::command]
pub async fn local_mkdir(prefix: String) -> R<()> {
    crate::blocking(move || std::fs::create_dir(&prefix).map_err(err)).await
}

/// Copies files into `prefix`; songs join the library. Emits `local-upload` progress.
#[tauri::command]
pub async fn local_upload(app: AppHandle, prefix: String, files: Vec<String>) -> R<Vec<Track>> {
    crate::blocking(move || {
        use std::io::{Read, Write};
        let (total, mut done) = (crate::s3::total_size(&files), 0u64);
        let mut states = vec![0u8; files.len()];
        let mut added = Vec::new();
        for (index, f) in files.iter().enumerate() {
            let name = Path::new(f).file_name().ok_or("nama file tidak valid")?.to_string_lossy();
            states[index] = 1;
            let st = states.clone();
            let emit = |done| app.emit("local-upload", Progress { states: st.clone(), name: &name, done, total });
            let _ = emit(done);
            let dest = Path::new(&prefix).join(&*name);
            if dest.exists() {
                return Err(format!("{name}: sudah ada di folder ini"));
            }
            let mut copy = || -> std::io::Result<()> {
                let (mut src, mut out) = (std::fs::File::open(f)?, std::fs::File::create(&dest)?);
                let mut buf = vec![0u8; 1 << 20];
                loop {
                    let n = src.read(&mut buf)?;
                    if n == 0 {
                        return Ok(());
                    }
                    out.write_all(&buf[..n])?;
                    done += n as u64;
                    let _ = emit(done);
                }
            };
            if let Err(e) = copy() {
                let _ = std::fs::remove_file(&dest);
                return Err(format!("{name}: {e}"));
            }
            states[index] = 2;
            if is_song(&dest) {
                added.extend(read_track(&dest.to_string_lossy()));
            }
        }
        let db = app.state::<Db>();
        db.upsert(&added).map_err(err)?;
        db.all().map_err(err)
    })
    .await
}

/// Moves files and folders to the system trash; songs among them leave the library.
#[tauri::command]
pub async fn local_delete(app: AppHandle, keys: Vec<String>) -> R<Vec<Track>> {
    crate::blocking(move || {
        let db = app.state::<Db>();
        for k in &keys {
            crate::trash(k.trim_end_matches('/')).map_err(|e| format!("{k}: {e}"))?;
            if k.ends_with('/') {
                db.remove_under(k).map_err(err)?;
            }
        }
        db.remove(&keys).map_err(err)?;
        db.all().map_err(err)
    })
    .await
}
