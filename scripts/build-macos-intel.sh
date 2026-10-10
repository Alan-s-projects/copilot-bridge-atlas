#!/usr/bin/env bash
set -euo pipefail

if [[ "$(uname -s)" != Darwin ]]; then
  printf '%s\n' 'Build the macOS Intel package on a Mac with Xcode Command Line Tools.' >&2
  exit 1
fi

atlas_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$atlas_root"
atlas_version="$(node --input-type=module - <<'NODE'
import { readFileSync } from "node:fs";
const packageVersion = JSON.parse(readFileSync("package.json", "utf8")).version;
const tauriVersion = JSON.parse(readFileSync("src-tauri/tauri.conf.json", "utf8")).version;
const cargoVersion = readFileSync("src-tauri/Cargo.toml", "utf8").match(/^version = "([^"]+)"$/m)?.[1];
if (!/^\d+\.\d+\.\d+$/.test(packageVersion) || packageVersion !== tauriVersion || packageVersion !== cargoVersion) {
  throw new Error("package.json, Cargo.toml, and tauri.conf.json must share one stable version.");
}
console.log(packageVersion);
NODE
)"
atlas_minimum="$(node -p 'JSON.parse(require("fs").readFileSync("src-tauri/tauri.macos.conf.json", "utf8")).bundle.macOS.minimumSystemVersion')"
if [[ -n "${MACOSX_DEPLOYMENT_TARGET:-}" && "$MACOSX_DEPLOYMENT_TARGET" != "$atlas_minimum" ]]; then
  printf '%s\n' 'MACOSX_DEPLOYMENT_TARGET must match tauri.macos.conf.json.' >&2
  exit 1
fi
export MACOSX_DEPLOYMENT_TARGET="$atlas_minimum"
xcrun --sdk macosx --show-sdk-path > /dev/null

atlas_target='x86_64-apple-darwin'
rustup target add "$atlas_target"
atlas_target_root="${CARGO_TARGET_DIR:-$atlas_root/src-tauri/target}"
if [[ "$atlas_target_root" != /* ]]; then
  atlas_target_root="$atlas_root/$atlas_target_root"
fi
export CARGO_TARGET_DIR="$atlas_target_root"
node node_modules/@tauri-apps/cli/tauri.js build --target "$atlas_target" --bundles app,dmg -- --locked
atlas_bundle="$atlas_target_root/$atlas_target/release/bundle"
atlas_app="$atlas_bundle/macos/Copilot Bridge Atlas.app"
atlas_built_dmg="$atlas_bundle/dmg/Copilot Bridge Atlas_${atlas_version}_x64.dmg"
atlas_dmg_name="Copilot-Bridge-Atlas-${atlas_version}-macOS-Intel.dmg"
mkdir -p release
python3 scripts/check-macos-bundle.py --app "$atlas_app" --dmg "$atlas_built_dmg"
cp "$atlas_built_dmg" "release/$atlas_dmg_name"
(
  cd release
  shasum -a 256 "$atlas_dmg_name" > "$atlas_dmg_name.sha256"
)
printf '%s\n' "$atlas_root/release/$atlas_dmg_name"
