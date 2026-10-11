# Cassette Tape (WebAssembly filter plug-in)

Draws a compact cassette from above on the active layer, with a live preview while you change the
settings. Freestanding C compiled to one 40 KB `.wasm` file with no imports; it runs in PhotoCraft's
plug-in sandbox on every platform.

## Install and use

1. **Filter › Plug-ins › Install Plug-in…**, pick `cassette_tape.wasm`.
2. Make a new landscape document (about 2000 × 1300 px is a good size) and run
   **Filter › Plug-ins › Cassette Tape…**. The dialog previews as you change settings.

The plug-in fills the whole canvas, so run it on an empty layer. Everything is measured in
millimetres (a real cassette is about 100 × 64 mm), so the cassette keeps its proportions at any
canvas size. Same seed and settings always give the same image.

## Controls

| Group | Controls |
|---|---|
| Scene | Seed, Size (how much of the canvas), Angle, Backdrop (studio grey, white, black, charcoal, kraft, blue, wood, transparent), Shadow |
| Shell | Shell color (white, black, grey, red, blue, green, yellow, orange, smoke), Shell finish (smooth, matte grain, speckled, brushed, glossy), Shell texture, Edge wear |
| Label | Label paper (grey, white, cream, kraft, newsprint, yellow), Label style (blank, ruled, stripe, stripe and ruled, double stripe), Stripe color, Label texture, Handwriting, Label wear, Label stains, **Label rips** + Rip side (all, top, bottom, left, right, corner) |
| Tape | Tape type (ferric brown, chrome, metal, gold), Tape amount (how full each reel looks), Tape texture, Window glare |
| Damage | **Scratches** + Scratch count + Scratch style (fine hairlines, deep gouges, mixed), Scuffs, Dust, Grime |

Intensities run 0–100 (0 = off). With Backdrop set to *transparent* the cassette and its shadow keep
their own transparency, so you can drop it onto other artwork.

## Notes

- RGB and grayscale layers; CMYK, Lab and others are refused (convert to RGB first). A grayscale
  layer with no alpha gets a slightly coarser texture (the sandbox gives each sample a fixed
  instruction budget).
- It runs in an interpreter, so a very large canvas takes a long time (over a minute and the run is
  stopped). Keep it to roughly 3000 px wide or less.
- The label, window, hubs, screws and lower part are drawn from measurements of a generic compact
  cassette; there is no logo or brand artwork.

## Build

`./build.sh` needs `clang` with the WebAssembly target and `wasm-ld`. `test_native.c` renders the
same code to a PPM for quick tuning:
`clang -O2 -o preview test_native.c -lm && ./preview 1500 1000 '"seed":3,"scratches":60' out.ppm`.

The manifest and the parameter tables in `cassette_tape.c` are written out by hand: when you add a
control, add it to the manifest, the `Params` struct, `NUMS`/`CHOICES` and `set_defaults`.

Tests: `crates/plugins/tests/cassette_tape.rs`, run against the real sandbox.
