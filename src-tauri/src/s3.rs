use crate::{Db, EXTS, Track, read_track};
use s3::{Bucket, Region, creds::Credentials};
use serde::{Deserialize, Serialize};
use std::path::{Component, Path, PathBuf};
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

/// Cache location for an object key; rejects keys that would escape the cache directory.
fn local_path(cache: &Path, key: &str) -> Option<PathBuf> {
    let rel = Path::new(key);
    rel.components().all(|c| matches!(c, Component::Normal(_))).then(|| cache.join(rel))
}

fn wanted(key: &str) -> bool {
    Path::new(key)
        .extension()
        .and_then(|x| x.to_str())
        .map(str::to_lowercase)
        .is_some_and(|x| x == "lrc" || EXTS.contains(&x.as_str()))
}

/// Mirrors audio files (and `.lrc` sidecars) under the configured prefix into the local cache,
/// indexes them, and returns the full library. Files already cached with the same size are skipped.
#[tauri::command]
pub fn s3_sync(app: AppHandle, db: State<Db>) -> Result<Vec<Track>, String> {
    let cfg = s3_get_config(app.clone())?;
    let bucket = open_bucket(&cfg)?;
    let cache = app.path().app_cache_dir().map_err(|e| e.to_string())?.join("s3").join(&cfg.bucket);

    let objects: Vec<_> = bucket
        .list(cfg.prefix.clone(), None)
        .map_err(|e| e.to_string())?
        .into_iter()
        .flat_map(|r| r.contents)
        .filter(|o| wanted(&o.key))
        .collect();

    let mut indexed = Vec::new();
    for (i, o) in objects.iter().enumerate() {
        let _ = app.emit("s3-progress", (i + 1, objects.len()));
        let Some(dest) = local_path(&cache, &o.key) else { continue };
        if std::fs::metadata(&dest).ok().map(|m| m.len()) != Some(o.size) {
            std::fs::create_dir_all(dest.parent().unwrap()).map_err(|e| e.to_string())?;
            let part = dest.with_extension("part");
            let mut f = std::fs::File::create(&part).map_err(|e| e.to_string())?;
            bucket.get_object_to_writer(&o.key, &mut f).map_err(|e| format!("{}: {e}", o.key))?;
            std::fs::rename(&part, &dest).map_err(|e| e.to_string())?;
        }
        if !o.key.to_lowercase().ends_with(".lrc") {
            indexed.extend(read_track(&dest));
        }
    }
    db.upsert(&indexed).map_err(|e| e.to_string())?;
    db.all().map_err(|e| e.to_string())
}
