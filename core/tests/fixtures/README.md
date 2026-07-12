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

## Supported layout classes

| Class | Support level |
|-------|----------------|
| Single-column reports (APA, business letters) | Full |
| IEEE/USENIX two-column academic papers | Full |
| Legal two-column flow documents | Full |
| Decorative brochures / marketing PDFs | Best-effort (`non_flow_document` warning possible) |
| Slide decks / presentation PDFs | Out of scope (warning only; no dedicated parser) |

| File | Purpose |
|------|---------|
| `digital_two_column.pdf` | Digital path, two-column layout ordering |
| `digital_abstract_tail.pdf` | Full-width abstract opener + two-column body (IEEE regression) |
| `digital_two_column_hanging_indent.pdf` | Two-column bibliography-style hanging indents |
| `digital_header_footer.pdf` | Header/Footer margin bands + chunk filtering |
| `digital_dense_footer_band.pdf` | Per-band line height splits footer/reference band |
| `digital_injection.pdf` | Prompt injection → `injection_flagged` fallback |
| `digital_minimal.pdf` | Single-column fallback routing (`no_llm_configured`, unsupported provider) |
| `digital_per_glyph_punctuation.pdf` | Punctuation-adjacent fragments: `Hello, world.` without spurious spaces |
| `digital_per_glyph_sentence.pdf` | Longer fragmented sentence with commas, semicolons, period |
| `digital_per_glyph_word.pdf` | One text object per letter; expects assembled word `Maximum` |
| `digital_tight_word_fragments.pdf` | Multi-letter fragments with sub-word-space gaps (`of this`, `brief overview`) |
| `digital_word_fragment_line.pdf` | Mid-word fragment merge (`Members chosen`) |
| `digital_form_nested_image.pdf` | Hybrid: top-level text + large image nested in Form XObject |
| `digital_form_nested_text_and_image.pdf` | Hybrid: text and image nested in form only (not Ocr-only) |
| `digital_vector_form_border.pdf` | Digital: vector path inside form, no nested images |
| `scanned_single_page.pdf` | Image-only page for rasterization / `Ocr` classification |

## OCR confidence calibration fixtures

Golden logit vectors for [`bundled.rs`](../../src/ocr/bundled.rs) `decode_ctc_logits_detailed` unit tests live in `ocr_confidence/`. Regenerate diagnostics with `AMBER_OCR_CONF_DEBUG=1` and `scripts/ocr_rec_parity.py`.

Regenerate form fixtures with `py -3.13 core/tests/fixtures/write_form_fixtures.py` (requires `pikepdf` + `Pillow`).

Extraction-hygiene fixtures (`digital_per_glyph_*`, `digital_word_fragment_line`) validate
[`core/src/ocr/pdf.rs`](../../src/ocr/pdf.rs) typographic join rules in CI — not real TestDocuments.
