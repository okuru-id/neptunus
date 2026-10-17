use std::{
    sync::{
        atomic::{AtomicI64, AtomicU64, AtomicUsize, Ordering},
        Mutex,
    },
    time::{SystemTime, UNIX_EPOCH},
};
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, WebviewWindow};

/// When the mini player last hid itself on blur. Clicking the tray icon steals focus first,
/// so without this the click would immediately re-open the popover it just closed.
static LAST_BLUR_HIDE: AtomicU64 = AtomicU64::new(0);

/// Physical screen point of the tray icon from the last click (`i64::MIN` = unknown).
static TRAY_X: AtomicI64 = AtomicI64::new(i64::MIN);
static TRAY_Y: AtomicI64 = AtomicI64::new(i64::MIN);

/// Bumped on every track change so a slow cover decode can't overwrite a newer track's art.
static TRACK_GEN: AtomicUsize = AtomicUsize::new(0);

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// Playback state mirrored into the tray menu, so the dropdown itself works as a mini player.
#[derive(Default, Clone)]
struct NowMenu {
    title: String,
    artist: String,
    playing: Option<bool>,
    art: Option<image::RgbaImage>,
}

impl NowMenu {
    fn loaded(&self) -> bool {
        self.playing.is_some()
    }
    fn title(&self) -> &str {
        if self.loaded() { &self.title } else { "Tidak ada lagu yang diputar" }
    }
    fn artist(&self) -> &str {
        match (self.loaded(), self.artist.is_empty()) {
            (false, _) => " ",
            (true, true) => "Tidak diketahui",
            (true, false) => &self.artist,
        }
    }
    fn toggle(&self) -> &str {
        if self.playing == Some(true) { "⏸  Jeda" } else { "▶  Putar" }
    }
}

/// The last known tray point is saved, so the mini player can open under the icon even when it is opened
/// from the menu (GNOME only reports the icon position on a double click).
fn pos_file(app: &AppHandle) -> Option<std::path::PathBuf> {
    app.path().app_data_dir().ok().map(|d| d.join("tray-pos"))
}

fn set_tray_point(app: &AppHandle, x: i64, y: i64) {
    TRAY_X.store(x, Ordering::Relaxed);
    TRAY_Y.store(y, Ordering::Relaxed);
    if let Some(f) = pos_file(app) {
        let _ = std::fs::write(f, format!("{x} {y}"));
    }
}

pub fn init(app: &AppHandle) -> tauri::Result<()> {
    app.manage(Mutex::new(NowMenu::default()));
    if let Some(s) = pos_file(app).and_then(|f| std::fs::read_to_string(f).ok()) {
        if let Some((x, y)) = s.split_once(' ').and_then(|(x, y)| Some((x.parse().ok()?, y.parse().ok()?))) {
            TRAY_X.store(x, Ordering::Relaxed);
            TRAY_Y.store(y, Ordering::Relaxed);
        }
    }
    backend::init(app)
}

fn update(app: &AppHandle, f: impl FnOnce(&mut NowMenu)) {
    let Some(state) = app.try_state::<Mutex<NowMenu>>() else { return };
    // Snapshot and unlock first: the backend may wait on the main thread, which may be waiting on this lock.
    let snapshot = {
        let mut s = state.lock().unwrap();
        f(&mut s);
        s.clone()
    };
    backend::refresh(app, &snapshot);
}

/// Called when the playing track changes.
pub fn set_track(app: &AppHandle, path: &str, title: &str, artist: &str) {
    update(app, |s| {
        s.title = title.to_owned();
        s.artist = artist.to_owned();
        s.art = None;
    });

    // Decode off the main thread (sync commands run on it).
    let token = TRACK_GEN.fetch_add(1, Ordering::SeqCst) + 1;
    let (app, path) = (app.clone(), path.to_owned());
    std::thread::spawn(move || {
        let Some(img) = crate::cover_bytes(&path).and_then(|(_, data)| image::load_from_memory(&data).ok()) else { return };
        let img = art(&img, 64, 12);
        if TRACK_GEN.load(Ordering::SeqCst) == token {
            update(&app, |s| s.art = Some(img));
        }
    });
}

/// `None` = stopped, `Some(true)` = playing, `Some(false)` = paused.
pub fn set_playing(app: &AppHandle, playing: Option<bool>) {
    let Some(state) = app.try_state::<Mutex<NowMenu>>() else { return };
    if state.lock().unwrap().playing == playing {
        return; // unchanged; avoid rebuilding the menu
    }
    if playing.is_none() {
        TRACK_GEN.fetch_add(1, Ordering::SeqCst);
    }
    update(app, |s| {
        s.playing = playing;
        if playing.is_none() {
            s.art = None;
        }
    });
}

/// Square-cropped, rounded thumbnail of the cover.
fn art(img: &image::DynamicImage, size: u32, radius: u32) -> image::RgbaImage {
    let mut px = img.resize_to_fill(size, size, image::imageops::FilterType::Triangle).to_rgba8();
    let (r, s) = (radius as f32, size as f32);
    for (x, y, p) in px.enumerate_pixels_mut() {
        // pixel-centre distance from the corner arc centre, for a 1px anti-aliased edge
        let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
        let dx = (r - fx).max(fx - (s - r)).max(0.0);
        let dy = (r - fy).max(fy - (s - r)).max(0.0);
        let cover = (r + 0.5 - (dx * dx + dy * dy).sqrt()).clamp(0.0, 1.0);
        p[3] = (p[3] as f32 * cover) as u8;
    }
    px
}

fn menu_event(app: &AppHandle, id: &str) {
    match id {
        "toggle" | "prev" | "next" => { let _ = app.emit("media-key", id); }
        "mini" => toggle_mini(app),
        "show" | "np-title" | "np-artist" => show_main(app),
        "quit" => app.exit(0),
        _ => {}
    }
}

/// Linux: a StatusNotifierItem (AppIndicator), drawn by the desktop with real transparency. GNOME's
/// extension opens the menu on any single click and only forwards a left double click (`Activate`),
/// so a double click opens the mini player.
#[cfg(target_os = "linux")]
mod backend {
    use super::*;
    use ksni::{blocking::{Handle, TrayMethods}, menu::StandardItem, MenuItem};
    use std::sync::OnceLock;

    static HANDLE: OnceLock<Handle<Tray>> = OnceLock::new();

    struct Tray {
        app: AppHandle,
        now: NowMenu,
        icon: Vec<ksni::Icon>,
        art_png: Vec<u8>,
    }

    impl ksni::Tray for Tray {
        fn id(&self) -> String {
            "pixelplay".into()
        }
        fn title(&self) -> String {
            "PixelPlay".into()
        }
        fn icon_pixmap(&self) -> Vec<ksni::Icon> {
            self.icon.clone()
        }
        fn tool_tip(&self) -> ksni::ToolTip {
            let title = if self.now.loaded() { format!("{} · {}", self.now.title(), self.now.artist()) } else { "PixelPlay".into() };
            ksni::ToolTip { title, ..Default::default() }
        }
        fn activate(&mut self, x: i32, y: i32) {
            set_tray_point(&self.app, x as i64, y as i64);
            toggle_mini(&self.app);
        }
        fn menu(&self) -> Vec<MenuItem<Self>> {
            let std = |id: &'static str, label: &str, enabled: bool| StandardItem {
                label: label.into(),
                enabled,
                activate: Box::new(move |t: &mut Self| menu_event(&t.app, id)),
                ..Default::default()
            };
            let item = |id, label: &str, enabled| std(id, label, enabled).into();
            let s = &self.now;
            vec![
                StandardItem { icon_data: self.art_png.clone(), ..std("np-title", s.title(), true) }.into(),
                item("np-artist", s.artist(), true),
                MenuItem::Separator,
                item("toggle", s.toggle(), s.loaded()),
                item("prev", "⏮  Sebelumnya", s.loaded()),
                item("next", "⏭  Berikutnya", s.loaded()),
                MenuItem::Separator,
                item("mini", "Mini player", true),
                item("show", "Tampilkan PixelPlay", true),
                item("quit", "Keluar", true),
            ]
        }
    }

    pub fn init(app: &AppHandle) -> tauri::Result<()> {
        // SNI pixmaps are ARGB32 in network byte order
        let icon = app
            .default_window_icon()
            .map(|i| {
                let data = i.rgba().chunks_exact(4).flat_map(|p| [p[3], p[0], p[1], p[2]]).collect();
                vec![ksni::Icon { width: i.width() as i32, height: i.height() as i32, data }]
            })
            .unwrap_or_default();
        let tray = Tray { app: app.clone(), now: NowMenu::default(), icon, art_png: Vec::new() };
        match tray.spawn() {
            Ok(h) => { let _ = HANDLE.set(h); }
            Err(e) => eprintln!("tray: {e}"),
        }
        Ok(())
    }

    pub fn refresh(_app: &AppHandle, now: &NowMenu) {
        let Some(h) = HANDLE.get() else { return };
        let mut art_png = Vec::new();
        if let Some(a) = &now.art {
            let _ = a.write_to(&mut std::io::Cursor::new(&mut art_png), image::ImageFormat::Png);
        }
        let now = now.clone();
        h.update(move |t| {
            t.now = now;
            t.art_png = art_png;
        });
    }
}

#[cfg(not(target_os = "linux"))]
mod backend {
    use super::*;
    use tauri::{
        image::Image,
        menu::{IconMenuItem, Menu, MenuItem, PredefinedMenuItem},
        tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
        Wry,
    };

    fn build_menu(app: &AppHandle, s: &NowMenu) -> tauri::Result<Menu<Wry>> {
        let item = |id: &str, label: &str, enabled: bool| MenuItem::with_id(app, id, label, enabled, None::<&str>);
        let art = s.art.as_ref().map(|a| Image::new_owned(a.as_raw().clone(), a.width(), a.height()));
        Menu::with_items(
            app,
            &[
                &IconMenuItem::with_id(app, "np-title", s.title(), true, art, None::<&str>)?,
                &item("np-artist", s.artist(), true)?,
                &PredefinedMenuItem::separator(app)?,
                &item("toggle", s.toggle(), s.loaded())?,
                &item("prev", "⏮  Sebelumnya", s.loaded())?,
                &item("next", "⏭  Berikutnya", s.loaded())?,
                &PredefinedMenuItem::separator(app)?,
                &item("mini", "Mini player", true)?,
                &item("show", "Tampilkan PixelPlay", true)?,
                &item("quit", "Keluar", true)?,
            ],
        )
    }

    pub fn init(app: &AppHandle) -> tauri::Result<()> {
        let mut tray = TrayIconBuilder::with_id("main")
            .tooltip("PixelPlay")
            .menu(&build_menu(app, &NowMenu::default())?)
            .show_menu_on_left_click(false)
            .on_menu_event(|app, e| menu_event(app, e.id.as_ref()))
            .on_tray_icon_event(|tray, event| {
                if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, rect, .. } = event {
                    let (pos, size) = (rect.position.to_physical::<f64>(1.0), rect.size.to_physical::<f64>(1.0));
                    set_tray_point(tray.app_handle(), (pos.x + size.width / 2.0) as i64, (pos.y + size.height / 2.0) as i64);
                    toggle_mini(tray.app_handle());
                }
            });
        if let Some(icon) = app.default_window_icon() {
            tray = tray.icon(icon.clone());
        }
        tray.build(app)?;
        Ok(())
    }

    pub fn refresh(app: &AppHandle, now: &NowMenu) {
        if let (Some(tray), Ok(menu)) = (app.tray_by_id("main"), build_menu(app, now)) {
            let _ = tray.set_menu(Some(menu));
        }
    }
}

/// Centre the popover on the tray icon: under the panel when the tray is at the top of the screen
/// (Linux, macOS), above the taskbar when it is at the bottom (Windows), clamped to the work area.
fn place(mini: &WebviewWindow) {
    let (tx, ty) = (TRAY_X.load(Ordering::Relaxed), TRAY_Y.load(Ordering::Relaxed));
    let known = tx != i64::MIN;
    let monitor = known
        .then(|| mini.monitor_from_point(tx as f64, ty as f64).ok().flatten())
        .flatten()
        .or_else(|| mini.primary_monitor().ok().flatten());
    let Some(m) = monitor else { return };
    let area = m.work_area();
    let (ax, ay, aw, ah) = (area.position.x, area.position.y, area.size.width as i32, area.size.height as i32);
    // a window that was never shown can report 0×0; fall back to its size in tauri.conf.json
    let (w, h) = match mini.outer_size() {
        Ok(s) if s.width > 1 => (s.width as i32, s.height as i32),
        _ => mini
            .config()
            .app
            .windows
            .iter()
            .find(|w| w.label == mini.label())
            .map(|w| ((w.width * m.scale_factor()) as i32, (w.height * m.scale_factor()) as i32))
            .unwrap_or((360, 176)),
    };
    let gap = (12.0 * m.scale_factor()) as i32;

    // unknown tray (e.g. opened from the menu before any click): the usual corner per OS
    let at_bottom = if known { ty as i32 > ay + ah / 2 } else { cfg!(target_os = "windows") };
    let cx = if known { tx as i32 } else { ax + aw - w / 2 - gap };
    let x = (cx - w / 2).clamp(ax + gap, (ax + aw - w - gap).max(ax + gap));
    let y = if at_bottom { ay + ah - h - gap } else { ay + gap };
    let _ = mini.set_position(PhysicalPosition::new(x, y));
}

pub fn toggle_mini(app: &AppHandle) {
    let Some(mini) = app.get_webview_window("mini") else { return };
    if mini.is_visible().unwrap_or(false) {
        let _ = mini.hide();
    } else if now_ms() - LAST_BLUR_HIDE.load(Ordering::Relaxed) > 300 {
        place(&mini);
        let _ = mini.show();
        place(&mini); // again: before the first show the window manager may not report the final size
        let _ = mini.set_focus();
    }
}

pub fn hide_mini_on_blur(mini: &tauri::Window) {
    LAST_BLUR_HIDE.store(now_ms(), Ordering::Relaxed);
    let _ = mini.hide();
}

pub fn show_main(app: &AppHandle) {
    if let Some(mini) = app.get_webview_window("mini") {
        let _ = mini.hide();
    }
    if let Some(main) = app.get_webview_window("main") {
        let _ = main.unminimize();
        let _ = main.show();
        let _ = main.set_focus();
    }
}

#[tauri::command]
pub fn open_main(app: AppHandle) {
    show_main(&app);
}
