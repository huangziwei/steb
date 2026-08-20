#!/bin/sh
# ./build.sh — cross-compile Steb and stage device/ for a USB copy:
# device/extensions/steb/ -> /mnt/us/extensions/steb/, device/documents/Steb.sh
# -> /mnt/us/documents/Steb.sh. One armv7 musl binary covers the fleet.
set -eu

TARGET="armv7-unknown-linux-musleabihf"
ROOT="$(cd "$(dirname "$0")" && pwd)"
OUT="$ROOT/device/extensions/steb/bin/steb"

# A missing cross target, as one line past cargo's own
# "can't find core for armv7-…" panic.
if ! rustup target list --installed | grep -qx "$TARGET"; then
    echo "error: rustup target '$TARGET' is not installed" >&2
    echo "       fix: rustup target add $TARGET" >&2
    exit 1
fi

# Single source of truth for the version: [workspace.package] in Cargo.toml.
# The binary picks it up through CARGO_PKG_VERSION; this copy is only for the
# build banner below.
VERSION="$(awk '/^\[workspace\.package\]/{f=1;next} /^\[/{f=0}
                f && /^version *=/{gsub(/[" ]/,""); sub(/^version=/,""); print; exit}' \
    "$ROOT/Cargo.toml")"
[ -n "$VERSION" ] || { echo "error: no version in [workspace.package]" >&2; exit 1; }

echo "==> building steb $VERSION for $TARGET"
cargo build --release --target "$TARGET" -p steb-native --bin steb-native

# Named `steb` on device, not `steb-native`: the tile's single-instance guard
# is `pidof steb`, and the tile runs that name. The cargo target keeps the
# longer name so a host build cannot collide with it.
mkdir -p "$(dirname "$OUT")"
cp "$ROOT/target/$TARGET/release/steb-native" "$OUT"
chmod +x "$OUT" 2>/dev/null || true

echo "==> staged $(ls -lh "$OUT" | awk '{print $5}') -> device/extensions/steb/bin/steb"
file "$OUT" 2>/dev/null || true

# The scriptlet is checked, never rebuilt: its `# Icon:` line is a ~21KB base64
# blob device/make-tile.sh regenerates, and it ships with the icon embedded. A
# cover edited without make-tile.sh leaves stale art.
TILE="$ROOT/device/documents/Steb.sh"
COVER="$ROOT/device/assets/cover.png"
if ! grep -q '^# Icon: data:image/png;base64,' "$TILE" 2>/dev/null; then
    echo "warning: $TILE has no embedded cover — run device/make-tile.sh" >&2
elif [ -f "$COVER" ] && [ "$COVER" -nt "$TILE" ]; then
    echo "warning: assets/cover.png is newer than the tile's embedded icon" >&2
    echo "         the old cover would ship — run device/make-tile.sh" >&2
fi

cat <<'EOF'

==> install — copy these two onto the device

    device/extensions/steb/    ->  /mnt/us/extensions/steb/
    device/documents/Steb.sh   ->  /mnt/us/documents/Steb.sh

Books land in /mnt/us/documents/standardebooks/.
Logs, if anything goes wrong, in /mnt/us/logs/steb.log.
EOF
