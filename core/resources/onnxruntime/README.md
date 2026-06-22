Place ONNX Runtime shared libraries for packaging verification in this directory.

Expected platform artifacts include `.dll` on Windows, `.dylib` on macOS, and `.so`
on Linux. Intentionally does not copy these files automatically from
`build.rs`; the `ort` crate downloads/copies runtime libraries for development
builds under `core/target/`, and release bundle verification can stage
platform-specific libraries here manually.

## Stage from a local Cargo build (Windows)

From the repo root, after building the core crate:

```powershell
cargo build --manifest-path core/Cargo.toml
powershell -ExecutionPolicy Bypass -File scripts/stage-onnxruntime.ps1
```

Tauri bundles anything matched by `bundle.resources` → `resources/onnxruntime/*`
in `core/tauri.conf.json`.

## Embedding model artifacts (separate from ORT runtime libs)

Bundled embedding tests expect model files under `~/.amber/models/embed/`:

- `avsolatorio_GIST-small-Embedding-v0.onnx`
- `avsolatorio_GIST-small-Embedding-v0_tokenizer.json`

Model download UX is deferred to M2.8.
