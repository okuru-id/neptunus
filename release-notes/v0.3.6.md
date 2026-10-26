# Neptunus v0.3.6

Film sources show up sooner, in the order the extensions give them, and episodes can be browsed with arrows on a computer.

## Changed

- ⚡ **Sources show as they come.** Each extension's sources appear as soon as it answers, with "N ekstensi lagi…" while the slower ones are still looked up. Before, the list waited for the slowest extension, which on a phone's connection could take a while.
- 🧾 **Sources in the extension's own order.** The list is no longer re-sorted by guessed quality; it shows what the extension sends, as it sends it. The extension tabs keep a steady order, with the ones that found nothing last.
- 🏎️ **A lighter source list.** Sources no longer wait behind catalog rows still loading from the same extension, and each row's badges are worked out once instead of on every redraw.

## New

- ⬅️➡️ **Episode arrows.** On a computer, two arrows beside **Musim** scroll the episode strip, which a mouse couldn't swipe.

## Install

| Platform | File |
|---|---|
| Linux | `.deb`, `.rpm` or `.AppImage` |
| Windows | `-setup.exe` or `.msi` |
| macOS | `_aarch64.dmg` (Apple Silicon) or `_x64.dmg` (Intel) |
| Android 8.0+ | `_arm64.apk` |
| iOS | `.ipa` (OTA when available) or `_unsigned.ipa`, sign it with SideStore, iloader or LiveContainer |

Update with **Unduh & pasang** in the update banner or Settings › Tentang.
