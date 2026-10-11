#!/bin/sh
# Builds paper_texture.wasm with clang (any recent clang with the WebAssembly target + wasm-ld).
set -eu
cd "$(dirname "$0")"
clang --target=wasm32 -O2 -nostdlib -ffreestanding -fno-builtin -fno-math-errno \
  -mno-simd128 -mno-bulk-memory -mno-reference-types -mno-multivalue -mno-nontrapping-fptoint \
  -Wall -Wextra -Wl,--no-entry -Wl,-z,stack-size=65536 -Wl,--initial-memory=262144 -Wl,--max-memory=536870912 -Wl,--strip-all \
  -o paper_texture.wasm paper_texture.c
echo "built paper_texture.wasm ($(wc -c < paper_texture.wasm) bytes)"
