#!/bin/sh
# Kconfig silently drops symbols it doesn't know: fail early if anything we
# need didn't make it into .config (a renamed option in a newer Buildroot).
set -e
cfg="$1/.config"
missing=0
for sym in $(grep -E '^BR2_[A-Z0-9_]+=y' "$(dirname "$0")/br2-external/configs/tvbrowser_defconfig" | cut -d= -f1); do
  if ! grep -q "^$sym=y" "$cfg"; then
    echo "MISSING in .config: $sym"
    missing=1
  fi
done
# and the ones that must be OFF: they must exist in Buildroot ("is not set"
# in .config) -- a misspelled name is silently ignored and leaves the default on
for sym in $(grep -E '^# BR2_[A-Z0-9_]+ is not set' "$(dirname "$0")/br2-external/configs/tvbrowser_defconfig" | awk '{print $2}'); do
  if grep -q "^$sym=y" "$cfg"; then
    echo "STILL ON in .config: $sym"
    missing=1
  elif ! grep -q "^# $sym is not set" "$cfg" && ! grep -rqs "config ${sym#BR2_}\$" "$1/package" "$1/Config.in" "$1/system" "$1/fs" "$1/linux" "$1/toolchain" 2>/dev/null; then
    echo "UNKNOWN option (typo?): $sym"
    missing=1
  fi
done
[ "$missing" = 0 ] && echo "config ok"
exit $missing
