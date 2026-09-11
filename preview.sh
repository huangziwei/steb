#!/bin/sh
# ./preview.sh [args…] — every screen to a PNG under artifacts/preview, with no
# Kindle behind it. Arguments pass straight through to the `preview` binary:
#
#   ./preview.sh --list
#   ./preview.sh --shot grid:full --panel basic --panel koa2
#
# $FONTS is the one per-machine path this needs: a copy of the Kindle's
# /usr/java/lib/fonts, named in preview.env beside this script (gitignored).
set -eu

ROOT="$(cd "$(dirname "$0")" && pwd)"
ENV_FILE="$ROOT/preview.env"

if [ -f "$ENV_FILE" ]; then
    # shellcheck disable=SC1090
    . "$ENV_FILE"
    export STEB_FONTS
fi

if [ -z "${STEB_FONTS:-}" ]; then
    echo "error: STEB_FONTS is unset" >&2
    echo "       write it into $ENV_FILE, pointing at a copy of the device's" >&2
    echo "       /usr/java/lib/fonts — the preview draws with the faces the" >&2
    echo "       Kindle has, not the host's." >&2
    exit 1
fi

cd "$ROOT"
exec cargo run --quiet --bin preview -- "$@"
