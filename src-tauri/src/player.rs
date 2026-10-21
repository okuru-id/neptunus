use rodio::{ChannelCount, Decoder, Player, Sample, SampleRate, Source, mixer::Mixer};
use serde::Serialize;
use tauri::Manager;
use std::{
    fs::File,
    sync::{
        Arc, Mutex,
        atomic::{AtomicI32, AtomicU64, Ordering::Relaxed},
    },
    time::Duration,
};

/// The device sink is not `Send`, so it lives on its own parked thread; only the mixer is shared.
pub struct AudioState {
    mixer: Mixer,
    player: Mutex<Option<Player>>,
    eq: Arc<[AtomicI32; 2 * BANDS]>,
    /// bumped by every `play`, so a song that took long to open doesn't start over a newer pick
    play_id: AtomicU64,
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
        Ok(Self { mixer, player: Mutex::new(None), play_id: AtomicU64::new(0), eq: Arc::new(std::array::from_fn(|i| AtomicI32::new(if i < BANDS { 0 } else { FREQS[i - BANDS] as i32 }))) })
    }
}

impl AudioState {
    pub fn is_playing(&self) -> bool {
        self.player.lock().unwrap().as_ref().is_some_and(|p| !p.is_paused() && !p.empty())
    }
}

/// A local file, or an `s3://` object streamed with range requests.
fn decode(path: &str, eq: &Arc<[AtomicI32; 2 * BANDS]>) -> Result<Box<dyn Source + Send>, String> {
    Ok(Box::new(Eq::new(decode_raw(path)?, eq.clone())))
}

fn decode_raw(path: &str) -> Result<Box<dyn Source + Send>, String> {
    if let Some(r) = crate::s3::reader(path) {
        let r = r?.with_readahead(path);
        let len = r.len();
        let d = Decoder::builder().with_data(r).with_byte_len(len).build().map_err(|e| e.to_string())?;
        return Ok(Box::new(d));
    }
    let file = File::open(path).map_err(|e| e.to_string())?;
    Ok(Box::new(Decoder::try_from(file).map_err(|e| e.to_string())?))
}

/// Length of the crossfade when switching songs.
const FADE: Duration = Duration::from_millis(400);

/// Opens `path` and switches to it: the old song keeps playing while the new one opens (an S3 stream takes
/// a moment), then fades out as the new one fades in. `false` when a newer pick came first and this one
/// was dropped.
#[tauri::command]
pub async fn play(app: tauri::AppHandle, path: String, volume: f32) -> Result<bool, String> {
    let state = app.state::<AudioState>();
    let id = state.play_id.fetch_add(1, Relaxed) + 1;
    let eq = state.eq.clone();
    let source = crate::blocking(move || decode(&path, &eq)).await?;
    if state.play_id.load(Relaxed) != id {
        return Ok(false);
    }
    let player = Player::connect_new(&state.mixer);
    player.set_volume(volume as _);
    player.append(source.fade_in(FADE));
    if let Some(old) = state.player.lock().unwrap().replace(player) {
        std::thread::spawn(move || {
            let (v, steps) = (old.volume(), 20);
            for i in 1..=steps {
                old.set_volume(v * (steps - i) as f32 / steps as f32);
                std::thread::sleep(FADE / steps);
            }
            old.stop();
        });
    }
    Ok(true)
}

/// Appends a track to the running player so it starts with no gap when the current one ends.
#[tauri::command]
pub async fn queue_next(app: tauri::AppHandle, path: String) -> Result<(), String> {
    let state = app.state::<AudioState>();
    let eq = state.eq.clone();
    let source = crate::blocking(move || decode(&path, &eq)).await?;
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

const BANDS: usize = 10;
const FREQS: [f32; BANDS] = [31.0, 62.0, 125.0, 250.0, 500.0, 1000.0, 2000.0, 4000.0, 8000.0, 16000.0];

/// Sets every band's gain (tenths of a dB) and center frequency (Hz); applies live, also to the preloaded next track.
#[tauri::command]
pub fn set_eq(state: tauri::State<AudioState>, gains: [i32; BANDS], freqs: [i32; BANDS]) {
    for (a, g) in state.eq.iter().zip(gains.into_iter().chain(freqs)) {
        a.store(g, Relaxed);
    }
}

/// 10-band parametric equalizer: one biquad per band and channel (see `biquad`). The preamp
/// cancels the peak of the combined response (overlapping boosts add up), so the result can't clip.
struct Eq<S> {
    inner: S,
    gains: Arc<[AtomicI32; 2 * BANDS]>,
    cur: [i32; 2 * BANDS],
    coef: [[f32; 5]; BANDS], // b0 b1 b2 a1 a2, normalised
    pre: f32,
    on: bool,
    z: Vec<[[f32; 2]; BANDS]>, // per channel, per band: z1 z2
    ch: usize,
    rate: u32,
    tick: u32,
}

impl<S: Source> Eq<S> {
    fn new(inner: S, gains: Arc<[AtomicI32; 2 * BANDS]>) -> Self {
        let mut e = Self { inner, gains, cur: [0; 2 * BANDS], coef: [[0.0; 5]; BANDS], pre: 1.0, on: false, z: Vec::new(), ch: 0, rate: 0, tick: 0 };
        e.frame_start();
        e
    }

    /// Called at each frame boundary: follows channel-count / sample-rate changes and new settings.
    fn frame_start(&mut self) {
        let (chs, rate) = (self.inner.channels().get() as usize, self.inner.sample_rate().get());
        let changed = chs != self.z.len() || rate != self.rate;
        if changed {
            self.z = vec![[[0.0; 2]; BANDS]; chs];
            self.rate = rate;
        }
        self.tick = (self.tick + 1) % 256;
        if !changed && self.tick != 0 {
            return;
        }
        let now: [i32; 2 * BANDS] = std::array::from_fn(|i| self.gains[i].load(Relaxed));
        if !changed && now == self.cur {
            return;
        }
        self.cur = now;
        let sr = rate as f32;
        for i in 0..BANDS {
            self.coef[i] = biquad(i, (now[BANDS + i] as f32).clamp(10.0, sr * 0.45), now[i] as f32 / 10.0, sr);
        }
        let on = now[..BANDS].iter().any(|&g| g != 0);
        if on && !self.on {
            self.z.iter_mut().for_each(|z| *z = [[0.0; 2]; BANDS]);
        }
        self.on = on;
        // peak of the summed response over 20 Hz..20 kHz (log-spaced), never above 0 dB of attenuation
        let peak = (0..=256)
            .map(|k| {
                let w = std::f32::consts::TAU * (20.0 * 1000f32.powf(k as f32 / 256.0)).min(sr * 0.49) / sr;
                self.coef.iter().map(|c| mag(c, w)).product::<f32>()
            })
            .fold(1.0, f32::max);
        self.pre = 1.0 / peak;
    }
}

/// RBJ cookbook biquad, normalised [b0 b1 b2 a1 a2]: the first band is a low shelf, the last a high
/// shelf, the rest peaking filters with Q 1.4. Must match `coef` in Equalizer.vue.
fn biquad(band: usize, f: f32, db: f32, sr: f32) -> [f32; 5] {
    let a = 10f32.powf(db / 40.0);
    let w = std::f32::consts::TAU * f / sr;
    let (c, s) = (w.cos(), w.sin());
    let [b0, b1, b2, a0, a1, a2] = if band == 0 || band == BANDS - 1 {
        let k = 2.0 * a.sqrt() * s / std::f32::consts::SQRT_2; // shelf slope 1
        let sg = if band == 0 { 1.0 } else { -1.0 };
        [
            a * ((a + 1.0) - sg * (a - 1.0) * c + k),
            sg * 2.0 * a * ((a - 1.0) - sg * (a + 1.0) * c),
            a * ((a + 1.0) - sg * (a - 1.0) * c - k),
            (a + 1.0) + sg * (a - 1.0) * c + k,
            -sg * 2.0 * ((a - 1.0) + sg * (a + 1.0) * c),
            (a + 1.0) + sg * (a - 1.0) * c - k,
        ]
    } else {
        let al = s / (2.0 * 1.4);
        [1.0 + al * a, -2.0 * c, 1.0 - al * a, 1.0 + al / a, -2.0 * c, 1.0 - al / a]
    };
    [b0 / a0, b1 / a0, b2 / a0, a1 / a0, a2 / a0]
}

/// |H(e^jw)| of a normalised biquad.
fn mag(c: &[f32; 5], w: f32) -> f32 {
    let (c1, s1, c2, s2) = (w.cos(), w.sin(), (2.0 * w).cos(), (2.0 * w).sin());
    let num = (c[0] + c[1] * c1 + c[2] * c2).powi(2) + (c[1] * s1 + c[2] * s2).powi(2);
    let den = (1.0 + c[3] * c1 + c[4] * c2).powi(2) + (c[3] * s1 + c[4] * s2).powi(2);
    (num / den).sqrt()
}

impl<S: Source> Iterator for Eq<S> {
    type Item = Sample;

    fn next(&mut self) -> Option<Sample> {
        if self.ch == 0 {
            self.frame_start();
        }
        let mut x = self.inner.next()?;
        if self.on {
            x *= self.pre;
            for (c, z) in self.coef.iter().zip(self.z[self.ch].iter_mut()) {
                let y = c[0] * x + z[0];
                z[0] = c[1] * x - c[3] * y + z[1];
                z[1] = c[2] * x - c[4] * y;
                x = y;
            }
        }
        self.ch = (self.ch + 1) % self.z.len();
        Some(x)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}

impl<S: Source> Source for Eq<S> {
    fn current_span_len(&self) -> Option<usize> {
        self.inner.current_span_len()
    }
    fn channels(&self) -> ChannelCount {
        self.inner.channels()
    }
    fn sample_rate(&self) -> SampleRate {
        self.inner.sample_rate()
    }
    fn total_duration(&self) -> Option<Duration> {
        self.inner.total_duration()
    }
    fn try_seek(&mut self, pos: Duration) -> Result<(), rodio::source::SeekError> {
        self.z.iter_mut().for_each(|z| *z = [[0.0; 2]; BANDS]);
        self.ch = 0;
        self.inner.try_seek(pos)
    }
}
