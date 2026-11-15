#!/usr/bin/env bash
# Netlify build: compile the server as a self-contained Linux program and hand it to
# Netlify as the function "app"; copy the static files to the folder Netlify serves.
# Run from web/ (Netlify does this when the base directory is "web").
set -euo pipefail

TARGET=x86_64-unknown-linux-musl

if ! command -v rustup >/dev/null 2>&1; then
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain none
  # shellcheck disable=SC1091
  . "$HOME/.cargo/env"
fi
rustup toolchain install stable --profile minimal
rustup default stable
rustup target add "$TARGET"

# The "ring" crate has C code. Use the musl compiler when the machine has one;
# otherwise the ordinary compiler does the job.
if ! command -v musl-gcc >/dev/null 2>&1; then
  export CC_x86_64_unknown_linux_musl=gcc
fi

cargo build --release --target "$TARGET"

mkdir -p netlify/functions dist
cp "target/$TARGET/release/flame_in_freefall" netlify/functions/app
rm -rf dist/static
cp -r static dist/static
echo "Built function: $(du -h netlify/functions/app | cut -f1)"
