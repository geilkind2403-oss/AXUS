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
rm -rf dist/pi dist/update && mkdir -p dist/pi dist/update
cp target/armv7-unknown-linux-musleabihf/release/tv-launcher dist/pi/
cp deploy/install.sh deploy/install-browser.sh deploy/tv-launcher.service deploy/tv-launcher-update.service deploy/apply-update.sh deploy/kodi-off.conf dist/pi/
tar -C dist -czf "$PWD/dist/tv-launcher-pi.tar.gz" pi
cp target/armv7-unknown-linux-musleabihf/release/tv-launcher dist/update/tv-launcher
printf '%s\n' "$TV_BUILD" > dist/update/TV_BUILD
(cd dist/update && sha256sum tv-launcher > tv-launcher.sha256)
tar -C dist/update -czf "$PWD/dist/tv-launcher-update.tar.gz" tv-launcher TV_BUILD tv-launcher.sha256
echo "dist/tv-launcher-pi.tar.gz"
echo "dist/tv-launcher-update.tar.gz"
