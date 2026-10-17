# Neptunus

**[neptunus.id](https://neptunus.id)**

A lightweight desktop music player for Linux, Windows and macOS, built with Tauri 2, Rust and Vue 3.

Inspired by [**PixelPlayer**](https://github.com/PixelPlayerHQ/PixelPlayer), the beautiful Material 3 Expressive music player for Android. If you're on Android, go check it out and give it a star.

> **Not affiliated.** Neptunus is an independent, from-scratch project. It shares PixelPlayer's design ideas (expressive shapes, colour from the album art) but contains none of its code or assets, and is not endorsed by its author.

## Features

- **Local library:** scan a folder; tags and cover art are read with lofty and kept in SQLite. Browse by song, album, artist or playlist, with search.
- **Playback:** gapless queue, shuffle, repeat and volume. Decoding is done natively in Rust (rodio/symphonia): MP3, FLAC, AAC/M4A, OGG, Opus, WAV.
- **Material 3 Expressive UI:** the whole interface takes its colours from the playing cover, with expressive clip shapes and spring motion.
- **Synced lyrics:** looked up in this order: a sidecar `.lrc` file, the embedded lyrics tag, then [LRCLIB](https://lrclib.net). Lyrics fetched from LRCLIB are cached locally, so each song is only downloaded once.
- **OS media controls:** MPRIS on Linux, SMTC on Windows, Now Playing on macOS.
- **System tray:** the tray menu has playback controls and the cover. Double click (Linux) or click (Windows and macOS) the icon to show the window.
- **S3 source:** stream music from an S3-compatible bucket without downloading it (tags, cover and `.lrc` files are read with range requests).
- **Small:** uses the system webview, so there is no bundled browser runtime.

The UI text is currently in Indonesian.

## Build

Requirements:
- [Rust](https://rustup.rs) (stable)
- Node.js 20+
- The [Tauri prerequisites](https://tauri.app/start/prerequisites/) for your OS. On Linux this includes `libwebkit2gtk-4.1-dev` and `libasound2-dev`.

Run in development mode:

```bash
npm install
npm run tauri dev
```

Build an installer for the current OS (`.deb`/`.AppImage`, `.msi`/`.exe` or `.dmg`):

```bash
npm run tauri build
```

### Building on Debian / Ubuntu

```bash
sudo apt update
sudo apt install -y build-essential curl wget file pkg-config libssl-dev \
  libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev libasound2-dev patchelf
npm ci
npm run tauri build
```

Output is in `src-tauri/target/release/bundle/` (`deb/`, `appimage/`, `rpm/`). To build only the `.deb`, run `npm run tauri build -- --bundles deb`, then install it with `sudo apt install ./Neptunus_*.deb`.

Release builds are made on Ubuntu 22.04 so the binaries run on more distros (older glibc). Pushing a `v*` tag builds installers for all three OSes in CI and attaches them to a draft release.

### Linux notes

- The app runs natively on Wayland by default. Set `NEPTUNUS_X11=1` to run through XWayland instead; the window can sometimes stay unclickable there.
- The tray icon needs a StatusNotifierItem host. On GNOME that is the AppIndicator extension, which Ubuntu ships by default.

## Tech stack

| Part | Choice |
|---|---|
| Shell | Tauri 2 |
| UI | Vue 3 (`<script setup>`), TypeScript, Vite |
| Audio | rodio + symphonia |
| Tags / cover art | lofty |
| Library | SQLite (rusqlite) |
| Media controls | souvlaki |
| Tray (Linux) | ksni |
| Cloud | rust-s3 |

## Credits

- [PixelPlayer](https://github.com/PixelPlayerHQ/PixelPlayer) by Theo Vilardo, for the inspiration.
- [LRCLIB](https://lrclib.net), for the free lyrics API.
