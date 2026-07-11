# Ingestion integration test fixtures

Small PDFs for [`../ingestion.rs`](../ingestion.rs). Regenerate with:

```bash
python core/tests/fixtures/generate_fixtures.py
```

After changing fixtures, calibrate assertions with:

```bash
cargo run --example ocr_demo -- core/tests/fixtures/<name>.pdf
```

Inspect `ocr_output.md` for `assembled_markdown`, chunk text, and candidate `meta.fallback_reason` before updating test expectations.

For an unseen PDF, run the demo with `AMBER_INGEST_DEBUG=1` and verify:

1. Per-page band and column-split counts are plausible for the document layout.
2. Markdown has no mid-sentence column jumps and no duplicate spaces.
3. `heading_context` is a complete title rather than one wrapped line.
4. Candidate details are contained in the source chunk; warnings in candidate metadata identify exceptions.
5. Do not add an arbitrary real document to CI: make a small synthetic fixture that isolates the regression.

| File | Purpose |
|------|---------|
| `digital_two_column.pdf` | Digital path, two-column layout ordering |
| `digital_abstract_tail.pdf` | Full-width abstract opener + two-column body (IEEE regression) |
| `digital_injection.pdf` | Prompt injection → `injection_flagged` fallback |
| `digital_minimal.pdf` | Single-column fallback routing (`no_llm_configured`, unsupported provider) |
| `scanned_single_page.pdf` | Image-only page for rasterization / `Ocr` classification |
