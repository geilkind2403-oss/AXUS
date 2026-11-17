# AXUS

### **A**ccelerated e**X**perience for **U**r **S**creen

A TV launcher for the Raspberry Pi that starts fast, draws straight to the screen and stays
quiet when nothing is happening. Remote first, browser optional, no desktop.

```
   ┌──────────────────────────────────────────┐
   │  A   Accelerated   starts in seconds     │
   │  X   eXperience    the interface itself  │
   │  U   Ur            your                  │
   │  S   Screen        the TV                │
   └──────────────────────────────────────────┘
```

---

## What it is

AXUS boots a Raspberry Pi 3 running LibreELEC straight into a TV interface instead of Kodi:
home screen, live TV, films, YouTube, weather, news and settings — all steered with the TV
remote, an IR remote, a keyboard, or your phone.

It is one static 7 MB Rust binary that talks to the hardware directly: no X11, no Wayland,
no compositor, no browser running in the background. The picture goes out through Linux KMS
and the remote is read through evdev and the kernel's HDMI-CEC API, so the Pi spends almost
all of its time idle.

**Accelerated** is meant literally:

- **Hardware video** — H.264 is decoded by the Pi's VideoCore through V4L2 (`bcm2835-codec`,
  the same decoder Kodi uses) and the frames go to a display plane as DMABUF: the picture
  never passes through the CPU.
- **Full-HD interface** — the UI is laid out in 1280×720 units and drawn at the TV's full
  1920×1080, redrawing only the parts of the screen that change.
- **Idle means idle** — the UI redraws only when something changes (plus the clock, once a
  second). While a film plays, the UI plane is switched off entirely.

---

## What's in it

| | |
|---|---|
| **Home** | Tiles for every app, big and readable from the sofa |
| **Live TV** | M3U lists from a file or URL, HLS on the Pi's decoder, channel numbers, zapping, guide with country groups, favourites |
| **Films** | USB drive, `/storage/videos` on the Pi, or the PC server — resume positions, audio track and subtitle choice, MKV subtitles |
| **YouTube** | Search, results and playback up to 1080p H.264 without a browser |
| **Weather** | Live conditions from Open-Meteo with a hand-written animated sky |
| **News** | Tagesschau, full articles |
| **Settings** | WLAN, Bluetooth, PC server, sound output, volume, phone remote with QR code, weather position, Live TV list and quality, screensaver, sleep timer that also switches the TV off, subtitle size, software update and a picker for any released version, restart / power off / back to Kodi |

### Controls

The **TV remote over HDMI-CEC**, any USB or Bluetooth keyboard or remote, an IR receiver, or
a **phone** — open `http://<pi-ip>:8080` on the same network (the QR code is under
*Einstellungen › Handy als Fernbedienung*). The phone can also type text, e.g. for a YouTube
search or a password box.

> The phone remote has no login yet — anyone who can reach port 8080 can drive the TV.
> Keep it on your own network, and switch it off under *Einstellungen* if you don't want it
> listening.

---

## Install (short version)

1. Download `tv-launcher-pi.tar.gz` from the newest
   [release](https://github.com/geilkind2403-oss/AXUS/releases/latest)
   (or build it yourself: `cd rust && ./build-pi.sh`, needs `rustup` and `clang`).
2. On the Pi, enable SSH (*Settings › LibreELEC › Services › SSH*), then:
   ```bash
   scp tv-launcher-pi.tar.gz root@<pi-ip>:/storage/
   ssh root@<pi-ip> "cd /storage && tar xzf tv-launcher-pi.tar.gz && sh pi/install.sh"
   ```
3. The launcher starts immediately and on every boot; Kodi stays off.

Back to Kodi: *Einstellungen › System › Zu Kodi wechseln*, or `sh pi/install.sh --uninstall`.
Logs: `journalctl -u tv-launcher -f`. Settings and data: `/storage/tv-launcher/data`.

Full details — PC film server, software updates, the optional YouTube TV browser, 720p mode
and the development workflow — are in **[rust/README.md](rust/README.md)**.

Want the whole system instead of LibreELEC? **[AXOS](https://github.com/geilkind2403-oss/AXOS)**
is a small Linux for the Pi 3 that boots straight into AXUS (in development).

---

## Project layout

```
rust/
  tv-launcher/    the TV interface   (KMS, evdev, CEC, ALSA, connman/BlueZ)
    player/       MP4/MKV/TS demuxers, V4L2 decoder, audio, A/V sync
    ui/           screens
  tv-server/      film server for the PC (Windows / Linux), converts while streaming
  tvcore/         library scanning shared by both
  deploy/         install scripts and systemd units
browser/          Buildroot config for the optional YouTube TV browser runtime
```

The browser runtime is built separately from the launcher for 32-bit ARM and installed as its
own bundle, so the Pi can run the home screen without ever keeping a browser open.

**Films on the Pi itself** must be H.264 (most `.mp4`/`.mkv`, AAC or MP3 sound). HEVC/x265, or
AC3/DTS sound, play through the PC server instead — it converts while streaming.

---

## Releases and version names

Every `v*` tag builds and publishes a release with the update archive (`tv-launcher-update.tar.gz`)
and a full Pi installer. The launcher can update itself from **Einstellungen › System ›
Software-Update suchen**, keeps the previous binary to roll back if a new build doesn't
stay healthy, and can install any older release under **Version wählen**.

Each 0.0.x line carries a codename. They are a personal tribute and imply no affiliation with
or endorsement by the people named.

---

## Credits

There is no open-source license yet, so all rights are reserved for now.

Fonts: Inter (SIL Open Font License, see `rust/tv-launcher/assets/fonts/OFL.txt`).
Weather data: [Open-Meteo](https://open-meteo.com). Live TV default list:
[iptv-org](https://github.com/iptv-org/iptv). AXUS is not affiliated with YouTube, ARD, ZDF,
ARTE or Prime Video; those tiles simply open (or stream) the public services.
