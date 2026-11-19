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

# Preflight pins the checkout and channel before any build runs.
version=$(node -p 'JSON.parse(require("fs").readFileSync("src-tauri/tauri.conf.json", "utf8")).version')
export SOLAR_BUILD_CHANNEL=${SOLAR_BUILD_CHANNEL:-release}
[[ ${SOLAR_RELEASE_VERSION:-} == "$version" ]] || fail 'Validated package version is missing or mismatched.'
[[ ${SOLAR_RELEASE_SHA:-} =~ ^([0-9a-f]{40}|[0-9a-f]{64})$ ]] || fail 'Validated source commit is missing.'
[[ $(git rev-parse HEAD) == "$SOLAR_RELEASE_SHA" ]] || fail 'Checkout does not match the validated source commit.'
if [[ $SOLAR_BUILD_CHANNEL == beta ]]; then
  [[ ( ${GITHUB_EVENT_NAME:-} == schedule || ${GITHUB_EVENT_NAME:-} == workflow_dispatch ) && ${GITHUB_REF:-} == refs/heads/main ]] || fail 'Beta builds require a scheduled or manual main run.'
  [[ ${SOLAR_BETA_ID:-} == "$SOLAR_RELEASE_SHA" ]] || fail 'Beta identity must match the validated main commit.'
elif [[ $SOLAR_BUILD_CHANNEL == release ]]; then
  [[ ${GITHUB_EVENT_NAME:-} == push && ${GITHUB_REF:-} == "refs/tags/$version" ]] || fail "Release builds require a pushed $version tag."
else
  fail 'Build channel must be release or beta.'
fi
node scripts/release.mjs check "$version"
[[ -z $(git status --porcelain) ]] || fail 'Build requires a clean checkout.'
export SOLAR_SOURCE_COMMIT="$SOLAR_RELEASE_SHA"
export SOLAR_SOURCE_MODIFIED=false
export VITE_SOLAR_BUILD_CHANNEL="$SOLAR_BUILD_CHANNEL"
export VITE_SOLAR_SOURCE_COMMIT="$SOLAR_RELEASE_SHA"

corepack pnpm install --frozen-lockfile
corepack pnpm run check
corepack pnpm run test:intro
corepack pnpm run test:system
corepack pnpm run test:models
corepack pnpm run test:profiles
corepack pnpm run test:tools
corepack pnpm run test:projects
corepack pnpm run test:release
corepack pnpm run test:beta
cargo fmt --all -- --check

# Do not put any signing/notarization secrets in this step's environment.
# The macOS-specific Tauri config bundles the Swift native helper automatically.
corepack pnpm tauri build --ci --target aarch64-apple-darwin --bundles app --no-sign -- --locked
app="$CARGO_TARGET_DIR/aarch64-apple-darwin/release/bundle/macos/Solar.app"
[[ -x "$app/Contents/MacOS/solar" ]] || fail 'Solar executable missing from app bundle.'
[[ -x "$app/Contents/MacOS/solar-native-helper" ]] || fail 'Native helper missing from app bundle.'
lipo -verify_arch arm64 "$app/Contents/MacOS/solar"
lipo -verify_arch arm64 "$app/Contents/MacOS/solar-native-helper"
# Record identity in the unsigned bundle; packaging verifies it before signing.
python3 - "$app/Contents/Info.plist" "$SOLAR_BUILD_CHANNEL" "$SOLAR_RELEASE_SHA" <<'IDENTITY'
import pathlib, plistlib, sys
path = pathlib.Path(sys.argv[1])
with path.open('rb') as source:
    data = plistlib.load(source)
data.update(SolarBuildChannel=sys.argv[2], SolarSourceCommit=sys.argv[3])
with path.open('wb') as destination:
    plistlib.dump(data, destination)
IDENTITY
echo 'Unsigned app built and native helper verified; ready for signing.'
