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

Release builds are made on Ubuntu 22.04 so the binaries run on more distros (older glibc). Pushing a `v*` tag builds installers for all three OSes, plus the Android APK and iOS IPA, in CI and attaches them to a draft release.

### Linux notes

- The app runs natively on Wayland by default. Set `NEPTUNUS_X11=1` to run through XWayland instead; the window can sometimes stay unclickable there.
- The tray icon needs a StatusNotifierItem host. On GNOME that is the AppIndicator extension, which Ubuntu ships by default.

## Mobile (Android and iOS)

Each release also has an Android APK (`Neptunus_<version>_arm64.apk`, Android 8.0+) and an unsigned iOS IPA (`Neptunus_<version>_unsigned.ipa`). An IPA can only be built on macOS with Xcode, so it comes from CI.

### Android

Download the APK on the phone and open it (allow installs from that source when asked), or from a computer with USB debugging on:

```bash
adb install -r Neptunus_*_arm64.apk
```

Updates install over the previous version. On first launch, allow access to music and audio so the local library can read the Music folder.

### iOS

The IPA has to be signed before iOS will run it. Pick one way:

**Free Apple ID (re-sign every 7 days)**

Use [SideStore](https://sidestore.io), [iloader](https://github.com/nab138/iloader) or [Sideloader](https://github.com/Dadoum/Sideloader): log in with your Apple ID, pick the IPA, and it is signed and installed for 7 days. Using a separate Apple ID for sideloading is a good idea. A free Apple ID can keep at most 3 apps signed at a time, SideStore included.

To keep the apps from expiring, let SideStore refresh them from the phone:

1. Install SideStore with iloader (it also sets up the pairing file SideStore needs), and the local VPN app SideStore asks for from the App Store (currently LocalDevVPN).
2. Install Neptunus from SideStore (**+**, then pick the IPA).
3. In the Shortcuts app, add an automation (for example daily at 03:00, or when the charger is connected) that runs SideStore's **Refresh All Apps**, set to run immediately.

Each refresh resets the apps to 7 days. If SideStore itself expires, it can't refresh anymore: install it again from a computer with iloader.

**Your own Apple Developer account (valid for a year)**

This needs a paid [Apple Developer Program](https://developer.apple.com/programs/) membership; a free Apple ID has no access to certificates. On Linux, make the key and certificate with `openssl`, and keep them outside the repo:

1. Make a private key and a certificate signing request:

   ```bash
   mkdir -p ~/ios-cert && cd ~/ios-cert
   openssl genrsa -out neptunus.key 2048
   openssl req -new -key neptunus.key -out neptunus.csr -subj "/emailAddress=YOUR_APPLE_ID_EMAIL/CN=Your Name/C=ID"
   ```

2. On [developer.apple.com/account](https://developer.apple.com/account), under **Certificates, Identifiers & Profiles**:
   - **Certificates › +** › **Apple Development**: upload `neptunus.csr` and download the certificate (`development.cer`).
   - **Devices › +**: add your iPhone's UDID (`idevice_id -l` prints it).
   - **Identifiers › + › App IDs › App**: an explicit bundle ID `id.neptunus.desktop`.
   - **Profiles › + › iOS App Development**: pick that App ID, the certificate and your device, then download the `.mobileprovision`.

3. Combine the certificate and key into a `.p12` (it asks for a password, used by `zsign -p` below; add `-legacy` if a Mac or Windows tool rejects the file):

   ```bash
   openssl x509 -inform DER -in development.cer -out development.pem
   openssl pkcs12 -export -inkey neptunus.key -in development.pem -out neptunus.p12
   ```

4. Sign with [zsign](https://github.com/zhlynn/zsign) and install over USB with libimobiledevice. zsign isn't packaged for Debian/Ubuntu; build it from source (`build/linux`, then `make`) and copy `bin/zsign` to `~/.local/bin`.

   ```bash
   sudo apt install libimobiledevice-utils ideviceinstaller
   zsign -k ~/ios-cert/neptunus.p12 -p 'P12_PASSWORD' -m ~/ios-cert/Neptunus_Dev.mobileprovision -o Neptunus_signed.ipa Neptunus_*_unsigned.ipa
   ideviceinstaller install Neptunus_signed.ipa
   ```

Unlock the iPhone and tap **Trust** the first time it is connected. The certificate and profile last a year; after that, make a new profile, sign again and reinstall. To add another device, register its UDID and download the profile again. Back up `neptunus.key` and `neptunus.p12`: losing them means making a new certificate.

Only sign with a certificate you own: an enterprise certificate is for a company's internal apps, and misusing one gets it revoked for everyone using it.

SideStore shows how many days an app has left under *My Apps*.

**Local library on iOS:** iOS apps can only read their own folder. Copy songs with the Files app to *On My iPhone › Neptunus*, then add that folder in Neptunus (Settings › Lokal › Tambah folder).

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
