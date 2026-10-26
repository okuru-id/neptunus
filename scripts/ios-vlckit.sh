#!/usr/bin/env bash
# CI (macOS), after `tauri ios init`: links MobileVLCKit into the generated Xcode project and adds the
# film screen built on it (src-tauri/ios/NeptunusPlayer.swift), then generates the project again.
# Without this step the app still builds, and films play in the WebView's player.
set -euo pipefail

# MobileVLCKit 3.7.4 (LGPL-2.1), as its CocoaPods spec points to it
URL=https://download.videolan.org/pub/cocoapods/prod/MobileVLCKit-3.0-20260922-1512.tar.xz
SHA256=c649c4c47bd065b3a5511746361b5a9066a34033f2df9cbc9454409c0f363b52

cd "$(dirname "$0")/.."
APPLE=src-tauri/gen/apple
[ -f "$APPLE/project.yml" ] || { echo "run 'tauri ios init' first" >&2; exit 1; }

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
curl -fsSL -o "$tmp/vlckit.tar.xz" "$URL"
echo "$SHA256  $tmp/vlckit.tar.xz" | shasum -a 256 -c -
tar -xJf "$tmp/vlckit.tar.xz" -C "$tmp"
fw=$(find "$tmp" -name MobileVLCKit.xcframework -type d -prune | head -1)
[ -n "$fw" ] || { echo "MobileVLCKit.xcframework not in the download" >&2; exit 1; }
mkdir -p "$APPLE/Frameworks"
rm -rf "$APPLE/Frameworks/MobileVLCKit.xcframework"
mv "$fw" "$APPLE/Frameworks/"

# the app target's sources: Sources/<app name>/ (main.mm and the bindings are there)
src=$(find "$APPLE/Sources" -mindepth 1 -maxdepth 1 -type d | head -1)
cp src-tauri/ios/NeptunusPlayer.swift "$src/"

# a dynamic framework: embedded in the app (and signed with it), else the app dies at launch
ruby -ryaml - "$APPLE/project.yml" <<'RUBY'
path = ARGV[0]
spec = YAML.load_file(path)
name, target = spec["targets"].find { |k, _| k.end_with?("_iOS") }
abort "no iOS target in #{path}" unless target
deps = target["dependencies"] ||= []
deps << { "framework" => "Frameworks/MobileVLCKit.xcframework", "embed" => true, "codeSign" => true }
# it exports its own clock_gettime, which fails for the clocks Rust's std asks for (a panic at launch);
# the linker takes a symbol from the first library that has it, so libSystem goes before it
base = ((target["settings"] ||= {})["base"] ||= {})
base["OTHER_LDFLAGS"] = "$(inherited) -lSystem"
File.write(path, spec.to_yaml)
puts "MobileVLCKit added to #{name}"
RUBY

(cd "$APPLE" && xcodegen generate --spec project.yml)
