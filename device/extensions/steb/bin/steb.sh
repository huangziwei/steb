#!/bin/sh
# bin/steb.sh — runs $EXT/bin/steb for documents/Steb.sh and for the
# menu.json entry.

EXT=/mnt/us/extensions/steb
LOG=/mnt/us/logs/steb.log

if pidof steb >/dev/null 2>&1; then
    exit 0
fi

# STEB_ORIGIN_VIEW is set by documents/Steb.sh, unset from menu.json.
restore_view_on_exit() {
    case "${STEB_ORIGIN_VIEW:-}" in
        KPP_*|LEGACY_*)
            lipc-set-prop com.lab126.appmgrd startView \
                "$STEB_ORIGIN_VIEW:0:app://com.lab126.KPPMainApp?view=$STEB_ORIGIN_VIEW" \
                2>/dev/null
            ;;
    esac
}
trap restore_view_on_exit EXIT

# `dirname $LOG` for the `>>` redirects below.
mkdir -p "$(dirname "$LOG")"

echo "[$(date)] launch $(uname -m)" >> "$LOG"
"$EXT/bin/steb" 2>> "$LOG"
# `$(date)` below overwrites `$?` in some shells.
STATUS=$?
echo "[$(date)] exit=$STATUS" >> "$LOG"
