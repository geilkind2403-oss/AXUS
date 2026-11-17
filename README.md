# AXUS

A lightweight TV launcher for Raspberry Pi and LibreELEC. The current launcher is written in Rust and is designed for remote control, direct display output and low idle resource use.

## Project layout

- `rust/` — Rust launcher and PC media server. See [rust/README.md](rust/README.md) for build, install and feature details.
- `browser/` — Buildroot configuration for the optional YouTube TV browser runtime.
- `server/` — the media servers from the earlier Electron launcher.

The browser runtime is built separately from the launcher so the Pi can run the home screen without keeping a browser open.
