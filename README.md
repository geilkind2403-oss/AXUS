# AXUS

A lightweight TV launcher for Raspberry Pi and LibreELEC. The current launcher is written in Rust and is designed for remote control, direct display output and low idle resource use.

## Project layout

- `rust/` — Rust launcher and the PC film server. See [rust/README.md](rust/README.md) for build, install and feature details.
- `browser/` — Buildroot configuration for the optional YouTube TV browser runtime.

The browser runtime is built separately from the launcher so the Pi can run the home screen without keeping a browser open.

Version codenames are a personal tribute and imply no affiliation with or endorsement by the people named.
