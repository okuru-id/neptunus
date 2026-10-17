use lofty::{file::TaggedFileExt, probe::Probe, tag::ItemKey};
use std::{hash::{Hash, Hasher}, path::Path, time::Duration};
use tauri::{AppHandle, Manager};

const UA: &str = concat!("PixelPlay Desktop/", env!("CARGO_PKG_VERSION"));

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct Lrclib {
    synced_lyrics: Option<String>,
    plain_lyrics: Option<String>,
}

impl Lrclib {
    fn text(self) -> Option<String> {
        self.synced_lyrics.or(self.plain_lyrics).filter(|s| !s.trim().is_empty())
    }
}

/// Sidecar `.lrc`, then the embedded lyrics tag.
fn local(path: &Path) -> Option<String> {
    if let Ok(txt) = std::fs::read_to_string(path.with_extension("lrc")) {
        return Some(txt);
    }
    let file = Probe::open(path).ok()?.read().ok()?;
    let tag = file.primary_tag().or_else(|| file.first_tag())?;
    tag.get_string(ItemKey::Lyrics).map(str::to_owned)
}

/// https://lrclib.net/docs: exact match first, then a looser search (album or duration may differ).
/// `Err` = network failure, which is not cached so it is retried next time.
fn lrclib(title: &str, artist: &str, album: &str, duration: f64) -> Result<Option<String>, attohttpc::Error> {
    let get = |url: &str| attohttpc::get(url).header("User-Agent", UA).timeout(Duration::from_secs(10));
    let dur = duration.round().to_string();
    let exact = get("https://lrclib.net/api/get")
        .params([("track_name", title), ("artist_name", artist), ("album_name", album), ("duration", &dur)])
        .send()?;
    if exact.is_success() {
        return Ok(exact.json::<Lrclib>()?.text());
    }
    let mut found: Vec<Lrclib> = get("https://lrclib.net/api/search")
        .params([("track_name", title), ("artist_name", artist)])
        .send()?
        .json()?;
    // prefer a synced result
    let i = found.iter().position(|l| l.synced_lyrics.is_some()).unwrap_or(0);
    Ok((i < found.len()).then(|| found.swap_remove(i)).and_then(Lrclib::text))
}

/// Local lyrics, else LRCLIB (cached in the app data dir; an empty cache file means "not found").
#[tauri::command]
pub async fn get_lyrics(app: AppHandle, path: String, title: String, artist: String, album: String, duration: f64) -> Option<String> {
    tauri::async_runtime::spawn_blocking(move || {
        if let Some(txt) = local(Path::new(&path)) {
            return Some(txt);
        }
        if title.is_empty() || artist.is_empty() {
            return None;
        }
        let mut h = std::collections::hash_map::DefaultHasher::new();
        (&title, &artist, &album).hash(&mut h);
        let cache = app.path().app_cache_dir().ok().map(|d| d.join("lyrics").join(format!("{:016x}.lrc", h.finish())));
        if let Some(txt) = cache.as_ref().and_then(|c| std::fs::read_to_string(c).ok()) {
            return (!txt.is_empty()).then_some(txt);
        }
        let txt = lrclib(&title, &artist, &album, duration).ok()?;
        if let Some(c) = &cache {
            let _ = std::fs::create_dir_all(c.parent()?);
            let _ = std::fs::write(c, txt.as_deref().unwrap_or(""));
        }
        txt
    })
    .await
    .ok()
    .flatten()
}

