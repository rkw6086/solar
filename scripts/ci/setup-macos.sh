#!/bin/bash
# Shared tool setup for the serialized stable and beta native jobs.
set -euo pipefail
[[ $(uname -s) == Darwin && $(uname -m) == arm64 ]]
[[ "$RUNNER_TEMP" == /* && "$SOLAR_JOB_ID" =~ ^[0-9]+-[0-9]+$ ]]
export PATH="$HOME/.cargo/bin:$PATH"
command -v rustup >/dev/null || { echo 'Install rustup for the runner user first.' >&2; exit 1; }
rustup toolchain install 1.99.1 --profile minimal --component rustfmt --target aarch64-apple-darwin
npm install --global corepack@0.36.0
printf '%s\n' "$HOME/.cargo/bin" >> "$GITHUB_PATH"
printf '%s\n' 'RUSTUP_TOOLCHAIN=1.99.1' >> "$GITHUB_ENV"
printf '%s\n' "CARGO_TARGET_DIR=$RUNNER_TEMP/solar-build-$SOLAR_JOB_ID" >> "$GITHUB_ENV"
printf '%s\n' "SOLAR_RELEASE_DIR=$RUNNER_TEMP/solar-artifacts-$SOLAR_JOB_ID" >> "$GITHUB_ENV"
