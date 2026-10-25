# Neptunus v0.3.3

Films pick up where you left them, and come back with their picture after the phone was locked.

## Fixed

- ⏯️ **Resume from the wrong place.** A film sometimes started again from the beginning, and lost the place you had reached. The place was saved once more right as the film opened, before it had jumped there, and iPhones sometimes skip a jump made that early. Neptunus now waits until the film is really at your place before saving anything, and tries the jump again when it was skipped.
- 🔁 **Finished films start over.** A film you watched into the credits (over 95%, or the last 10 seconds) starts from the beginning when you play it again, and shows as watched in **Lanjutkan menonton**.
- 🍎 **Black picture after locking the iPhone.** Pausing a film, locking the screen and playing again could leave the picture black while the sound went on. Coming back to the app now redraws the picture.

## Install

| Platform | File |
|---|---|
| Linux | `.deb`, `.rpm` or `.AppImage` |
| Windows | `-setup.exe` or `.msi` |
| macOS | `_aarch64.dmg` (Apple Silicon) or `_x64.dmg` (Intel) |
| Android 8.0+ | `_arm64.apk` |
| iOS | `.ipa` (OTA when available) or `_unsigned.ipa`, sign it with SideStore, iloader or LiveContainer |

Update with **Unduh & pasang** in the update banner or Settings › Tentang.
