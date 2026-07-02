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

Bundled embedding tests and local inference expect model files under `~/.amber/models/embed/` (sanitized model ID as filename):

- **Light Tier**:
  - `avsolatorio_GIST-small-Embedding-v0.onnx`
  - `avsolatorio_GIST-small-Embedding-v0_tokenizer.json`
- **Standard Tier**:
  - `avsolatorio_GIST-Embedding-v0.onnx`
  - `avsolatorio_GIST-Embedding-v0_tokenizer.json`
- **Quality Tier**:
  - `microsoft_harrier-oss-v1-270m.onnx`
  - `microsoft_harrier-oss-v1-270m_tokenizer.json`

## OCR model artifacts (PP-OCRv6_small)

Bundled document ingestion layout analysis and text extraction expect OCR models under `~/.amber/models/ocr/`:

- `det.onnx` (Text detection model)
- `rec.onnx` (Text recognition model)
- `ppocrv6_dict.txt` (Vocabulary mapping file for CTC decoding)

## First-Run Model Setup

Rather than manual staging, the in-app Model Download Manager handles downloading these models directly from HuggingFace during the frictionless first-run onboarding setup. The manager verifies files using SHA-256 hash validation and supports HTTP range-based chunk-resume. The active embedding model tier is dynamically determined by probing system hardware resources (RAM and VRAM).
