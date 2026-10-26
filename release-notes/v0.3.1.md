# Neptunus v0.3.1

The film catalogs open faster and stay smooth on phones.

## Fixed

- 🎬 **Lag when opening the film catalogs.** Every catalog row built all of its 50 to 100 posters at once, so a page with a few rows put hundreds of cards on screen while a phone shows three per row. Rows now start with a handful and add more as you swipe along them, and rows further down wait until you scroll near them. The catalogs show up about twice as fast and the app stays responsive while they load.

## Install

| Platform | File |
|---|---|
| Linux | `.deb`, `.rpm` or `.AppImage` |
| Windows | `-setup.exe` or `.msi` |
| macOS | `_aarch64.dmg` (Apple Silicon) or `_x64.dmg` (Intel) |
| Android 8.0+ | `_arm64.apk` |
| iOS | `.ipa` (OTA when available) or `_unsigned.ipa`, sign it with SideStore, iloader or LiveContainer |

Update with **Unduh & pasang** in the update banner or Settings › Tentang.
