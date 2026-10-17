# Product

<!-- impeccable:product-schema 1 -->

## Platform

web

## Users

People who keep their own music collection (local folders, optionally an S3-compatible bucket) and listen on a Linux, Windows or macOS desktop, often for long sessions in the background while working.

## Product Purpose

Neptunus is a lightweight desktop music player: scan a folder, browse by song, album, artist and playlist, and play with gapless transitions, synced lyrics and OS media controls. Success means a large personal library feels fast to browse and pleasant to live with.

## Positioning

Desktop sibling of PixelPlayer for Android. It carries over that app's Material 3 Expressive identity (dynamic colour from the playing album art, expressive shapes) to a native-sized desktop app (Tauri, no bundled browser runtime).

## Operating Context

- Listening sessions where the window is glanced at, not stared at; the player bar is the most used surface.
- Libraries range from dozens to thousands of tracks; cover art quality varies and many tracks have none.
- Lyrics come from `.lrc` sidecars or embedded tags; many tracks have no lyrics.

## Capabilities and Constraints

- Tauri 2 + Vue 3 (Composition API, `<script setup>`, Vite; two entries: `index.html` main window, `mini.html` tray mini player). Rust backend: rodio playback, lofty tags, SQLite library, souvlaki media controls, rust-s3 mirror. System tray: on Linux a StatusNotifierItem via ksni (transparent icon; a click opens the menu, a left double click opens the mini player under the icon, since GNOME's AppIndicator extension only forwards double clicks; app runs through XWayland so windows can be positioned, `NEPTUNUS_WAYLAND=1` opts out; no libayatana), Tauri tray on Windows/macOS (mini player above the tray icon).
- Features: folder scan, library persisted, songs/albums/artists/playlists views, search, queue with shuffle/repeat, gapless, Now Playing with synced lyrics, S3 sync to local cache.
- UI copy is Indonesian (hardcoded, no i18n yet).
- Undecided: crossfade, streaming S3 without full download, light theme preference default (follows OS).

## Brand Commitments

- Name: Neptunus (neptunus.id). Formerly PixelPlay.
- Visual language: Material 3 Expressive, matching PixelPlayer Android (user-confirmed).

## Evidence on Hand

No logo or brand assets in this repo yet. No screenshots, testimonials or metrics; none may be invented.

## Product Principles

1. The music leads: cover art and its colour drive the interface.
2. Playback controls are always one glance away.
3. Fast on large libraries over decoration.
4. Feel like the same family as PixelPlayer Android.
