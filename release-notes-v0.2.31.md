# Neptunus v0.2.31

The app stays smooth on phones after opening and closing films.

## Fixed

- 📱 **Lag after opening and closing films.** Each film closed left its video decoder and buffers behind until the phone got round to freeing them, so after a few films the whole app slowed down. Closing a film now lets go of all of it at once, and a film closed before it started no longer keeps downloading in the background. Where you stopped is still remembered.
- 🌊 **Less work while music is paused.** The wavy progress bar kept redrawing every frame even while paused. It now rests once the music stops, which also helps while a film plays.

## Install

| Platform | File |
|---|---|
| Linux | `.deb`, `.rpm` or `.AppImage` |
| Windows | `-setup.exe` or `.msi` |
| macOS | `_aarch64.dmg` (Apple Silicon) or `_x64.dmg` (Intel) |
| Android 8.0+ | `_arm64.apk` |
| iOS | `.ipa` (OTA when available) or `_unsigned.ipa`, sign it with SideStore, iloader or LiveContainer |

Update with **Unduh & pasang** in the update banner or Settings › Tentang.
