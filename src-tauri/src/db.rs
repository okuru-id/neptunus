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
             CREATE TABLE IF NOT EXISTS playlists (
               id INTEGER PRIMARY KEY, name TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS playlist_tracks (
               playlist_id INTEGER NOT NULL REFERENCES playlists(id) ON DELETE CASCADE,
               path TEXT NOT NULL REFERENCES tracks(path) ON DELETE CASCADE,
               pos INTEGER NOT NULL, PRIMARY KEY (playlist_id, path));",
        )?;
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

    pub fn remove_from_playlist(&self, id: i64, path: &str) -> rusqlite::Result<()> {
        self.0.lock().unwrap()
            .execute("DELETE FROM playlist_tracks WHERE playlist_id=?1 AND path=?2", params![id, path])
            .map(|_| ())
    }
}
