# Ingestion integration test fixtures

Small PDFs for [`../ingestion.rs`](../ingestion.rs). Regenerate with:

```bash
python core/tests/fixtures/generate_fixtures.py
```

After changing fixtures, calibrate assertions with:

```bash
cargo run --example ocr_demo -- core/tests/fixtures/<name>.pdf
```

Inspect `ocr_output.txt` for `assembled_markdown`, chunk text, and candidate `meta.fallback_reason` before updating test expectations.

| File | Purpose |
|------|---------|
| `digital_two_column.pdf` | Digital path, two-column layout ordering |
| `digital_injection.pdf` | Prompt injection → `injection_flagged` fallback |
| `digital_minimal.pdf` | Single-column fallback routing (`no_llm_configured`, unsupported provider) |
| `scanned_single_page.pdf` | Image-only page for rasterization / `Ocr` classification |
