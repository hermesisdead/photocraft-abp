# Generative Fill: model browser and tool (status)

The Generative Fill tool sits in the retouching group of the toolbar, next to Content-Aware Move.
Draw a selection with it, then use its options bar:

- **Generate…** opens the Generative Fill pop-up (prompt, negative prompt, steps, guidance, model).
- **Models…** opens the model window: a dropdown of every `.onnx` file in the models folder,
  a one-click **Download default model** button (LaMa, about 208 MB) with a progress bar, Cancel,
  and a red error message with **Try again** if a download fails, an **Open models folder** button
  (drop your own `.onnx` files in and press Refresh), and a Hugging Face search for more models.

## What works

- **Model search.** `ai.generativeFill.search` queries the Hub's model API. The browser runs it on a
  background thread.
- **Verified downloads.** `ai.generativeFill.download` fetches the `.onnx` files of a model
  repository. Each file must match the size and SHA-256 that the Hub lists for it. A file without a
  checksum on record is refused. Files are written under `*.part` names and installed only after
  every file verifies. A failed download leaves nothing behind.
- **Storage.** Models live in `<cache>/photocraft/generative-fill/<owner>__<name>/`, where `<cache>`
  is `$XDG_CACHE_HOME`, `~/Library/Caches` on macOS, or `%LOCALAPPDATA%` on Windows. Each folder has
  a `manifest.json` that lists the verified files.
- **Pop-up validation.** `ai.generativeFill` checks the prompt length, the step and guidance ranges,
  the model, and the selection.

## What does not work yet

- **Inference.** No inpainting backend is part of this build. A valid `ai.generativeFill` request
  reports that the installed model cannot run yet, and it does not change the document. Adding the
  backend means choosing a runtime (ONNX Runtime or a pure-Rust one) and an inpainting model whose
  operators it supports. That work is not done.
- **Web build.** Downloads are unavailable in the WebAssembly build, and the commands say so.

## Dependencies

The native build adds `ureq` (HTTP, rustls TLS) and `sha2` to `photocraft-engine`. Both are
target-gated to non-wasm targets.

## Where the code is

- `crates/engine/src/generative_fill_cmds.rs`: commands, Hub client, verification, storage.
- `crates/ui-egui/src/generative_fill_ui.rs`: the browser window and the options-bar buttons.
- `crates/ui-egui/src/prefs_ui.rs`: the `ai.generativeFill` pop-up arm.
