use rusqlite::{Connection, params};
use serde::Serialize;
use std::sync::Mutex;

#[derive(Serialize, Clone)]
pub struct Track {
    pub path: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration_secs: u64,
}

/// A playlist entry whose song is not in the library yet (imported from Spotify); downloading it
/// turns it into a normal playlist track.
#[derive(Serialize)]
pub struct Pending {
    pub id: i64,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration_secs: u64,
    /// `spotify:track:<id>`: finds the song's full metadata and cover art when downloading
    pub uri: String,
    /// cover art URL from the catalog the song was saved from ("" when none)
    pub cover: String,
}

#[derive(Serialize)]
pub struct Playlist {
    id: i64,
    name: String,
}

pub struct Db(Mutex<Connection>);

const COLS: &str = "t.path, t.title, t.artist, t.album, t.duration_secs";

fn row(r: &rusqlite::Row) -> rusqlite::Result<Track> {
    Ok(Track { path: r.get(0)?, title: r.get(1)?, artist: r.get(2)?, album: r.get(3)?, duration_secs: r.get::<_, i64>(4)? as u64 })
}

impl Db {
    pub fn open(path: &std::path::Path) -> rusqlite::Result<Self> {
        let c = Connection::open(path)?;
        c.execute_batch(
            "PRAGMA foreign_keys = ON;
             CREATE TABLE IF NOT EXISTS tracks (
               path TEXT PRIMARY KEY, title TEXT NOT NULL, artist TEXT NOT NULL,
               album TEXT NOT NULL, duration_secs INTEGER NOT NULL);
             CREATE TABLE IF NOT EXISTS folders (path TEXT PRIMARY KEY);
             CREATE TABLE IF NOT EXISTS playlists (
               id INTEGER PRIMARY KEY, name TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS playlist_tracks (
               playlist_id INTEGER NOT NULL REFERENCES playlists(id) ON DELETE CASCADE,
               path TEXT NOT NULL REFERENCES tracks(path) ON DELETE CASCADE,
               pos INTEGER NOT NULL, PRIMARY KEY (playlist_id, path));
             CREATE TABLE IF NOT EXISTS playlist_pending (
               id INTEGER PRIMARY KEY, playlist_id INTEGER NOT NULL REFERENCES playlists(id) ON DELETE CASCADE,
               title TEXT NOT NULL, artist TEXT NOT NULL, album TEXT NOT NULL,
               duration_secs INTEGER NOT NULL, pos INTEGER NOT NULL, uri TEXT NOT NULL DEFAULT '');
             CREATE TABLE IF NOT EXISTS downloads (
               path TEXT PRIMARY KEY REFERENCES tracks(path) ON DELETE CASCADE,
               ext TEXT NOT NULL, track TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS subtitles (url TEXT PRIMARY KEY, vtt TEXT NOT NULL, used INTEGER NOT NULL);
             CREATE TABLE IF NOT EXISTS ext_cache (key TEXT PRIMARY KEY, value TEXT NOT NULL, saved INTEGER NOT NULL);",
        )?;
        // tables made before the Spotify uri column (fails harmlessly once it exists)
        let _ = c.execute("ALTER TABLE playlist_pending ADD COLUMN uri TEXT NOT NULL DEFAULT ''", []);
        let _ = c.execute("ALTER TABLE playlist_pending ADD COLUMN cover TEXT NOT NULL DEFAULT ''", []);
        Ok(Self(Mutex::new(c)))
    }

    pub fn upsert(&self, tracks: &[Track]) -> rusqlite::Result<()> {
        let mut c = self.0.lock().unwrap();
        let tx = c.transaction()?;
        for t in tracks {
            tx.execute(
                "INSERT INTO tracks VALUES (?1,?2,?3,?4,?5) ON CONFLICT(path) DO UPDATE SET
                 title=?2, artist=?3, album=?4, duration_secs=?5",
                params![t.path, t.title, t.artist, t.album, t.duration_secs as i64],
            )?;
        }
        tx.commit()
    }

    pub fn all(&self) -> rusqlite::Result<Vec<Track>> {
        let c = self.0.lock().unwrap();
        let mut s = c.prepare(&format!("SELECT {COLS} FROM tracks t ORDER BY t.artist, t.album, t.title"))?;
        s.query_map([], row)?.collect()
    }

    pub fn playlists(&self) -> rusqlite::Result<Vec<Playlist>> {
        let c = self.0.lock().unwrap();
        let mut s = c.prepare("SELECT id, name FROM playlists ORDER BY name")?;
        s.query_map([], |r| Ok(Playlist { id: r.get(0)?, name: r.get(1)? }))?.collect()
    }

    pub fn create_playlist(&self, name: &str) -> rusqlite::Result<i64> {
        let c = self.0.lock().unwrap();
        c.execute("INSERT INTO playlists (name) VALUES (?1)", [name])?;
        Ok(c.last_insert_rowid())
    }

    pub fn delete_playlist(&self, id: i64) -> rusqlite::Result<()> {
        self.0.lock().unwrap().execute("DELETE FROM playlists WHERE id=?1", [id]).map(|_| ())
    }

    pub fn playlist_tracks(&self, id: i64) -> rusqlite::Result<Vec<Track>> {
        let c = self.0.lock().unwrap();
        let mut s = c.prepare(&format!(
            "SELECT {COLS} FROM tracks t JOIN playlist_tracks p ON p.path = t.path
             WHERE p.playlist_id = ?1 ORDER BY p.pos"))?;
        s.query_map([id], row)?.collect()
    }

    pub fn add_to_playlist(&self, id: i64, path: &str) -> rusqlite::Result<()> {
        self.0.lock().unwrap().execute(
            "INSERT OR IGNORE INTO playlist_tracks
             SELECT ?1, ?2, COALESCE(MAX(pos), 0) + 1 FROM playlist_tracks WHERE playlist_id = ?1",
            params![id, path],
        ).map(|_| ())
    }

    pub fn add_to_playlist_at(&self, id: i64, path: &str, pos: i64) -> rusqlite::Result<()> {
        self.0.lock().unwrap()
            .execute("INSERT OR IGNORE INTO playlist_tracks VALUES (?1, ?2, ?3)", params![id, path, pos])
            .map(|_| ())
    }

    pub fn add_pending(&self, id: i64, t: &Pending, pos: i64) -> rusqlite::Result<()> {
        self.0.lock().unwrap().execute(
            "INSERT INTO playlist_pending (playlist_id, title, artist, album, duration_secs, pos, uri, cover) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
            params![id, t.title, t.artist, t.album, t.duration_secs as i64, pos, t.uri, t.cover],
        ).map(|_| ())
    }

    pub fn pending(&self, id: i64) -> rusqlite::Result<Vec<Pending>> {
        let c = self.0.lock().unwrap();
        let mut s = c.prepare(
            "SELECT id, title, artist, album, duration_secs, uri, cover FROM playlist_pending WHERE playlist_id = ?1 ORDER BY pos")?;
        s.query_map([id], |r| Ok(Pending {
            id: r.get(0)?, title: r.get(1)?, artist: r.get(2)?, album: r.get(3)?, duration_secs: r.get::<_, i64>(4)? as u64, uri: r.get(5)?, cover: r.get(6)?,
        }))?.collect()
    }

    pub fn pending_one(&self, id: i64) -> rusqlite::Result<Option<Pending>> {
        let c = self.0.lock().unwrap();
        let mut s = c.prepare("SELECT id, title, artist, album, duration_secs, uri, cover FROM playlist_pending WHERE id = ?1")?;
        let mut rows = s.query_map([id], |r| Ok(Pending {
            id: r.get(0)?, title: r.get(1)?, artist: r.get(2)?, album: r.get(3)?, duration_secs: r.get::<_, i64>(4)? as u64, uri: r.get(5)?, cover: r.get(6)?,
        }))?;
        rows.next().transpose()
    }

    /// The pending entry's song now exists at `path`: it takes the entry's place in the playlist.
    pub fn resolve_pending(&self, id: i64, path: &str) -> rusqlite::Result<()> {
        let mut c = self.0.lock().unwrap();
        let tx = c.transaction()?;
        tx.execute(
            "INSERT OR IGNORE INTO playlist_tracks SELECT playlist_id, ?2, pos FROM playlist_pending WHERE id = ?1",
            params![id, path],
        )?;
        tx.execute("DELETE FROM playlist_pending WHERE id = ?1", [id])?;
        tx.commit()
    }

    pub fn remove_pending(&self, id: i64) -> rusqlite::Result<()> {
        self.0.lock().unwrap().execute("DELETE FROM playlist_pending WHERE id = ?1", [id]).map(|_| ())
    }

    pub fn remove_from_playlist(&self, id: i64, path: &str) -> rusqlite::Result<()> {
        self.0.lock().unwrap()
            .execute("DELETE FROM playlist_tracks WHERE playlist_id=?1 AND path=?2", params![id, path])
            .map(|_| ())
    }

    /// Small app preferences (JSON or plain text) that should live with the library, not the webview.
    pub fn setting(&self, key: &str) -> rusqlite::Result<Option<String>> {
        use rusqlite::OptionalExtension;
        self.0.lock().unwrap().query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| r.get(0)).optional()
    }

    pub fn set_setting(&self, key: &str, value: Option<&str>) -> rusqlite::Result<()> {
        let c = self.0.lock().unwrap();
        match value {
            Some(v) => c.execute("INSERT INTO settings VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = ?2", params![key, v]),
            None => c.execute("DELETE FROM settings WHERE key = ?1", [key]),
        }
        .map(|_| ())
    }

    /// A subtitle file fetched before, as WebVTT; touching it keeps it off the cleanup list.
    pub fn subtitle(&self, url: &str) -> rusqlite::Result<Option<String>> {
        use rusqlite::OptionalExtension;
        let c = self.0.lock().unwrap();
        c.execute("UPDATE subtitles SET used = unixepoch() WHERE url = ?1", [url])?;
        c.query_row("SELECT vtt FROM subtitles WHERE url = ?1", [url], |r| r.get(0)).optional()
    }

    /// Keeps the 300 most recently used files (a few MB at most).
    pub fn save_subtitle(&self, url: &str, vtt: &str) -> rusqlite::Result<()> {
        let c = self.0.lock().unwrap();
        c.execute("INSERT OR REPLACE INTO subtitles VALUES (?1, ?2, unixepoch())", params![url, vtt])?;
        c.execute("DELETE FROM subtitles WHERE url NOT IN (SELECT url FROM subtitles ORDER BY used DESC LIMIT 300)", [])
            .map(|_| ())
    }

    /// A cached extension answer and its age in seconds.
    pub fn cached(&self, key: &str) -> rusqlite::Result<Option<(String, i64)>> {
        use rusqlite::OptionalExtension;
        self.0.lock().unwrap()
            .query_row("SELECT value, unixepoch() - saved FROM ext_cache WHERE key = ?1", [key], |r| Ok((r.get(0)?, r.get(1)?)))
            .optional()
    }

    /// Stores an answer; anything untouched for a week goes.
    pub fn cache(&self, key: &str, value: &str) -> rusqlite::Result<()> {
        let c = self.0.lock().unwrap();
        c.execute("INSERT OR REPLACE INTO ext_cache VALUES (?1, ?2, unixepoch())", params![key, value])?;
        c.execute("DELETE FROM ext_cache WHERE saved < unixepoch() - 7 * 86400", []).map(|_| ())
    }

    /// iOS moves the app's data container (a new UUID in its path) on every reinstall or update, so saved
    /// paths point at the old one. Rewrites `…/Application/<old>/` to the container `dir` is in.
    #[cfg(any(target_os = "ios", test))]
    pub fn rehome(&self, dir: &str) -> rusqlite::Result<()> {
        let id = |p: &str| {
            let s = &p[p.find("/Application/")?..];
            Some(s[..13 + s[13..].find('/')? + 1].to_owned())
        };
        let Some(new) = id(&format!("{dir}/")) else { return Ok(()) };
        let mut c = self.0.lock().unwrap();
        let mut olds: Vec<String> = c
            .prepare("SELECT path FROM folders UNION SELECT path FROM tracks")?
            .query_map([], |r| r.get::<_, String>(0))?
            .filter_map(|p| id(&p.ok()?))
            .filter(|o| *o != new)
            .collect();
        olds.sort();
        olds.dedup();
        let tx = c.transaction()?;
        tx.execute_batch("PRAGMA defer_foreign_keys = ON")?;
        for old in &olds {
            // rows that already exist under the new container (a folder added again) win; the old ones go
            for t in ["tracks", "playlist_tracks", "downloads", "folders"] {
                tx.execute(&format!("UPDATE OR IGNORE {t} SET path = replace(path, ?1, ?2)"), [old, &new])?;
                tx.execute(&format!("DELETE FROM {t} WHERE instr(path, ?1)"), [old])?;
            }
            tx.execute("UPDATE settings SET value = replace(value, ?1, ?2)", [old, &new])?;
        }
        tx.commit()
    }

    pub fn folders(&self) -> rusqlite::Result<Vec<String>> {
        let c = self.0.lock().unwrap();
        let mut s = c.prepare("SELECT path FROM folders ORDER BY path")?;
        s.query_map([], |r| r.get(0))?.collect()
    }

    pub fn add_folder(&self, path: &str) -> rusqlite::Result<()> {
        self.0.lock().unwrap().execute("INSERT OR IGNORE INTO folders VALUES (?1)", [path]).map(|_| ())
    }

    /// Forgets a library folder and its songs.
    pub fn remove_folder(&self, path: &str) -> rusqlite::Result<()> {
        self.0.lock().unwrap().execute("DELETE FROM folders WHERE path=?1", [path])?;
        self.remove_under(&format!("{}/", path.trim_end_matches('/')))
    }

    /// Drops tracks under the directory `dir` (ending in `/`).
    pub fn remove_under(&self, dir: &str) -> rusqlite::Result<()> {
        self.0.lock().unwrap()
            .execute("DELETE FROM tracks WHERE substr(path, 1, length(?1)) = ?1", [dir])
            .map(|_| ())
    }

    /// Drops tracks (and their playlist entries) whose path is in `paths`.
    pub fn remove(&self, paths: &[String]) -> rusqlite::Result<()> {
        let mut c = self.0.lock().unwrap();
        let tx = c.transaction()?;
        for p in paths {
            tx.execute("DELETE FROM tracks WHERE path=?1", [p])?;
        }
        tx.commit()
    }

    /// Remembers which extension (and which of its tracks) a downloaded file came from.
    pub fn set_origin(&self, path: &str, ext: &str, track: &str) -> rusqlite::Result<()> {
        self.0.lock().unwrap()
            .execute("INSERT OR REPLACE INTO downloads VALUES (?1,?2,?3)", params![path, ext, track])
            .map(|_| ())
    }

    pub fn origin(&self, path: &str) -> rusqlite::Result<Option<(String, String)>> {
        let c = self.0.lock().unwrap();
        let mut s = c.prepare("SELECT ext, track FROM downloads WHERE path=?1")?;
        let mut rows = s.query_map([path], |r| Ok((r.get(0)?, r.get(1)?)))?;
        rows.next().transpose()
    }

    pub fn track(&self, path: &str) -> rusqlite::Result<Option<Track>> {
        let c = self.0.lock().unwrap();
        let mut s = c.prepare("SELECT path, title, artist, album, duration_secs FROM tracks WHERE path=?1")?;
        let mut rows = s.query_map([path], row)?;
        rows.next().transpose()
    }

    /// A re-download landed at another path: playlists follow it, the old row goes.
    pub fn move_path(&self, old: &str, new: &str) -> rusqlite::Result<()> {
        let mut c = self.0.lock().unwrap();
        let tx = c.transaction()?;
        tx.execute("UPDATE OR IGNORE playlist_tracks SET path=?2 WHERE path=?1", [old, new])?;
        tx.execute("DELETE FROM tracks WHERE path=?1", [old])?;
        tx.commit()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rehome_moves_paths_to_the_new_container() {
        let f = std::env::temp_dir().join(format!("rehome-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&f);
        let db = Db::open(&f).unwrap();
        let (old, new) = ("/private/var/mobile/Containers/Data/Application/AAA", "/private/var/mobile/Containers/Data/Application/BBB");
        let t = |p: &str| Track { path: p.into(), title: "t".into(), artist: "a".into(), album: "b".into(), duration_secs: 1 };
        db.add_folder(&format!("{old}/Documents")).unwrap();
        db.upsert(&[t(&format!("{old}/Documents/a.flac")), t(&format!("{old}/Documents/b.flac")), t(&format!("{new}/Documents/b.flac"))]).unwrap();
        db.0.lock().unwrap().execute_batch(&format!(
            "INSERT INTO playlists VALUES (1, 'p'); INSERT INTO playlist_tracks VALUES (1, '{old}/Documents/a.flac', 0);"
        )).unwrap();
        db.rehome(&format!("{new}/Library/Application Support/id.neptunus.desktop")).unwrap();
        assert_eq!(db.folders().unwrap(), vec![format!("{new}/Documents")]);
        let c = db.0.lock().unwrap();
        let paths: Vec<String> = c.prepare("SELECT path FROM tracks ORDER BY path").unwrap().query_map([], |r| r.get(0)).unwrap().map(Result::unwrap).collect();
        assert_eq!(paths, vec![format!("{new}/Documents/a.flac"), format!("{new}/Documents/b.flac")]);
        let pl: String = c.query_row("SELECT path FROM playlist_tracks", [], |r| r.get(0)).unwrap();
        assert_eq!(pl, format!("{new}/Documents/a.flac"));
        drop(c);
        let _ = std::fs::remove_file(&f);
    }
}
