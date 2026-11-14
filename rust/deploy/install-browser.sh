#!/bin/sh
# Installs the YouTube TV browser (WPE WebKit + cog) for the TV launcher.
# Run ON THE PI, next to tv-browser-rootfs.tar.gz (from the GitHub Actions
# run "YouTube TV browser"):   sh install-browser.sh
# Afterwards the YouTube tile opens the real YouTube TV app.
# Remove: sh install-browser.sh --uninstall
set -e
DIR=/storage/tv-launcher/browser
HERE=$(cd "$(dirname "$0")" && pwd)

unmount_all() {
  for m in $(awk '{print $2}' /proc/mounts | grep "^$DIR/" | sort -r); do umount -l "$m" 2>/dev/null || true; done
}

if [ "$1" = "--uninstall" ]; then
  unmount_all
  rm -rf "$DIR"
  echo "Browser entfernt; die YouTube-Kachel nimmt wieder das eingebaute YouTube."
  exit 0
fi

TAR="$HERE/tv-browser-rootfs.tar.gz"
[ -f "$TAR" ] || { echo "tv-browser-rootfs.tar.gz fehlt neben install-browser.sh"; exit 1; }
FREE=$(df -m /storage | awk 'NR==2 {print $4}')
[ "$FREE" -gt 600 ] || { echo "Zu wenig Platz auf /storage ($FREE MB frei, ~600 MB nötig)"; exit 1; }
unmount_all
rm -rf "$DIR.new"
mkdir -p "$DIR.new"
echo "Entpacke (dauert eine Minute) ..."
tar xzf "$TAR" -C "$DIR.new" --exclude='./dev/*' 2>/dev/null || tar xzf "$TAR" -C "$DIR.new"
rm -rf "$DIR"
mv "$DIR.new" "$DIR"
du -sh "$DIR"
echo "Fertig. Die YouTube-Kachel startet jetzt die echte YouTube-TV-App."
echo "Log des Browsers: /storage/tv-launcher/data/browser.log"
