#!/bin/bash
set -euo pipefail

fail() { echo "Solar macOS build: $*" >&2; exit 1; }
[[ $(uname -s) == Darwin && $(uname -m) == arm64 ]] || fail 'Use a native Apple Silicon macOS runner.'
[[ -n ${CARGO_TARGET_DIR:-} && -n ${SOLAR_RELEASE_DIR:-} ]] || fail 'Set job-specific CARGO_TARGET_DIR and SOLAR_RELEASE_DIR.'
[[ $(rustc --version) == 'rustc 1.99.1 '* ]] || fail 'Rust 1.99.1 is required.'
[[ $(node --version) == v24.17.0 ]] || fail 'Node 24.17.0 is required.'
xcode-select -p >/dev/null
xcrun --find swiftc >/dev/null
xcrun --find clang >/dev/null
xcrun --find notarytool >/dev/null
xcrun --find stapler >/dev/null

# Refuse mismatched version tags, rather than publishing the wrong app version.
version=$(node -p 'JSON.parse(require("fs").readFileSync("src-tauri/tauri.conf.json", "utf8")).version')
if [[ ${GITHUB_REF:-} == refs/tags/* ]]; then
  [[ $GITHUB_REF == "refs/tags/v$version" ]] || fail "Tag must be v$version to match tauri.conf.json."
fi
[[ -z $(git status --porcelain) ]] || fail 'Build requires a clean checkout.'
export SOLAR_SOURCE_COMMIT
SOLAR_SOURCE_COMMIT=$(git rev-parse HEAD)
export SOLAR_SOURCE_MODIFIED=false

corepack pnpm install --frozen-lockfile
corepack pnpm run check
corepack pnpm run test:intro
corepack pnpm run test:system
corepack pnpm run test:models
corepack pnpm run test:profiles
corepack pnpm run test:tools
corepack pnpm run test:projects
cargo fmt --all -- --check

# Do not put any signing/notarization secrets in this step's environment.
# The macOS-specific Tauri config bundles the Swift native helper automatically.
corepack pnpm tauri build --ci --target aarch64-apple-darwin --bundles app --no-sign -- --locked
app="$CARGO_TARGET_DIR/aarch64-apple-darwin/release/bundle/macos/Solar.app"
[[ -x "$app/Contents/MacOS/solar" ]] || fail 'Solar executable missing from app bundle.'
[[ -x "$app/Contents/MacOS/solar-native-helper" ]] || fail 'Native helper missing from app bundle.'
lipo -verify_arch arm64 "$app/Contents/MacOS/solar"
lipo -verify_arch arm64 "$app/Contents/MacOS/solar-native-helper"
echo 'Unsigned app built and native helper verified; ready for signing.'
