#!/usr/bin/env python3
"""Paddle-style CTC decode parity check for Amber OCR rec.onnx (Phase A deliverable).

Usage:
  python scripts/ocr_rec_parity.py <path_to_crop.png>
  python scripts/ocr_rec_parity.py --models-dir ~/.amber/models/ocr <crop.png>

Compares decoded text and mean confidence against PP-OCRv6_small_rec_onnx using
CTCLabelDecode semantics (argmax + max per timestep, mean over selection mask).
"""

from __future__ import annotations

import argparse
import sys
from pathlib import Path

import numpy as np

try:
    import onnxruntime as ort
except ImportError:
    print("onnxruntime required: pip install onnxruntime", file=sys.stderr)
    sys.exit(1)

try:
    from PIL import Image
except ImportError:
    print("Pillow required: pip install pillow", file=sys.stderr)
    sys.exit(1)


def load_vocab(dict_path: Path) -> list[str]:
    return dict_path.read_text(encoding="utf-8").splitlines()


def preprocess_recognition_patch(img: Image.Image) -> np.ndarray:
    """Match Amber bundled.rs preprocess_recognition_patch."""
    target_h = 48
    pw, ph = img.size
    aspect = pw / max(ph, 1)
    target_w = max(16, int(round(target_h * aspect)))
    resized = img.resize((target_w, target_h), Image.Resampling.BILINEAR)
    rgb = np.array(resized.convert("RGB"), dtype=np.float32) / 255.0
    normalized = (rgb - 0.5) / 0.5  # HWC
    chw = np.transpose(normalized, (2, 0, 1))
    return np.expand_dims(chw, axis=0).astype(np.float32)


def classify_timestep(values: np.ndarray) -> str:
    s = float(values.sum())
    vmin, vmax = float(values.min()), float(values.max())
    if vmin >= 0.0 and vmax <= 1.0 and abs(s - 1.0) < 0.05:
        return "probabilities"
    if vmax <= 0.0:
        return "log_probabilities"
    return "logits"


def softmax(x: np.ndarray) -> np.ndarray:
    x = x - x.max()
    e = np.exp(x)
    return e / e.sum()


def token_scores_for_timestep(values: np.ndarray, kind: str) -> tuple[int, float, float, float]:
    """Return (argmax, peak_prob, margin_prob, logit_sep)."""
    argmax = int(values.argmax())
    if kind == "probabilities":
        peak = float(values[argmax])
        sorted_vals = np.sort(values)[::-1]
        margin = float(sorted_vals[0] - sorted_vals[1]) if len(sorted_vals) > 1 else peak
        logit_sep = margin
    elif kind == "log_probabilities":
        probs = np.exp(values - values.max())
        probs = probs / probs.sum()
        peak = float(probs[argmax])
        sorted_p = np.sort(probs)[::-1]
        margin = float(sorted_p[0] - sorted_p[1]) if len(sorted_p) > 1 else peak
        logit_sep = float(values[argmax] - np.partition(values, -2)[-2])
    else:
        probs = softmax(values)
        peak = float(probs[argmax])
        sorted_p = np.sort(probs)[::-1]
        margin = float(sorted_p[0] - sorted_p[1]) if len(sorted_p) > 1 else peak
        sorted_l = np.sort(values)[::-1]
        logit_sep = float(sorted_l[0] - sorted_l[1]) if len(sorted_l) > 1 else 0.0
    return argmax, peak, margin, logit_sep


def paddle_ctc_decode(preds: np.ndarray, vocab: list[str]) -> dict:
    """preds shape [1, T, C] or [1, C, T]."""
    if preds.ndim == 3 and preds.shape[1] > preds.shape[2]:
        # [1, C, T] -> [1, T, C]
        preds = np.transpose(preds, (0, 2, 1))
    if preds.ndim == 2:
        preds = np.expand_dims(preds, 0)

    batch = preds[0]  # [T, C]
    t0 = batch[0]
    kind = classify_timestep(t0)

  # Paddle: preds_idx = argmax, preds_prob = max (on raw preds)
    idx = batch.argmax(axis=1)
    raw_max = batch.max(axis=1)

    selection = np.ones(len(idx), dtype=bool)
    selection[1:] = idx[1:] != idx[:-1]
    selection &= idx != 0  # blank

    chars = []
    conf_list = []
    margins = []
    logit_seps = []
    for t in np.where(selection)[0]:
        i = int(idx[t])
        if i <= 0 or i > len(vocab):
            continue
        chars.append(vocab[i - 1])
        _, peak, margin, logit_sep = token_scores_for_timestep(batch[t], kind)
        conf_list.append(peak)
        margins.append(margin)
        logit_seps.append(logit_sep)

    text = "".join(chars)
    mean_conf = float(np.mean(conf_list)) if conf_list else 0.0
    mean_margin = float(np.mean(margins)) if margins else 0.0
    mean_logit_sep = float(np.mean(logit_seps)) if logit_seps else 0.0

    # Also report Paddle-naive max(axis=2) without softmax
    naive_conf = float(np.mean(raw_max[selection])) if selection.any() else 0.0

    return {
        "text": text,
        "mean_conf": mean_conf,
        "naive_max_conf": naive_conf,
        "mean_margin": mean_margin,
        "mean_logit_sep": mean_logit_sep,
        "output_kind": kind,
        "num_classes": batch.shape[1],
        "seq_len": batch.shape[0],
        "t0_min": float(t0.min()),
        "t0_max": float(t0.max()),
        "t0_sum": float(t0.sum()),
    }


def main() -> int:
    parser = argparse.ArgumentParser(description="OCR rec.onnx Paddle-style parity")
    parser.add_argument("image", type=Path, help="Cropped text line PNG")
    parser.add_argument(
        "--models-dir",
        type=Path,
        default=Path.home() / ".amber" / "models" / "ocr",
    )
    args = parser.parse_args()

    rec_path = args.models_dir / "rec.onnx"
    dict_path = args.models_dir / "ppocrv6_dict.txt"
    if not rec_path.is_file():
        print(f"Missing {rec_path}", file=sys.stderr)
        return 1
    if not dict_path.is_file():
        print(f"Missing {dict_path}", file=sys.stderr)
        return 1

    vocab = load_vocab(dict_path)
    img = Image.open(args.image)
    tensor = preprocess_recognition_patch(img)

    session = ort.InferenceSession(str(rec_path), providers=["CPUExecutionProvider"])
    input_name = session.get_inputs()[0].name
    outputs = session.run(None, {input_name: tensor})
    preds = outputs[0]

    result = paddle_ctc_decode(preds, vocab)
    print(f"image: {args.image}")
    print(f"rec_shape: {list(preds.shape)}")
    print(f"output_kind (t=0): {result['output_kind']}")
    print(f"t0 stats: min={result['t0_min']:.6f} max={result['t0_max']:.6f} sum={result['t0_sum']:.6f}")
    print(f"seq_len={result['seq_len']} num_classes={result['num_classes']}")
    print(f"text: {result['text']!r}")
    print(f"mean_conf (normalized): {result['mean_conf']:.8f}")
    print(f"naive_max_conf (raw max): {result['naive_max_conf']:.8f}")
    print(f"mean_margin: {result['mean_margin']:.8f}")
    print(f"mean_logit_sep: {result['mean_logit_sep']:.6f}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
