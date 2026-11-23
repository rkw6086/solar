#!/bin/bash
# Pinned native tools for GitHub-hosted Apple Silicon jobs.
set -euo pipefail
[[ $(uname -s) == Darwin && $(uname -m) == arm64 ]]
[[ "$RUNNER_TEMP" == /* && "$SOLAR_JOB_ID" =~ ^[0-9]+-[0-9]+$ ]]
if [[ -n ${SOLAR_BUILD_CACHE_ROOT:-} ]]; then
  [[ $SOLAR_BUILD_CACHE_ROOT == "$RUNNER_TEMP/solar-ci-cache-v1" &&
     ${SOLAR_CACHED_TOOLS_DIR:-} == "$SOLAR_BUILD_CACHE_ROOT/tools" &&
     ${SOLAR_CACHED_TARGET_DIR:-} == "$RUNNER_TEMP/solar-build" ]]
  for directory in "$SOLAR_BUILD_CACHE_ROOT" "$SOLAR_CACHED_TOOLS_DIR" "$SOLAR_CACHED_TARGET_DIR" "$SOLAR_CACHED_TOOLS_DIR/corepack" "$SOLAR_CACHED_TOOLS_DIR/pnpm-store"; do
    [[ ! -L "$directory" && ( ! -e "$directory" || -d "$directory" ) ]]
  done
  mkdir -p "$SOLAR_CACHED_TOOLS_DIR/corepack" "$SOLAR_CACHED_TOOLS_DIR/pnpm-store"
  printf '%s\n' "COREPACK_HOME=$SOLAR_CACHED_TOOLS_DIR/corepack" >> "$GITHUB_ENV"
  printf '%s\n' "SOLAR_PNPM_STORE_DIR=$SOLAR_CACHED_TOOLS_DIR/pnpm-store" >> "$GITHUB_ENV"
fi
export PATH="$HOME/.cargo/bin:$PATH"
command -v rustup >/dev/null || { echo 'Install rustup for the runner user first.' >&2; exit 1; }
rustup toolchain install 1.99.1 --profile minimal --component rustfmt --target aarch64-apple-darwin
npm install --global corepack@0.36.0
# Install public, hash-pinned layout tools before the signing step receives keys.
# Keep this environment out of the app and unsigned build caches.
dmg_tools="$RUNNER_TEMP/solar-dmg-tools-$SOLAR_JOB_ID"
[[ ! -e "$dmg_tools" && ! -L "$dmg_tools" ]]
python3 -m venv "$dmg_tools"
"$dmg_tools/bin/python3" -m pip install --disable-pip-version-check --only-binary=:all: --require-hashes -r scripts/ci/dmg/requirements.txt
printf '%s\n' "SOLAR_DMG_PYTHON=$dmg_tools/bin/python3" >> "$GITHUB_ENV"
printf '%s\n' "$HOME/.cargo/bin" >> "$GITHUB_PATH"
printf '%s\n' 'RUSTUP_TOOLCHAIN=1.99.1' >> "$GITHUB_ENV"
printf '%s\n' "CARGO_TARGET_DIR=${SOLAR_CACHED_TARGET_DIR:-$RUNNER_TEMP/solar-build-$SOLAR_JOB_ID}" >> "$GITHUB_ENV"
printf '%s\n' "SOLAR_RELEASE_DIR=$RUNNER_TEMP/solar-artifacts-$SOLAR_JOB_ID" >> "$GITHUB_ENV"
