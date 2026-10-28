//! Imports a public Spotify playlist, album, artist (top tracks) or track: reads the track list from the embed page (no login) and fills a
//! new playlist with the songs already in the library; the rest are reported back.
use crate::{
    Db,
    db::{Pending, Track},
};
use serde::Serialize;
use serde_json::{Value, json};
use std::{collections::HashMap, time::Duration};
use tauri::{AppHandle, Manager, State};

#[derive(Serialize)]
pub struct Import {
    id: i64,
    name: String,
    added: usize,
    pending: usize,
}

/// `("playlist", id)` from an open.spotify.com link or a `spotify:kind:id` URI.
fn parse(url: &str) -> Option<(&'static str, String)> {
    let url = url.trim().split('?').next()?;
    let tail = url.strip_prefix("spotify:").map(|u| u.replace(':', "/"));
    let tail = tail.as_deref().unwrap_or(url);
    let mut parts = tail.rsplit('/');
    let id = parts.next()?;
    let kind = parts.next()?;
    let kind = ["playlist", "album", "artist", "track"].into_iter().find(|k| *k == kind)?;
    (!id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric())).then_some((kind, id.to_owned()))
}

/// Lowercase letters/digits only, dropping "(feat. x)", "[live]" and " - Remastered" tails.
pub(crate) fn norm(s: &str) -> String {
    let dash = s.find(" - ").filter(|&i| s[i..].to_lowercase().contains("remaster"));
    let cut = s.find(['(', '[']).into_iter().chain(dash).min().unwrap_or(s.len());
    s[..cut].chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

type Wanted = (String, String, String, u64, String); // title, artists, album, seconds, uri

fn fetch(kind: &str, id: &str) -> Result<(String, Vec<Wanted>), String> {
    let html = attohttpc::get(format!("https://open.spotify.com/embed/{kind}/{id}"))
        .header("User-Agent", "Mozilla/5.0")
        .timeout(Duration::from_secs(15))
        .send()
        .and_then(|r| r.error_for_status())
        .and_then(|r| r.text())
        .map_err(|e| format!("Spotify tidak bisa dihubungi: {e}"))?;
    let start = html.find("__NEXT_DATA__").and_then(|i| html[i..].find('>').map(|j| i + j + 1));
    let json = start.and_then(|s| html[s..].find("</script>").map(|e| &html[s..s + e]));
    let data: Value = json.and_then(|j| serde_json::from_str(j).ok()).ok_or("Halaman playlist tidak terbaca")?;
    let entity = &data["props"]["pageProps"]["state"]["data"]["entity"];
    let list = entity["trackList"].as_array().ok_or("Tidak ditemukan atau tidak publik")?;
    let name = entity["name"].as_str().unwrap_or("Spotify").to_owned();
    // an album page names its album; playlists and artists leave it blank
    let album = if entity["type"] == "album" { name.clone() } else { String::new() };
    let tracks = list
        .iter()
        .filter_map(|t| {
            let artists = t["subtitle"].as_str().unwrap_or("").replace('\u{a0}', " ");
            let uri = t["uri"].as_str().unwrap_or("").to_owned();
            Some((t["title"].as_str()?.to_owned(), artists, album.clone(), t["duration"].as_u64().unwrap_or(0) / 1000, uri))
        })
        .collect();
    Ok((name, tracks))
}

/// Library lookup by title and main artist: `find(title, artists)` gives the path of a matching song.
fn library_matcher(db: &Db) -> Result<impl Fn(&str, &str) -> Option<String>, String> {
    let mut by_title: HashMap<String, Vec<(String, String)>> = HashMap::new();
    for t in db.all().map_err(|e| e.to_string())? {
        by_title.entry(norm(&t.title)).or_default().push((norm(&t.artist), t.path));
    }
    Ok(move |title: &str, artists: &str| {
        let first = norm(artists.split(',').next().unwrap_or(""));
        by_title.get(&norm(title))?.iter()
            .find(|(a, _)| a.is_empty() || first.is_empty() || a.contains(&first) || first.contains(a.as_str()))
            .map(|(_, path)| path.clone())
    })
}

#[tauri::command]
pub async fn spotify_import(db: State<'_, Db>, url: String) -> Result<Import, String> {
    let (kind, id) = parse(&url).ok_or("Link Spotify tidak dikenali")?;
    let (name, wanted) = crate::blocking(move || fetch(kind, &id)).await?;
    fill(&db, name, wanted.into_iter().map(|(t, a, al, s, u)| (t, a, al, s, u, String::new())).collect())
}

/// A new playlist `name` from songs found elsewhere (title, artists, album, seconds, uri): the ones in
/// the library are added, the rest wait as pending songs to download or stream.
#[tauri::command]
pub fn playlist_from(db: State<Db>, name: String, tracks: Vec<(String, String, String, u64, String, String)>) -> Result<Import, String> {
    fill(&db, name, tracks)
}

/// songs as (title, artists, album, seconds, uri, cover URL)
fn fill(db: &Db, name: String, wanted: Vec<(String, String, String, u64, String, String)>) -> Result<Import, String> {
    let find = library_matcher(db)?;
    let pl = db.create_playlist(&name).map_err(|e| e.to_string())?;
    let (mut added, mut pending) = (0, 0);
    for (pos, (title, artists, album, secs, uri, cover)) in wanted.into_iter().enumerate() {
        let pos = pos as i64 + 1;
        match find(&title, &artists) {
            Some(path) => {
                db.add_to_playlist_at(pl, &path, pos).map_err(|e| e.to_string())?;
                added += 1;
            }
            None => {
                let p = Pending { id: 0, title, artist: artists, album, duration_secs: secs, uri, cover };
                db.add_pending(pl, &p, pos).map_err(|e| e.to_string())?;
                pending += 1;
            }
        }
    }
    Ok(Import { id: pl, name, added, pending })
}

/// Cover art URL (640px) of a `spotify:track:` uri, from Spotify's public oEmbed endpoint.
fn cover(uri: &str) -> Option<String> {
    if uri.is_empty() {
        return None;
    }
    let res: Value = attohttpc::get("https://open.spotify.com/oembed")
        .param("url", uri)
        .header("User-Agent", "Mozilla/5.0")
        .timeout(Duration::from_secs(10))
        .send()
        .ok()?
        .json()
        .ok()?;
    // the 300px image id; the same image at 640px only differs in this size prefix
    Some(res["thumbnail_url"].as_str()?.replace("ab67616d00001e02", "ab67616d0000b273"))
}

/// (Deezer track id, ISRC) of a song, from Deezer's public API; many download extensions match on these.
fn deezer(title: &str, artist: &str) -> Option<(String, String, String)> {
    let get = |url: &str| attohttpc::get(url).header("User-Agent", "Mozilla/5.0").timeout(Duration::from_secs(10));
    let first = norm(artist.split(',').next().unwrap_or(""));
    let res: Value = get("https://api.deezer.com/search").param("q", format!("{artist} {title}")).send().ok()?.json().ok()?;
    let hit = res["data"].as_array()?.iter().find(|t| {
        norm(t["title"].as_str().unwrap_or("")) == norm(title) && norm(t["artist"]["name"].as_str().unwrap_or("")).contains(&first)
    })?;
    let id = hit["id"].as_u64()?;
    let full: Value = get(&format!("https://api.deezer.com/track/{id}")).send().ok()?.json().ok()?;
    Some((id.to_string(), full["isrc"].as_str().unwrap_or("").to_owned(), hit["album"]["title"].as_str().unwrap_or("").to_owned()))
}

#[tauri::command]
/// The playlist's pending songs. Ones that reached the library some other way (the catalog, a folder
/// scan) become normal playlist tracks first.
pub fn playlist_pending(db: State<Db>, id: i64) -> Result<Vec<Pending>, String> {
    let pending = db.pending(id).map_err(|e| e.to_string())?;
    if pending.is_empty() {
        return Ok(pending);
    }
    let find = library_matcher(&db)?;
    let mut left = vec![];
    for p in pending {
        match find(&p.title, &p.artist) {
            Some(path) => db.resolve_pending(p.id, &path).map_err(|e| e.to_string())?,
            None => left.push(p),
        }
    }
    Ok(left)
}

#[tauri::command]
pub fn pending_remove(db: State<Db>, id: i64) -> Result<(), String> {
    db.remove_pending(id).map_err(|e| e.to_string())
}

/// Downloads a pending playlist entry with download extension `ext`; the song then replaces the entry.
/// Progress events (`ext-download`) are keyed `pending:<id>`.
#[tauri::command]
/// `track`: the song as a metadata extension describes it (Spotify Web's `getTrack`), like a catalog
/// download; without it the entry's own title, artist and Spotify id are used.
pub async fn pending_download(app: AppHandle, id: i64, ext: String, quality: Option<String>, track: Option<Value>) -> Result<Track, String> {
    crate::blocking(move || {
        let db = app.state::<Db>();
        let p = db.pending_one(id).map_err(|e| e.to_string())?.ok_or("Entri sudah tidak ada")?;
        // the extension finds the song itself (`checkAvailability`): by Spotify id (SongLink), ISRC or title
        let spotify_id = p.uri.rsplit(':').next().unwrap_or("");
        let mut track = track.filter(|t| t.is_object()).unwrap_or_else(|| json!({
            "id": spotify_id, "name": p.title, "artists": p.artist, "album_name": p.album,
            "duration_ms": p.duration_secs * 1000, "provider_id": "spotify",
        }));
        // fill what the source left out: ISRC and Deezer id (matching), album (tags), cover art (tags)
        let blank = |t: &Value, k: &str| t[k].as_str().is_none_or(str::is_empty);
        if blank(&track, "isrc") || blank(&track, "deezer_id") || blank(&track, "album_name") {
            let (deezer_id, isrc, album) = deezer(&p.title, &p.artist).unwrap_or_default();
            if blank(&track, "album_name") { track["album_name"] = album.into(); }
            if blank(&track, "isrc") { track["isrc"] = isrc.into(); }
            if blank(&track, "deezer_id") { track["deezer_id"] = deezer_id.into(); }
        }
        if ["cover_url", "images", "image_url"].iter().all(|k| blank(&track, k)) {
            track["cover_url"] = if p.cover.is_empty() { cover(&p.uri).into() } else { p.cover.clone().into() };
        }
        let t = crate::ext::fetch(&app, &ext, &track, quality, &format!("pending:{id}"))?;
        db.resolve_pending(id, &t.path).map_err(|e| e.to_string())?;
        Ok(t)
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_links() {
        let u = "https://open.spotify.com/artist/7Gy1PxqrgsiqWF6JNYPHeB?si=E7Q1pO-hTmCprAZ1zxl0lA";
        assert_eq!(parse(u), Some(("artist", "7Gy1PxqrgsiqWF6JNYPHeB".into())));
        assert_eq!(parse("spotify:playlist:37i9dQZF1DXcBWIGoYBM5M"), Some(("playlist", "37i9dQZF1DXcBWIGoYBM5M".into())));
        assert_eq!(parse("https://open.spotify.com/intl-id/album/abc123"), Some(("album", "abc123".into())));
        assert_eq!(parse("https://example.com/x"), None);
        assert_eq!(norm("Berita Kehilangan (feat. X) - Remastered"), "beritakehilangan");
        assert_eq!(norm("Song - 2011 Remaster"), "song");
        assert_ne!(norm("Bunga Maaf - Instrumental"), norm("Bunga Maaf"));
    }
}
