#!/usr/bin/env bash
# Builds LanPing and packs it as an AppImage.
# Needs `appimagetool` on PATH (https://github.com/AppImage/appimagetool).
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
build_dir="$root/target/appimage"
app_dir="$build_dir/LanPing.AppDir"
version="$(cargo metadata --no-deps --format-version 1 --manifest-path "$root/Cargo.toml" \
  | python3 -c 'import json,sys; print(next(p["version"] for p in json.load(sys.stdin)["packages"] if p["name"]=="lanping"))')"

if ! command -v appimagetool >/dev/null 2>&1; then
  echo "appimagetool not found on PATH. Download it from https://github.com/AppImage/appimagetool/releases" >&2
  exit 1
fi

cargo build --release -p lanping --manifest-path "$root/Cargo.toml"

rm -rf "$app_dir"
mkdir -p "$app_dir/usr/bin"
install -m 755 "$root/target/release/lanping" "$app_dir/usr/bin/lanping"
install -m 644 "$root/packaging/lanping.desktop" "$app_dir/lanping.desktop"
install -m 644 "$root/packaging/lanping.svg" "$app_dir/lanping.svg"
ln -s usr/bin/lanping "$app_dir/AppRun"

output="$build_dir/LanPing-$version-x86_64.AppImage"
ARCH=x86_64 appimagetool --appimage-extract-and-run "$app_dir" "$output" 2>/dev/null \
  || ARCH=x86_64 appimagetool "$app_dir" "$output"
echo "Built $output"
