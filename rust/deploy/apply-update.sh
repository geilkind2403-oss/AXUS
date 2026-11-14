#!/bin/sh
# Runs outside tv-launcher's service group so it can stop and replace it.
set -eu

DIR=/storage/tv-launcher
CURRENT="$DIR/tv-launcher"
STAGED="$DIR/tv-launcher.new"
BACKUP="$DIR/tv-launcher.previous"
VERSION="$DIR/tv-launcher.new.version"

[ -x "$STAGED" ] || { echo "Kein geprüftes Update liegt bereit"; exit 1; }
[ -x "$CURRENT" ] || { echo "Installierter Launcher fehlt"; exit 1; }

systemctl stop tv-launcher.service || true
cp -p "$CURRENT" "$BACKUP.new"
mv -f "$BACKUP.new" "$BACKUP"
mv -f "$STAGED" "$CURRENT"
chmod 755 "$CURRENT"
systemctl start tv-launcher.service || true

# Require five consecutive healthy seconds. A crashing binary is restarted by
# systemd; any restart during this window resets the health counter.
healthy=0
elapsed=0
while [ "$elapsed" -lt 15 ]; do
    sleep 1
    elapsed=$((elapsed + 1))
    if systemctl is-active --quiet tv-launcher.service; then
        healthy=$((healthy + 1))
        if [ "$healthy" -ge 5 ]; then
            echo "Update aktiv: $(cat "$VERSION" 2>/dev/null || echo unbekannt)"
            exit 0
        fi
    else
        healthy=0
    fi
done

echo "Neuer Launcher blieb nicht stabil; rolle zurück"
systemctl stop tv-launcher.service || true
mv -f "$BACKUP" "$CURRENT"
systemctl start tv-launcher.service || true
exit 1
