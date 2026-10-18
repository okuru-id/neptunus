use rodio::{Decoder, Player, mixer::Mixer};
use serde::Serialize;
use std::{fs::File, sync::Mutex, time::Duration};

/// The device sink is not `Send`, so it lives on its own parked thread; only the mixer is shared.
pub struct AudioState {
    mixer: Mixer,
    player: Mutex<Option<Player>>,
}

#[derive(Serialize)]
pub struct PlayerStatus {
    position_secs: f64,
    paused: bool,
    finished: bool,
    /// Sounds still queued, including the one playing (2 while the next track is preloaded).
    queued: usize,
}

impl AudioState {
    pub fn new() -> Result<Self, String> {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || match rodio::DeviceSinkBuilder::open_default_sink() {
            Ok(sink) => {
                let _ = tx.send(Ok(sink.mixer().clone()));
                loop {
                    std::thread::park();
                }
            }
            Err(e) => {
                let _ = tx.send(Err(e.to_string()));
            }
        });
        let mixer = rx.recv().map_err(|e| e.to_string())??;
        Ok(Self { mixer, player: Mutex::new(None) })
    }
}

impl AudioState {
    pub fn is_playing(&self) -> bool {
        self.player.lock().unwrap().as_ref().is_some_and(|p| !p.is_paused() && !p.empty())
    }
}

/// A local file, or an `s3://` object streamed with range requests.
fn decode(path: &str) -> Result<Box<dyn rodio::Source + Send>, String> {
    if let Some(r) = crate::s3::reader(path) {
        let r = r?.with_readahead();
        let len = r.len();
        let d = Decoder::builder().with_data(r).with_byte_len(len).build().map_err(|e| e.to_string())?;
        return Ok(Box::new(d));
    }
    let file = File::open(path).map_err(|e| e.to_string())?;
    Ok(Box::new(Decoder::try_from(file).map_err(|e| e.to_string())?))
}

#[tauri::command]
pub async fn play(state: tauri::State<'_, AudioState>, path: String, volume: f32) -> Result<(), String> {
    let source = decode(&path)?;
    let player = Player::connect_new(&state.mixer);
    player.set_volume(volume as _);
    player.append(source);
    *state.player.lock().unwrap() = Some(player);
    Ok(())
}

/// Appends a track to the running player so it starts with no gap when the current one ends.
#[tauri::command]
pub async fn queue_next(state: tauri::State<'_, AudioState>, path: String) -> Result<(), String> {
    let source = decode(&path)?;
    if let Some(p) = state.player.lock().unwrap().as_ref() {
        p.append(source);
    }
    Ok(())
}

#[tauri::command]
pub fn set_paused(state: tauri::State<AudioState>, paused: bool) {
    if let Some(p) = state.player.lock().unwrap().as_ref() {
        if paused { p.pause() } else { p.play() }
    }
}

#[tauri::command]
pub fn seek(state: tauri::State<AudioState>, secs: f64) -> Result<(), String> {
    match state.player.lock().unwrap().as_ref() {
        Some(p) => p.try_seek(Duration::from_secs_f64(secs)).map_err(|e| e.to_string()),
        None => Ok(()),
    }
}

#[tauri::command]
pub fn set_volume(state: tauri::State<AudioState>, volume: f32) {
    if let Some(p) = state.player.lock().unwrap().as_ref() {
        p.set_volume(volume as _);
    }
}

#[tauri::command]
pub fn status(state: tauri::State<AudioState>) -> Option<PlayerStatus> {
    state.player.lock().unwrap().as_ref().map(|p| PlayerStatus {
        position_secs: p.get_pos().as_secs_f64(),
        paused: p.is_paused(),
        finished: p.empty(),
        queued: p.len(),
    })
}
