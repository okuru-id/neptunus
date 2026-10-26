# Neptunus v0.3.5

Films from addons that need their own request headers play now, and the film cache can be tidied one film at a time.

## Fixed

- ▶️ **Sources that kept loading.** Many addons give links that only answer with the headers they name (a Referer, an Origin or a User-Agent), and Stremio sends those. Neptunus didn't, so those links never started. They are sent now, also when you resume from **Lanjutkan**.
- 📏 **Cache size counted right.** A film downloaded only in part counted as its whole length, so older films were cleared from the cache too early and Settings showed more than was used. Both now count what really came down.

## New

- 🗂️ **Cached films list.** **Settings › Lokal › Cache film** lists the films in the cache with their size, how much came down and when you last played them. Tick the ones to remove, or pick them all. Films cached before this version show as "Tanpa judul".

This release replaces v0.3.4, which had the same changes but still called itself 0.3.3.

## Install

| Platform | File |
|---|---|
| Linux | `.deb`, `.rpm` or `.AppImage` |
| Windows | `-setup.exe` or `.msi` |
| macOS | `_aarch64.dmg` (Apple Silicon) or `_x64.dmg` (Intel) |
| Android 8.0+ | `_arm64.apk` |
| iOS | `.ipa` (OTA when available) or `_unsigned.ipa`, sign it with SideStore, iloader or LiveContainer |

Update with **Unduh & pasang** in the update banner or Settings › Tentang.
