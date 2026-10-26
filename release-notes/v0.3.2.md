# Neptunus v0.3.2

Film posters stay on your phone, and the cache mark only shows on films that really played.

## New

- 🖼️ **Posters kept offline.** Posters, backdrops and episode pictures in the video catalogs are saved the first time they show, so the catalogs look complete without a connection or when an addon's image host is slow. A film listed by several extensions is saved once and shared between them. The posters stay under 200 MB, the least recently seen going first, and **Settings › Lokal › Hapus cache** clears them with the films.

## Fixed

- ✅ **Cache mark on films that never played.** Picking a source marked the film as cached straight away, even when it failed to play. The mark now waits until the film has really played for a few seconds, and only counts what was actually downloaded.
- 🏷️ **A clearer cache mark.** Cached films, episodes and sources show a small ✓ icon (and a **Di cache** tag on the source) instead of a coloured border.

## Install

| Platform | File |
|---|---|
| Linux | `.deb`, `.rpm` or `.AppImage` |
| Windows | `-setup.exe` or `.msi` |
| macOS | `_aarch64.dmg` (Apple Silicon) or `_x64.dmg` (Intel) |
| Android 8.0+ | `_arm64.apk` |
| iOS | `.ipa` (OTA when available) or `_unsigned.ipa`, sign it with SideStore, iloader or LiveContainer |

Update with **Unduh & pasang** in the update banner or Settings › Tentang.
