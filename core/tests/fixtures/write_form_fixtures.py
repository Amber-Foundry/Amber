#!/usr/bin/env python3
"""Generate Form XObject classification fixtures (requires pikepdf + Pillow)."""

from __future__ import annotations

import io
from pathlib import Path

import pikepdf
from pikepdf import Dictionary, Name, Stream
from PIL import Image

FIXTURES_DIR = Path(__file__).resolve().parent


def helvetica_font(pdf: pikepdf.Pdf) -> Dictionary:
    return pdf.make_indirect(
        Dictionary(
            Type=Name.Font,
            Subtype=Name.Type1,
            BaseFont=Name.Helvetica,
        )
    )


def jpeg_image(pdf: pikepdf.Pdf, width: int, height: int) -> Stream:
    img = Image.new("RGB", (width, height), color=(190, 48, 48))
    buf = io.BytesIO()
    img.save(buf, format="JPEG", quality=85)
    return Stream(
        pdf,
        buf.getvalue(),
        Type=Name.XObject,
        Subtype=Name.Image,
        Width=width,
        Height=height,
        ColorSpace=Name.DeviceRGB,
        BitsPerComponent=8,
        Filter=Name.DCTDecode,
    )


def make_form(
    pdf: pikepdf.Pdf,
    bbox: list[float],
    content: bytes,
    resources: Dictionary,
) -> Stream:
    return Stream(
        pdf,
        content,
        Type=Name.XObject,
        Subtype=Name.Form,
        BBox=bbox,
        Resources=resources,
    )


def write_page(
    pdf: pikepdf.Pdf,
    page_content: bytes,
    xobjects: dict[str, Stream],
) -> None:
    page = pdf.add_blank_page(page_size=(612, 792))
    font = helvetica_font(pdf)
    resources = Dictionary(
        Font=Dictionary(F1=font),
        XObject=Dictionary({f"/{k}": v for k, v in xobjects.items()}),
    )
    page.obj.Resources = resources
    page.obj.Contents = pdf.make_indirect(Stream(pdf, page_content))


def main() -> None:
    # Top-level text + large nested image (Hybrid)
    pdf = pikepdf.Pdf.new()
    img = jpeg_image(pdf, 320, 220)
    form = make_form(
        pdf,
        [0, 0, 320, 220],
        b"q 320 0 0 220 0 0 cm /FmImg Do Q",
        Dictionary(XObject=Dictionary({"/FmImg": img})),
    )
    write_page(
        pdf,
        b"BT /F1 14 Tf 72 720 Td (FORM_NESTED_IMAGE_FIXTURE_TITLE) Tj ET "
        b"q 1 0 0 1 72 380 cm /NestedForm Do Q",
        {"NestedForm": form},
    )
    out = FIXTURES_DIR / "digital_form_nested_image.pdf"
    pdf.save(out)
    print(f"wrote {out}")

    # Text + image nested in form only (Hybrid, not Ocr-only)
    pdf = pikepdf.Pdf.new()
    img = jpeg_image(pdf, 300, 200)
    form = make_form(
        pdf,
        [0, 0, 300, 260],
        b"BT /F1 12 Tf 10 230 Td (FORM_NESTED_TEXT_AND_IMAGE_TITLE) Tj ET "
        b"q 300 0 0 200 10 10 cm /FmImg Do Q",
        Dictionary(
            Font=Dictionary(F1=helvetica_font(pdf)),
            XObject=Dictionary({"/FmImg": img}),
        ),
    )
    write_page(
        pdf,
        b"q 1 0 0 1 72 400 cm /NestedForm Do Q",
        {"NestedForm": form},
    )
    out = FIXTURES_DIR / "digital_form_nested_text_and_image.pdf"
    pdf.save(out)
    print(f"wrote {out}")

    # Top-level text + vector-only form border (Digital)
    pdf = pikepdf.Pdf.new()
    form = make_form(
        pdf,
        [0, 0, 200, 120],
        b"q 1 w 0 0 m 200 0 l 200 120 l 0 120 l h S Q",
        Dictionary(),
    )
    write_page(
        pdf,
        b"BT /F1 14 Tf 72 720 Td (FORM_VECTOR_BORDER_FIXTURE_TITLE) Tj ET "
        b"q 1 0 0 1 72 500 cm /NestedForm Do Q",
        {"NestedForm": form},
    )
    out = FIXTURES_DIR / "digital_vector_form_border.pdf"
    pdf.save(out)
    print(f"wrote {out}")


if __name__ == "__main__":
    main()
