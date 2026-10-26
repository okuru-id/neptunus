# Neptunus v0.2.28

Small fixes around the video and lyrics screens.

## Fixed

- 🔄 **Reload button on phones.** The **Muat ulang katalog** button in the video catalog showed as an empty circle on narrow screens. Its icon is back.
- 🔔 **Notifications over a playing film.** The bell's list opened behind the video player. It now shows on top of it.
- ⚙️ **Settings from the lyrics screen.** Tapping settings while the lyrics screen was open changed the page behind it, so nothing seemed to happen. The lyrics screen now closes and Settings shows.
- 🎬 **A film no longer disappears when settings is tapped.** Opening Settings while a film played closed the player. The settings button is now disabled while a film is open; close the film first.

## Changed

- 🔍 **No search on a film's page.** The search field hides while a film's detail page is open and comes back in the catalog.
- 📝 **Release notes on the release page.** Releases now carry their notes from the start.

## Install

| Platform | File |
|---|---|
| Linux | `.deb`, `.rpm` or `.AppImage` |
| Windows | `-setup.exe` or `.msi` |
| macOS | `_aarch64.dmg` (Apple Silicon) or `_x64.dmg` (Intel) |
| Android 8.0+ | `_arm64.apk` |
| iOS | `.ipa` (OTA when available) or `_unsigned.ipa`, sign it with SideStore, iloader or LiveContainer |

Update with **Unduh & pasang** in the update banner or Settings › Tentang.
