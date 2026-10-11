#!/bin/bash
# Builds PhotoCraft and runs it.
# Run this from inside the photocraft folder: ./build.sh

set -e

echo "Checking for Rust..."
if ! command -v cargo &> /dev/null; then
    echo "Rust isn't installed. Install it first from https://rustup.rs then run this again."
    exit 1
fi

echo "Building PhotoCraft (this can take a while the first time)..."
cargo build --release -p photocraft

echo ""
echo "Build finished. The app is at: target/release/photocraft"
echo "Starting it now..."
./target/release/photocraft
