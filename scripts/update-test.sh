#!/usr/bin/env bash
# End-to-end test of the updater without publishing a release. Builds this version as the
# "old" app and a copy numbered 99.0.0 as the "new" one, serves the new one from a local
# server shaped like GitHub's releases/latest, and launches the old one pointed at it with a
# scratch APPDATA. Ctrl+C stops the server.
#
#   scripts/update-test.sh            the normal update
#   scripts/update-test.sh badsum     serve a wrong checksum; the install must be refused
set -euo pipefail
cd "$(dirname "$0")/.."

MODE="${1:-}"
W="target/update-test"
PORT=8765
NEW=99.0.0

if tasklist | grep -qi blazeaudio.exe; then
    echo "BlazeAudio is already running; close it first (the test copy would just focus it)." >&2
    exit 1
fi

rm -rf "$W"
mkdir -p "$W/app" "$W/server" "$W/appdata"
cp Cargo.toml "$W/Cargo.toml.bak"
cp Cargo.lock "$W/Cargo.lock.bak"
restore() { cp "$W/Cargo.toml.bak" Cargo.toml; cp "$W/Cargo.lock.bak" Cargo.lock; }
trap restore EXIT

echo "Building the new version ($NEW)..."
sed -i "0,/^version = \".*\"/s//version = \"$NEW\"/" Cargo.toml
touch src/main.rs  # a version change alone may not re-copy the exe
cargo build -q
cp target/debug/blazeaudio.exe "$W/server/"
restore

echo "Building the old version ($(grep -m1 '^version' Cargo.toml | cut -d'"' -f2))..."
touch src/main.rs  # a version change alone may not re-copy the exe
cargo build -q
cp target/debug/blazeaudio.exe "$W/app/"

hash=$(sha256sum "$W/server/blazeaudio.exe" | cut -d' ' -f1)
if [ "$MODE" = badsum ]; then
    hash=$(printf '0%.0s' $(seq 64))
fi
printf '%s  blazeaudio.exe' "$hash" > "$W/server/blazeaudio.exe.sha256"
url="http://127.0.0.1:$PORT"
cat > "$W/server/latest.json" <<JSON
{"tag_name": "v$NEW", "assets": [
  {"name": "blazeaudio.exe", "browser_download_url": "$url/blazeaudio.exe"},
  {"name": "blazeaudio.exe.sha256", "browser_download_url": "$url/blazeaudio.exe.sha256"}]}
JSON

python -m http.server "$PORT" --bind 127.0.0.1 --directory "$W/server" &
server=$!
trap 'restore; kill $server 2>/dev/null' EXIT
sleep 1

echo "Launching the old version from $W/app. About 5 s after it opens, General > Updates"
echo "should show the install. Restart now should reopen it as $NEW."
APPDATA="$(cygpath -w "$PWD/$W/appdata")" BLAZE_UPDATE_URL="$url/latest.json" "$W/app/blazeaudio.exe" &
wait $server
