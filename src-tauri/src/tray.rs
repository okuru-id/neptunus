use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Mutex,
};
use tauri::{AppHandle, Emitter, Manager};

/// Bumped on every track change so a slow cover decode can't overwrite a newer track's art.
static TRACK_GEN: AtomicUsize = AtomicUsize::new(0);

/// Playback state mirrored into the tray menu, so the dropdown works as a small player.
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

pub fn init(app: &AppHandle) -> tauri::Result<()> {
    app.manage(Mutex::new(NowMenu::default()));
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
        "show" | "np-title" | "np-artist" => show_main(app),
        "quit" => app.exit(0),
        _ => {}
    }
}

/// Linux: a StatusNotifierItem (AppIndicator), drawn by the desktop with real transparency. GNOME's
/// extension opens the menu on any single click and only forwards a left double click (`Activate`),
/// so a double click shows the main window.
#[cfg(target_os = "linux")]
mod backend {
    use super::*;
    use ksni::{blocking::TrayMethods, menu::StandardItem, MenuItem};
    use std::sync::{mpsc, OnceLock};

    /// Updates go through a worker thread: `Handle::update` waits for the tray service, which may itself be
    /// waiting on the main thread (a tray click calls window methods), so calling it from a command on the
    /// main thread could deadlock.
    static UPDATES: OnceLock<mpsc::Sender<NowMenu>> = OnceLock::new();

    struct Tray {
        app: AppHandle,
        now: NowMenu,
        icon: Vec<ksni::Icon>,
        art_png: Vec<u8>,
    }

    impl ksni::Tray for Tray {
        fn id(&self) -> String {
            "neptunus".into()
        }
        fn title(&self) -> String {
            "Neptunus".into()
        }
        fn icon_pixmap(&self) -> Vec<ksni::Icon> {
            self.icon.clone()
        }
        fn tool_tip(&self) -> ksni::ToolTip {
            let title = if self.now.loaded() { format!("{} · {}", self.now.title(), self.now.artist()) } else { "Neptunus".into() };
            ksni::ToolTip { title, ..Default::default() }
        }
        fn activate(&mut self, _x: i32, _y: i32) {
            show_main(&self.app);
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
                item("show", "Tampilkan Neptunus", true),
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
        let h = match tray.spawn() {
            Ok(h) => h,
            Err(e) => {
                eprintln!("tray: {e}");
                return Ok(());
            }
        };
        let (tx, rx) = mpsc::channel::<NowMenu>();
        std::thread::spawn(move || {
            while let Ok(mut now) = rx.recv() {
                while let Ok(newer) = rx.try_recv() {
                    now = newer; // skip to the latest state
                }
                let mut art_png = Vec::new();
                if let Some(a) = &now.art {
                    let _ = a.write_to(&mut std::io::Cursor::new(&mut art_png), image::ImageFormat::Png);
                }
                h.update(move |t| {
                    t.now = now;
                    t.art_png = art_png;
                });
            }
        });
        let _ = UPDATES.set(tx);
        Ok(())
    }

    pub fn refresh(_app: &AppHandle, now: &NowMenu) {
        if let Some(tx) = UPDATES.get() {
            let _ = tx.send(now.clone());
        }
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
                &item("show", "Tampilkan Neptunus", true)?,
                &item("quit", "Keluar", true)?,
            ],
        )
    }

    pub fn init(app: &AppHandle) -> tauri::Result<()> {
        let mut tray = TrayIconBuilder::with_id("main")
            .tooltip("Neptunus")
            .menu(&build_menu(app, &NowMenu::default())?)
            .show_menu_on_left_click(false)
            .on_menu_event(|app, e| menu_event(app, e.id.as_ref()))
            .on_tray_icon_event(|tray, event| {
                if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                    show_main(tray.app_handle());
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

pub fn show_main(app: &AppHandle) {
    if let Some(main) = app.get_webview_window("main") {
        let _ = main.unminimize();
        let _ = main.show();
        let _ = main.set_focus();
    }
}
