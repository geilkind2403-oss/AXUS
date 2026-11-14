#!/bin/sh
# Builds the Pi 3 launcher (static ARM binary) and packs it with the install
# files into dist/tv-launcher-pi.tar.gz. Needs: rustup, clang.
set -e
cd "$(dirname "$0")"
rustup target add armv7-unknown-linux-musleabihf >/dev/null
# which build is running: shown in the log and under Settings > System
TV_BUILD="$(git rev-parse --short HEAD 2>/dev/null || echo dev) $(date -u +%Y-%m-%d\ %H:%M)"
export TV_BUILD
cargo build --release --target armv7-unknown-linux-musleabihf -p tv-launcher
rm -rf dist/pi && mkdir -p dist/pi
cp target/armv7-unknown-linux-musleabihf/release/tv-launcher dist/pi/
cp deploy/install.sh deploy/install-browser.sh deploy/tv-launcher.service deploy/kodi-off.conf dist/pi/
tar -C dist -czf dist/tv-launcher-pi.tar.gz pi
echo "dist/tv-launcher-pi.tar.gz"
