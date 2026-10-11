# Paper Texture (WebAssembly filter plug-in)

Generates a seeded paper texture on the active layer. Written in freestanding C, compiled to one
20 KB `.wasm` file with no imports; it runs in PhotoCraft's plug-in sandbox on every platform.

## Install

**Filter › Plug-ins › Install Plug-in…**, pick `paper_texture.wasm`. It then appears as
**Filter › Plug-ins › Paper Texture…**. (Or put the file in your Plug-ins folder, set in
Edit › Preferences › Plug-ins.) Use it on a layer; select an area first to texture only that area.

## Controls

Every effect has its own 0–100 intensity (0 = off). Same seed and settings always give the same paper.

| Control | What it does |
|---|---|
| Seed | Different number, different sheet |
| Paper | white, cream, aged, parchment, newsprint, kraft |
| Apply | *replace layer*, or *multiply over layer* (puts the paper over a photo or artwork) |
| Texture, Texture size | Grain, fibres and tooth |
| Wrinkles, Wrinkle size | Crumpled, lit relief |
| Creases, Crease count | Thin worn lines with a lit side |
| Folds, Fold count, Fold layout | Straight folds: horizontal, vertical, both, diagonal |
| Stains, Stain count | Coffee-ring stains |
| Aging | Yellowing, blotches, edge wear, foxing specks |
| Burns, Burn spots, Burn through | Scorched edges and spots; Burn through makes them burn holes (transparent) |
| Rips, Rip edges | Torn edge with a pale fibrous rim: all edges, top, bottom, left, right or corner |

Notes
- Sizes are relative to the canvas, so the paper looks the same at any resolution.
- Holes from rips and burns are transparent on layers with alpha. On a layer without alpha
  (a Background) they show a dark backdrop instead.
- RGB and grayscale documents. Grayscale gets a slightly coarser texture (the sandbox gives each
  sample a fixed instruction budget); CMYK, Lab and others are refused. Convert to RGB first.
- Large canvases take a while: this runs in an interpreter. All effects at 100 on a 24 MP RGB
  layer is a long wait; typical settings on a few megapixels take seconds.

## Build

`./build.sh` needs `clang` with the WebAssembly target and `wasm-ld` (no Rust target needed).
`test_native.c` renders the same code natively to a PPM for quick tuning:
`clang -O2 -o preview test_native.c -lm && ./preview 700 900 '"seed":3,"burns":50' out.ppm`.

The tests are `crates/plugins/tests/paper_texture.rs`, run against the real sandbox.
