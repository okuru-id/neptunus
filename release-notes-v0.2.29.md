# Neptunus v0.2.29

You can now see and manage the film cache, cached films stand out in the catalog, and playing films on a phone works better.

## New

- 💾 **Film cache in Settings › Lokal.** A new **Cache film** panel shows how much space online films take for buffering. **Hapus cache** empties it, and **Simpan selama** sets how long an unplayed film stays: 1, 3, 7 (default), 14 or 30 days. The cache still never grows past 10 GB.
- 🟣 **Cached films are marked.** In the extension catalog, films and episodes with a stream already in the cache get a coloured border and a download badge. In the source list, the cached source has a border too.
- 🖼️ **Sound but no picture.** When a film plays its audio but its video can't be decoded on this device (often HEVC/AV1 10-bit), the player now says so after a few seconds and suggests trying another source, such as an H.264/x264 one.

## Fixed

- 📱 **Tapping a playing film.** On phones and tablets, a tap no longer pauses the film: it shows the player controls, and another tap hides them. Pause with the ⏯ button. With a mouse, a click still pauses.
- 📱 **Buffer bar on phones.** The buffer bar no longer flickers between short and long while a film plays.
- 🍎 **Local videos on iOS.** `.mp4` files in your library didn't play on iPhone and iPad. They now play through the same local server as on the desktop, seeking included.
- 🍎 **iOS folders after a reinstall.** Reinstalling or updating the app on iOS moves its storage, so library folders had to be removed and added again. Folders, songs, playlists and downloads now follow along on their own.
- ✅ **Verification label.** After finishing an extension verification on a phone, Settings › Ekstensi now shows **Terverifikasi** and **Verifikasi ulang** right away.
- 💬 **Unsupported formats** now suggest trying another source instead of VLC.

## Install

| Platform | File |
|---|---|
| Linux | `.deb`, `.rpm` or `.AppImage` |
| Windows | `-setup.exe` or `.msi` |
| macOS | `_aarch64.dmg` (Apple Silicon) or `_x64.dmg` (Intel) |
| Android 8.0+ | `_arm64.apk` |
| iOS | `.ipa` (OTA when available) or `_unsigned.ipa`, sign it with SideStore, iloader or LiveContainer |

Update with **Unduh & pasang** in the update banner or Settings › Tentang.
