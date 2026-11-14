#!/bin/sh
# Installs the TV Launcher on a Raspberry Pi 3 with LibreELEC, instead of Kodi.
# Run ON THE PI (SSH in as root, see README), from the folder with the files:
#   sh install.sh
# Undo: sh install.sh --uninstall   (Kodi comes back after a reboot)
set -e
DIR=/storage/tv-launcher
UNITS=/storage/.config/system.d
FLAG=/storage/.config/tv-launcher.enabled
HERE=$(cd "$(dirname "$0")" && pwd)

if [ "$1" = "--uninstall" ]; then
  rm -f "$FLAG"
  systemctl disable tv-launcher.service 2>/dev/null || true
  systemctl stop tv-launcher.service 2>/dev/null || true
  rm -f "$UNITS/tv-launcher.service"
  for s in kodi pulseaudio eventlircd; do rm -f "$UNITS/$s.service.d/tv-launcher.conf"; done
  systemctl daemon-reload
  systemctl start pulseaudio eventlircd kodi
  echo "Kodi ist zurück. Die Daten bleiben in $DIR (löschen mit: rm -rf $DIR)."
  exit 0
fi

[ -f "$HERE/tv-launcher" ] || { echo "tv-launcher (die Pi-Datei) fehlt neben install.sh"; exit 1; }
mkdir -p "$DIR/data" /storage/videos
cp "$HERE/tv-launcher" "$DIR/tv-launcher.new" && chmod 755 "$DIR/tv-launcher.new" && mv "$DIR/tv-launcher.new" "$DIR/tv-launcher"
cp "$HERE/tv-launcher.service" "$UNITS/tv-launcher.service"
for s in kodi pulseaudio eventlircd; do
  mkdir -p "$UNITS/$s.service.d"
  cp "$HERE/kodi-off.conf" "$UNITS/$s.service.d/tv-launcher.conf"
done
# Time zone for the clock (POSIX rule: Germany). Change it here if needed.
[ -f "$DIR/env" ] || echo 'TZ=CET-1CEST,M3.5.0,M10.5.0/3' > "$DIR/env"
touch "$FLAG"
systemctl daemon-reload
systemctl enable tv-launcher.service
systemctl stop kodi pulseaudio eventlircd 2>/dev/null || true
systemctl restart tv-launcher.service
sleep 2
systemctl --no-pager status tv-launcher.service | head -5 || true
echo
echo "Fertig. Der TV Launcher läuft jetzt statt Kodi (auch nach jedem Neustart)."
echo "Filme: USB-Stick einstecken oder nach /storage/videos kopieren."
echo "Log ansehen: journalctl -u tv-launcher -f"
