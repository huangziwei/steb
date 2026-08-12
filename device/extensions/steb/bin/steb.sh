#!/bin/sh
# The single front door. Both the home-screen tile (documents/Steb.sh) and the
# KUAL menu entry run this, so launch logging stays in one place rather than
# being duplicated — and a fix to either path is a fix to both.
#
# Captures stderr to a sibling log so a non-zero exit is never silent: on a
# device with no shell, that log is the only account of what happened.
EXT=/mnt/us/extensions/steb
LOG=/mnt/us/steb.log

echo "[$(date)] launch $(uname -m)" >> "$LOG"

# The download directory is created by the scriptlet's on_install hook, but a
# KUAL-only install never runs that hook — and the first download would then
# race a missing directory. Cheap to make sure here too.
mkdir -p /mnt/us/documents/standardebooks

# No chmod: the user partition is FAT, which has no mode bits, and exec already
# works there.
"$EXT/bin/steb" "$@" 2>> "$LOG"
echo "[$(date)] exit=$?" >> "$LOG"
