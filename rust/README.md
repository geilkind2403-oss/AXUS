# TV Launcher – Rust version (Raspberry Pi 3 + LibreELEC)

The Electron launcher (`src/`, `server/`) rewritten in Rust, for a Raspberry Pi 3
running LibreELEC **without Kodi**. Two programs:

| Program | Runs on | Replaces |
|---|---|---|
| `tv-launcher` | the Pi (one static 7 MB binary, no libraries needed) | the Electron app |
| `tv-server` | the PC (Windows / Linux) | `server/mediaserver.js` + `server/musicserver.js` (music: set `MUSIC_PORT=0` to switch it off) |

## Why it is fast on a Pi 3

- **No browser in the launcher by default, no Kodi, no X11/Wayland.** The launcher draws
  straight to the TV through Linux KMS (`/dev/dri`) and reads the remote through evdev /
  the kernel's CEC API. An optional YouTube TV browser can be installed separately (see
  “Optional YouTube TV app” below); it does not run in the background.
- **Hardware video.** H.264 is decoded by the Pi's VideoCore through V4L2
  (`bcm2835-codec`, the same decoder Kodi uses) and the decoded frames go to a display
  plane as DMABUF: **zero copies** of the picture through the CPU.
- **Hardware scaling.** The UI is drawn at 1280×720 and the display controller scales it
  to the TV's 1080p mode. While a film plays, the UI is an overlay plane that is switched
  **off** when nothing is shown (no OSD, no subtitle), so the CPU is idle.
- **Idle means idle.** The UI redraws only when something changes (plus the clock once a
  second). Images are decoded once at the size they are shown, with a RAM budget.
- The heavy jobs the Pi 3 can't do (HEVC, AC3/DTS sound, MKV over the network) are done
  by `tv-server` on the PC, which converts while streaming (`/play/<id>?v=h264`).

## What's in it

Home screen · **Live TV** (M3U channel lists from a file or URL, HLS played on the Pi's
decoder, zapping, channel numbers, guide with country groups, favourites) · Films (USB drive / `/storage/videos` on the Pi **and** the PC server, resume
positions, audio track and subtitle choice, MKV subtitles) · YouTube (search, results,
playback in up to 1080p H.264, no browser) · Weather (Open-Meteo) · News
(Tagesschau, full articles) · Settings (WLAN, Bluetooth, PC server, sound output,
volume, phone remote with QR code, place for the weather, Live TV list and quality,
screensaver, sleep timer that also switches the TV off, subtitle size, restart / power
off / back to Kodi).

Controls: the **TV remote via HDMI-CEC**, any USB/Bluetooth keyboard or remote, an IR
receiver, or the **phone** (`http://<pi-ip>:8080`, shown with a QR code under
Einstellungen › Handy als Fernbedienung – it can also type text, e.g. for YouTube search).

## Install on the Pi

1. Build (or download the `tv-launcher-pi` artifact from the GitHub Actions run):
   ```bash
   cd rust && ./build-pi.sh
   ```
   This creates `dist/tv-launcher-pi.tar.gz` (needs `rustup` and `clang`).
2. In LibreELEC: *Settings › LibreELEC › Services › SSH* switch on and use the SSH
   credentials configured on that device.
3. Copy it over and install:
   ```bash
   scp rust/dist/tv-launcher-pi.tar.gz root@<pi-ip>:/storage/
   ```
   ```bash
   ssh root@<pi-ip> "cd /storage && tar xzf tv-launcher-pi.tar.gz && sh pi/install.sh"
   ```
   The launcher starts at once and after every boot; Kodi stays off.
4. Films: plug in a USB drive (anything under `/var/media/…` is scanned) or copy to
   `/storage/videos`.

Back to Kodi: Einstellungen › System › *Zu Kodi wechseln*, or `sh pi/install.sh --uninstall`.
Log: `journalctl -u tv-launcher -f`. Data and settings: `/storage/tv-launcher/data`.

### Software updates

Open **Einstellungen › System › Software-Update suchen**. The launcher checks the
latest GitHub Release, downloads `tv-launcher-update.tar.gz`, and verifies its SHA-256
before installation. The update service keeps the previous binary and restores it if
the new launcher does not stay healthy.

The repository is private, so add a fine-grained GitHub token with **Contents: Read-only**
access to this repository on the Pi. Put it in `/storage/tv-launcher/env` as
`TV_UPDATE_TOKEN=github_pat_…` and restart the launcher. Keep this file private. Each
`v*` tag builds and publishes a release automatically; the release includes the update
archive and a full Pi installer. The updater must first be installed once using the Pi
installer package, since older builds do not include the GitHub updater.

## Optional YouTube TV app

The YouTube tile uses the launcher's built-in YouTube search and hardware video player by
default. To open the real YouTube TV web app instead, install the optional WPE WebKit + cog
bundle. This browser is built separately for 32-bit ARM because it is much larger than the
launcher; the browser root filesystem is not included in the normal launcher package.

1. In GitHub Actions, run the **YouTube TV browser** workflow and download its
   `tv-browser-pi` artifact. It contains `tv-browser-rootfs.tar.gz`.
2. Put that archive next to `install-browser.sh` on the Pi and run:
   ```sh
   sh install-browser.sh
   ```
   It needs about 600 MB free on `/storage` for installation.
3. Return to the launcher. Selecting YouTube now opens `https://www.youtube.com/tv`.
   Home on the remote or phone closes the browser and returns to the launcher. TV-remote
   keys and phone input are forwarded to the browser; USB/Bluetooth keyboards connect to
   it directly.

The launcher gives the browser DRM/KMS display ownership while it runs. Its UI is hidden
and the main loop sleeps between checks, so the launcher uses very little CPU but remains
available to catch Home and restore the display. The browser exits when you return Home;
it is not a general-purpose browser and is only launched at the YouTube TV URL. Remove the
bundle with `sh install-browser.sh --uninstall`; the tile then returns to the built-in
YouTube experience. The Pi 3's memory and YouTube's changing web app can still limit
playback, so the browser workflow needs to be built and tried on the target Pi.

**Films on the Pi itself** must be H.264 (most `.mp4`/`.mkv` files; AAC or MP3 sound). HEVC/x265
files, or AC3/DTS sound, play through the PC server instead (it converts them).

## The PC server

```bash
cd rust && cargo build --release -p tv-server
```

Put `tv-server.exe` anywhere, next to it a `tv-server.conf`:

```
MEDIA_DIR=D:\Filme
MUSIC_DIR=D:\Musik
```

and `ffmpeg.exe` + `ffprobe.exe` (or have them in PATH). Start it; it prints the
addresses to enter on the Pi (Einstellungen › PC-Server). Same API and the same film
ids as the old Node servers, so saved resume positions carry over and the old Electron
launcher works with it too.

## Development

```bash
cd rust
cargo test --workspace            # demuxers are checked against files made by ffmpeg
SHOT_SAMPLE=1 cargo run -p tv-launcher -- --shot /tmp/shots   # every screen as PNG
```

Layout: `tvcore/` (library scanning shared by both), `tv-server/`, `tv-launcher/src/`
(`platform/` = KMS, evdev, CEC, ALSA, connman/BlueZ · `player/` = MP4/MKV demux, V4L2
decoder, audio, A/V sync · `ui/` = screens · `youtube.rs`, `weather.rs`, `news.rs`).

## Not tested on real hardware yet

Everything that only exists on the Pi – KMS planes, the V4L2 decoder, ALSA, CEC – is
written against the kernel interfaces and compiles, but could not be run here. The
demuxers, audio decoding, UI, server and the ARM build itself are tested (the ARM binary
was run under emulation). Expect a round of fixes after the first start on the Pi:
`journalctl -u tv-launcher` shows what happened.

Music is left out of the Pi launcher for now (the PC server still serves music for the
old Electron app; `tvcore` keeps the music scanner for when it comes back).

YouTube uses YouTube's internal app API (like Kodi's YouTube add-on and SmartTube do);
when YouTube changes it, `src/youtube.rs` needs an update.

Live TV: put your `.m3u` into `/storage/tv-launcher/livetv/` (or on the USB drive), or enter
a URL under Einstellungen › Live TV. Without either, the public German list of iptv-org is
used. Encrypted streams and fMP4-HLS are not supported yet.
