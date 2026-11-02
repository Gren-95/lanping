#!/usr/bin/env bash
# Builds LanPing.app for Intel and Apple silicon and packs it as a .dmg.
# Runs on macOS and needs the two Rust targets:
#   rustup target add aarch64-apple-darwin x86_64-apple-darwin
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
build_dir="$root/target/macos"
app="$build_dir/LanPing.app"
version="$(sed -n 's/^version = "\(.*\)"/\1/p' "$root/Cargo.toml" | head -n 1)"
output="$build_dir/LanPing-$version-macos-universal.dmg"

for target in aarch64-apple-darwin x86_64-apple-darwin; do
  cargo build --release --locked -p lanping --target "$target" --manifest-path "$root/Cargo.toml"
done

rm -rf "$build_dir"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
lipo -create -output "$app/Contents/MacOS/lanping" \
  "$root/target/aarch64-apple-darwin/release/lanping" \
  "$root/target/x86_64-apple-darwin/release/lanping"
cp "$root/packaging/lanping.icns" "$app/Contents/Resources/lanping.icns"
sed "s/@VERSION@/$version/g" "$root/packaging/Info.plist" > "$app/Contents/Info.plist"

# An ad-hoc signature is not a Developer ID, but Apple silicon refuses to run unsigned code.
codesign --force --deep --sign - "$app"
codesign --verify --deep --strict "$app"
lipo -archs "$app/Contents/MacOS/lanping"

staging="$build_dir/dmg"
mkdir -p "$staging"
cp -R "$app" "$staging/"
ln -s /Applications "$staging/Applications"
hdiutil create -volname "LanPing" -srcfolder "$staging" -format UDZO -ov "$output"
echo "Built $output"
