use crate::ocr::engine::{OcrEngine, OcrError, OcrOutput, OcrTextBlock, Rect};
use crate::ocr::{download_ocr_models_if_needed, load_ocr_registry, ocr_models_dir};
use image::GenericImageView;
use ort::execution_providers::CPUExecutionProvider;
#[cfg(target_os = "windows")]
use ort::execution_providers::DirectMLExecutionProvider;
use ort::session::{builder::GraphOptimizationLevel, Session};
use ort::value::Tensor;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

/// Bundled ONNX implementation of `OcrEngine` using PP-OCR models via `ort`.
pub struct BundledOcrEngine {
    det_session: Session,
    rec_session: Session,
    vocab: Vec<String>,
}

impl BundledOcrEngine {
    /// Loads DBNet (`det.onnx`), CRNN (`rec.onnx`), and vocabulary (`ppocrv6_dict.txt`)
    /// from default models directory `~/.amber/models/ocr/`.
    pub fn new() -> Result<Self, OcrError> {
        download_ocr_models_if_needed()?;
        let dir = ocr_models_dir()?;
        let registry = load_ocr_registry()?;

        let det_path = dir.join(&registry.det.filename);
        let rec_path = dir.join(&registry.rec.filename);
        let dict_path = dir.join(&registry.dict.filename);

        Self::from_paths(&det_path, &rec_path, &dict_path)
    }

    /// Loads DBNet (`det_path`), CRNN (`rec_path`), and vocabulary (`dict_path`)
    /// from specified file paths.
    pub fn from_paths(
        det_path: &Path,
        rec_path: &Path,
        dict_path: &Path,
    ) -> Result<Self, OcrError> {
        ensure_file_exists(det_path, "Detection model")?;
        ensure_file_exists(rec_path, "Recognition model")?;
        ensure_file_exists(dict_path, "Vocabulary dictionary")?;

        let det_session = build_session(det_path)?;
        let rec_session = build_session(rec_path)?;
        let vocab = load_vocab(dict_path)?;

        Ok(Self {
            det_session,
            rec_session,
            vocab,
        })
    }

    /// Pre-processes `image` for DBNet detection: resizes long side to max 960px (padded to multiple of 32)
    /// and normalizes to standard ImageNet mean/std float array.
    fn preprocess_detection(image: &image::DynamicImage) -> (Vec<f32>, usize, usize, f32, f32) {
        let (orig_w, orig_h) = image.dimensions();
        let max_side = 960.0f32;
        let scale = if (orig_w as f32) > max_side || (orig_h as f32) > max_side {
            max_side / ((orig_w as f32).max(orig_h as f32))
        } else {
            1.0f32
        };

        let target_w_unpadded = (orig_w as f32 * scale).round() as usize;
        let target_h_unpadded = (orig_h as f32 * scale).round() as usize;

        // Pad to nearest multiple of 32
        let target_w = target_w_unpadded.div_ceil(32) * 32;
        let target_h = target_h_unpadded.div_ceil(32) * 32;

        let resized = image.resize_exact(
            target_w_unpadded as u32,
            target_h_unpadded as u32,
            image::imageops::FilterType::Triangle,
        );

        let rgb = resized.to_rgb8();

        let mut tensor_data = vec![0.0f32; 3 * target_h * target_w];
        let mean = [0.485f32, 0.456f32, 0.406f32];
        let std = [0.229f32, 0.224f32, 0.225f32];

        let plane_size = target_h * target_w;
        for y in 0..target_h_unpadded {
            for x in 0..target_w_unpadded {
                let pixel = rgb.get_pixel(x as u32, y as u32);
                let idx = y * target_w + x;
                tensor_data[idx] = ((pixel[0] as f32 / 255.0) - mean[0]) / std[0];
                tensor_data[plane_size + idx] = ((pixel[1] as f32 / 255.0) - mean[1]) / std[1];
                tensor_data[2 * plane_size + idx] = ((pixel[2] as f32 / 255.0) - mean[2]) / std[2];
            }
        }

        (
            tensor_data,
            target_w,
            target_h,
            orig_w as f32 / target_w_unpadded as f32,
            orig_h as f32 / target_h_unpadded as f32,
        )
    }

    /// Extract bounding boxes from probability heatmap using connected component labeling.
    fn extract_boxes_from_heatmap(
        heatmap: &[f32],
        width: usize,
        height: usize,
        scale_x: f32,
        scale_y: f32,
        thresh: f32,
    ) -> Vec<Rect> {
        if width == 0 || height == 0 || heatmap.len() < width * height {
            return Vec::new();
        }

        let mut visited = vec![false; width * height];
        let mut bboxes = Vec::new();

        // 8-neighbor connectivity directions
        let dx: [isize; 8] = [-1, 0, 1, -1, 1, -1, 0, 1];
        let dy: [isize; 8] = [-1, -1, -1, 0, 0, 1, 1, 1];

        let min_component_area = 15; // Filter out tiny noise specks

        for y in 0..height {
            for x in 0..width {
                let idx = y * width + x;
                if !visited[idx] && heatmap[idx] > thresh {
                    let mut queue = std::collections::VecDeque::new();
                    queue.push_back((x, y));
                    visited[idx] = true;

                    let mut min_x = x;
                    let mut max_x = x;
                    let mut min_y = y;
                    let mut max_y = y;
                    let mut count = 0usize;

                    while let Some((cx, cy)) = queue.pop_front() {
                        count += 1;
                        min_x = min_x.min(cx);
                        max_x = max_x.max(cx);
                        min_y = min_y.min(cy);
                        max_y = max_y.max(cy);

                        for i in 0..8 {
                            let nx = cx as isize + dx[i];
                            let ny = cy as isize + dy[i];

                            if nx >= 0 && nx < width as isize && ny >= 0 && ny < height as isize {
                                let n_idx = ny as usize * width + nx as usize;
                                if !visited[n_idx] && heatmap[n_idx] > thresh {
                                    visited[n_idx] = true;
                                    queue.push_back((nx as usize, ny as usize));
                                }
                            }
                        }
                    }

                    if count >= min_component_area {
                        // Expand bounding box slightly (3px pad X, 2px pad Y) to avoid clipping text edges
                        let pad_x = 3.0f32;
                        let pad_y = 2.0f32;

                        let box_min_x = ((min_x as f32) - pad_x).max(0.0);
                        let box_min_y = ((min_y as f32) - pad_y).max(0.0);
                        let box_max_x = ((max_x as f32) + pad_x).min((width - 1) as f32);
                        let box_max_y = ((max_y as f32) + pad_y).min((height - 1) as f32);

                        let box_w = (box_max_x - box_min_x + 1.0).max(1.0);
                        let box_h = (box_max_y - box_min_y + 1.0).max(1.0);

                        let rect = Rect::new(
                            box_min_x * scale_x,
                            box_min_y * scale_y,
                            box_w * scale_x,
                            box_h * scale_y,
                        );
                        bboxes.push(rect);
                    }
                }
            }
        }

        // Sort bounding boxes top-to-bottom, left-to-right in natural reading order using strict total ordering
        bboxes.sort_by(|a, b| {
            let row_a = (a.y / 15.0).floor() as i32;
            let row_b = (b.y / 15.0).floor() as i32;
            match row_a.cmp(&row_b) {
                std::cmp::Ordering::Equal => {
                    a.x.partial_cmp(&b.x).unwrap_or(std::cmp::Ordering::Equal)
                }
                other => other,
            }
        });

        bboxes
    }

    /// Pre-processes text patch crop for CRNN recognition `[1, 3, 48, target_w]`.
    fn preprocess_recognition_patch(patch: &image::DynamicImage) -> (Vec<f32>, usize, usize) {
        let (pw, ph) = patch.dimensions();
        let target_h = 48usize;
        let aspect = pw as f32 / ph.max(1) as f32;
        let target_w = (target_h as f32 * aspect).round().max(16.0) as usize;

        let resized = patch.resize_exact(
            target_w as u32,
            target_h as u32,
            image::imageops::FilterType::Triangle,
        );
        let rgb = resized.to_rgb8();

        let mut tensor_data = vec![0.0f32; 3 * target_h * target_w];
        let plane_size = target_h * target_w;

        for y in 0..target_h {
            for x in 0..target_w {
                let pixel = rgb.get_pixel(x as u32, y as u32);
                let idx = y * target_w + x;
                tensor_data[idx] = (pixel[0] as f32 / 255.0 - 0.5) / 0.5;
                tensor_data[plane_size + idx] = (pixel[1] as f32 / 255.0 - 0.5) / 0.5;
                tensor_data[2 * plane_size + idx] = (pixel[2] as f32 / 255.0 - 0.5) / 0.5;
            }
        }

        (tensor_data, target_w, target_h)
    }
}

/// How CRNN ONNX output values should be interpreted at each timestep.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecOutputKind {
    Logits,
    Probabilities,
    LogProbabilities,
}

/// Result of CTC greedy decode with diagnostic scores.
///
/// `confidence` is always Paddle CTCLabelDecode semantics: arithmetic mean of per-emitted-token
/// peak probabilities in [0, 1]. See `core/tests/fixtures/ocr_confidence/PHASE_A.md` for why
/// this single metric was chosen after the Phase A spike (Path A / B1 — no per-block switching).
#[derive(Debug, Clone, PartialEq)]
pub struct CtcDecodeResult {
    pub text: String,
    /// Mean per-emitted-token peak probability (Paddle `np.mean(conf_list)`).
    pub confidence: f32,
    /// Diagnostic only (`AMBER_OCR_CONF_DEBUG`); mean `(p1 - p2)` per emitted token.
    pub mean_margin: f32,
}

const BLANK_INDEX: usize = 0;

fn ocr_confidence_debug_enabled() -> bool {
    std::env::var("AMBER_OCR_CONF_DEBUG")
        .ok()
        .is_some_and(|v| v == "1")
}

fn classify_rec_timestep(values: &[f32]) -> RecOutputKind {
    if values.is_empty() {
        return RecOutputKind::Logits;
    }
    let sum: f32 = values.iter().sum();
    let min = values.iter().copied().fold(f32::INFINITY, f32::min);
    let max = values.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    if min >= 0.0 && max <= 1.0 && (sum - 1.0).abs() < 0.05 {
        RecOutputKind::Probabilities
    } else if max <= 0.0 {
        RecOutputKind::LogProbabilities
    } else {
        RecOutputKind::Logits
    }
}

fn softmax_peak_and_margin(values: &[f32]) -> (f32, f32, f32) {
    let mut max_val = f32::NEG_INFINITY;
    let mut max_idx = 0usize;
    for (idx, &val) in values.iter().enumerate() {
        if val > max_val {
            max_val = val;
            max_idx = idx;
        }
    }
    let mut exp_sum = 0.0f32;
    for &val in values {
        exp_sum += (val - max_val).exp();
    }
    let peak = if exp_sum > 0.0 {
        (1.0 / exp_sum).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let mut runner_up = f32::NEG_INFINITY;
    for (idx, &val) in values.iter().enumerate() {
        if idx != max_idx && val > runner_up {
            runner_up = val;
        }
    }
    let runner_up_prob = if runner_up.is_finite() && exp_sum > 0.0 {
        ((runner_up - max_val).exp() / exp_sum).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let logit_sep = if runner_up.is_finite() {
        max_val - runner_up
    } else {
        0.0
    };
    (peak, (peak - runner_up_prob).max(0.0), logit_sep)
}

fn timestep_peak_and_margin(values: &[f32], kind: RecOutputKind) -> (f32, f32, f32) {
    match kind {
        RecOutputKind::Logits => softmax_peak_and_margin(values),
        RecOutputKind::Probabilities => {
            let mut max_val = f32::NEG_INFINITY;
            let mut max_idx = 0usize;
            for (idx, &val) in values.iter().enumerate() {
                if val > max_val {
                    max_val = val;
                    max_idx = idx;
                }
            }
            let peak = max_val.clamp(0.0, 1.0);
            let mut runner_up = 0.0f32;
            for (idx, &val) in values.iter().enumerate() {
                if idx != max_idx {
                    runner_up = runner_up.max(val);
                }
            }
            (peak, (peak - runner_up).max(0.0), peak - runner_up)
        }
        RecOutputKind::LogProbabilities => {
            let mut max_log = f32::NEG_INFINITY;
            for &val in values {
                if val > max_log {
                    max_log = val;
                }
            }
            let mut exp_sum = 0.0f32;
            for &val in values {
                exp_sum += (val - max_log).exp();
            }
            let mut max_idx = 0usize;
            let mut max_val = f32::NEG_INFINITY;
            for (idx, &val) in values.iter().enumerate() {
                if val > max_val {
                    max_val = val;
                    max_idx = idx;
                }
            }
            let peak = if exp_sum > 0.0 {
                ((max_val - max_log).exp() / exp_sum).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let mut runner_up_log = f32::NEG_INFINITY;
            for (idx, &val) in values.iter().enumerate() {
                if idx != max_idx && val > runner_up_log {
                    runner_up_log = val;
                }
            }
            let runner_up_prob = if runner_up_log.is_finite() && exp_sum > 0.0 {
                ((runner_up_log - max_log).exp() / exp_sum).clamp(0.0, 1.0)
            } else {
                0.0
            };
            (
                peak,
                (peak - runner_up_prob).max(0.0),
                if runner_up_log.is_finite() {
                    max_val - runner_up_log
                } else {
                    0.0
                },
            )
        }
    }
}

/// Paddle-style CTC greedy decode with output-kind detection (Phase A Path A / B1).
pub fn decode_ctc_logits(
    logits: &[f32],
    seq_len: usize,
    num_classes: usize,
    vocab: &[String],
) -> (String, f32) {
    let result = decode_ctc_logits_detailed(logits, seq_len, num_classes, vocab);
    (result.text, result.confidence)
}

pub fn decode_ctc_logits_detailed(
    preds: &[f32],
    seq_len: usize,
    num_classes: usize,
    vocab: &[String],
) -> CtcDecodeResult {
    if seq_len == 0 || num_classes == 0 || preds.len() < seq_len * num_classes {
        return CtcDecodeResult {
            text: String::new(),
            confidence: 0.0,
            mean_margin: 0.0,
        };
    }

    let kind = classify_rec_timestep(&preds[..num_classes]);

    let mut indices = Vec::with_capacity(seq_len);
    for t in 0..seq_len {
        let slice = &preds[t * num_classes..(t + 1) * num_classes];
        let mut max_idx = 0usize;
        let mut max_val = f32::NEG_INFINITY;
        for (idx, &val) in slice.iter().enumerate() {
            if val > max_val {
                max_val = val;
                max_idx = idx;
            }
        }
        indices.push(max_idx);
    }

    let mut selection = vec![true; seq_len];
    for t in 1..seq_len {
        selection[t] = indices[t] != indices[t - 1];
    }
    for t in 0..seq_len {
        if indices[t] == BLANK_INDEX {
            selection[t] = false;
        }
    }

    let mut text = String::new();
    let mut prob_sum = 0.0f32;
    let mut margin_sum = 0.0f32;
    let mut token_count = 0usize;

    for t in 0..seq_len {
        if !selection[t] {
            continue;
        }
        let class_idx = indices[t];
        if class_idx == 0 || class_idx > vocab.len() {
            continue;
        }
        if let Some(ch) = vocab.get(class_idx - 1) {
            text.push_str(ch);
            let slice = &preds[t * num_classes..(t + 1) * num_classes];
            let (peak, margin, _) = timestep_peak_and_margin(slice, kind);
            prob_sum += peak;
            margin_sum += margin;
            token_count += 1;
        }
    }

    if token_count == 0 {
        return CtcDecodeResult {
            text: String::new(),
            confidence: 0.0,
            mean_margin: 0.0,
        };
    }

    let mean_probability = (prob_sum / token_count as f32).clamp(0.0, 1.0);
    let mean_margin = margin_sum / token_count as f32;

    CtcDecodeResult {
        text,
        confidence: mean_probability,
        mean_margin,
    }
}

fn log_rec_block_debug(
    block_index: usize,
    rec_shape: &[i64],
    seq_len: usize,
    num_classes: usize,
    is_ct_layout: bool,
    decode: &CtcDecodeResult,
    preds: &[f32],
) {
    if !ocr_confidence_debug_enabled() {
        return;
    }
    let t0 = &preds[..num_classes.min(preds.len())];
    let kind = classify_rec_timestep(t0);
    let t0_min = t0.iter().copied().fold(f32::INFINITY, f32::min);
    let t0_max = t0.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    let t0_sum: f32 = t0.iter().sum();
    let (_, _, logit_sep) = timestep_peak_and_margin(t0, kind);
    let preview: String = decode.text.chars().take(48).collect();
    eprintln!(
        "[ocr-rec] block={block_index} shape={rec_shape:?} seq={seq_len} classes={num_classes} \
         ct_layout={is_ct_layout} kind={kind:?} t0_min={t0_min:.6} t0_max={t0_max:.6} t0_sum={t0_sum:.6} \
         logit_sep={logit_sep:.4} mean_margin={:.8} conf={:.8} text={preview:?}",
        decode.mean_margin,
        decode.confidence,
    );
}

fn parse_rec_output_shape(rec_shape: &[i64]) -> Result<(usize, usize, bool), OcrError> {
    match rec_shape {
        [1, c, t] if *c > *t => Ok((*t as usize, *c as usize, true)),
        [1, t, c] => Ok((*t as usize, *c as usize, false)),
        [t, 1, c] => Ok((*t as usize, *c as usize, false)),
        [t, c] => Ok((*t as usize, *c as usize, false)),
        other => Err(OcrError::InferenceFailed(format!(
            "unsupported CRNN output shape {other:?}; expected [1,T,C], [1,C,T], [T,1,C], or [T,C]"
        ))),
    }
}

impl OcrEngine for BundledOcrEngine {
    fn recognize(&self, image: &image::DynamicImage) -> Result<OcrOutput, OcrError> {
        let (img_w, img_h) = image.dimensions();
        if img_w == 0 || img_h == 0 {
            return Ok(OcrOutput::new(Vec::new()));
        }

        let (det_tensor, det_w, det_h, scale_x, scale_y) = Self::preprocess_detection(image);

        let shape = vec![1i64, 3, det_h as i64, det_w as i64];
        let input_tensor = Tensor::from_array((shape, det_tensor))?;
        let inputs = ort::inputs! {
            "x" => input_tensor
        }?;

        let outputs = self.det_session.run(inputs)?;

        let det_output_tensor = outputs
            .values()
            .next()
            .ok_or_else(|| OcrError::InferenceFailed("DBNet returned no outputs".to_string()))?;

        let (_heatmap_shape, heatmap_data) = det_output_tensor
            .try_extract_raw_tensor::<f32>()
            .map_err(|err| {
                OcrError::InferenceFailed(format!("failed extracting DBNet output: {err}"))
            })?;

        let bboxes =
            Self::extract_boxes_from_heatmap(heatmap_data, det_w, det_h, scale_x, scale_y, 0.3f32);

        if bboxes.is_empty() {
            return Ok(OcrOutput::new(Vec::new()));
        }

        let mut text_blocks = Vec::with_capacity(bboxes.len());
        let (img_w, img_h) = image.dimensions();

        for (block_index, bbox) in bboxes.into_iter().enumerate() {
            let crop_x = (bbox.x.max(0.0) as u32).min(img_w.saturating_sub(1));
            let crop_y = (bbox.y.max(0.0) as u32).min(img_h.saturating_sub(1));
            let crop_w = (bbox.width as u32).min(img_w - crop_x).max(1);
            let crop_h = (bbox.height as u32).min(img_h - crop_y).max(1);

            let cropped_patch = image.crop_imm(crop_x, crop_y, crop_w, crop_h);
            let (rec_tensor, rec_w, rec_h) = Self::preprocess_recognition_patch(&cropped_patch);

            let rec_input_shape = vec![1i64, 3, rec_h as i64, rec_w as i64];
            let rec_input_tensor = Tensor::from_array((rec_input_shape, rec_tensor))?;
            let rec_inputs = ort::inputs! {
                "x" => rec_input_tensor
            }?;

            let rec_outputs = self.rec_session.run(rec_inputs)?;

            let rec_output_tensor = rec_outputs
                .values()
                .next()
                .ok_or_else(|| OcrError::InferenceFailed("CRNN returned no outputs".to_string()))?;

            let (rec_shape, rec_data) =
                rec_output_tensor
                    .try_extract_raw_tensor::<f32>()
                    .map_err(|err| {
                        OcrError::InferenceFailed(format!("failed extracting CRNN output: {err}"))
                    })?;

            let (seq_len, num_classes, is_ct_layout) = parse_rec_output_shape(rec_shape)?;

            let mut transposed_logits;
            let logits_slice = if is_ct_layout {
                transposed_logits = vec![0.0f32; seq_len * num_classes];
                for c in 0..num_classes {
                    for t in 0..seq_len {
                        transposed_logits[t * num_classes + c] = rec_data[c * seq_len + t];
                    }
                }
                &transposed_logits[..]
            } else {
                rec_data
            };

            let decode =
                decode_ctc_logits_detailed(logits_slice, seq_len, num_classes, &self.vocab);
            if block_index < 3 {
                log_rec_block_debug(
                    block_index,
                    rec_shape,
                    seq_len,
                    num_classes,
                    is_ct_layout,
                    &decode,
                    logits_slice,
                );
            }

            if !decode.text.trim().is_empty() {
                text_blocks.push(OcrTextBlock::new(decode.text, bbox, decode.confidence));
            }
        }

        Ok(OcrOutput::new(text_blocks))
    }
}

fn build_session(model_path: &Path) -> Result<Session, OcrError> {
    #[cfg(target_os = "windows")]
    {
        let builder = Session::builder()
            .and_then(|builder| builder.with_optimization_level(GraphOptimizationLevel::Level3))
            .map_err(|err| {
                OcrError::InferenceFailed(format!("failed to create ONNX session builder: {err}"))
            })?;
        match builder
            .with_execution_providers([DirectMLExecutionProvider::default().build()])
            .and_then(|builder| builder.commit_from_file(model_path))
        {
            Ok(session) => return Ok(session),
            Err(err) => {
                eprintln!("Warning: DirectML OCR session failed, falling back to CPU: {err}");
            }
        }
    }

    let builder = Session::builder()
        .and_then(|builder| builder.with_optimization_level(GraphOptimizationLevel::Level3))
        .map_err(|err| {
            OcrError::InferenceFailed(format!("failed to create ONNX session builder: {err}"))
        })?;
    builder
        .with_execution_providers([CPUExecutionProvider::default().build()])
        .and_then(|builder| builder.commit_from_file(model_path))
        .map_err(|err| {
            OcrError::InferenceFailed(format!(
                "failed to load ONNX model {}: {err}",
                model_path.display()
            ))
        })
}

fn load_vocab(dict_path: &Path) -> Result<Vec<String>, OcrError> {
    let file = File::open(dict_path).map_err(|err| {
        OcrError::IoError(format!(
            "failed opening dict file {}: {err}",
            dict_path.display()
        ))
    })?;

    let reader = BufReader::new(file);
    let mut vocab = Vec::new();
    for line in reader.lines() {
        let l = line.map_err(|err| OcrError::IoError(err.to_string()))?;
        vocab.push(l);
    }

    Ok(vocab)
}

fn ensure_file_exists(path: &Path, label: &str) -> Result<(), OcrError> {
    if path.is_file() {
        Ok(())
    } else {
        Err(OcrError::ModelNotFound(format!(
            "{} missing at {}",
            label,
            path.display()
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageBuffer, Rgb};
    use std::path::PathBuf;

    #[derive(serde::Deserialize)]
    struct LogitFixture {
        seq_len: usize,
        num_classes: usize,
        vocab: Vec<String>,
        logits: Vec<f32>,
        expected_text: String,
        #[serde(default)]
        min_confidence: Option<f32>,
        #[serde(default)]
        max_confidence: Option<f32>,
    }

    fn sharp_logits_ab() -> Vec<f32> {
        vec![
            0.0, 10.0, 0.0, 0.0, 0.0, 10.0, 0.0, 0.0, 0.0, 0.0, 10.0, 0.0,
        ]
    }

    fn load_logit_fixture(name: &str) -> LogitFixture {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/ocr_confidence")
            .join(name);
        let json = std::fs::read_to_string(&path)
            .unwrap_or_else(|err| panic!("failed to read {}: {err}", path.display()));
        serde_json::from_str(&json)
            .unwrap_or_else(|err| panic!("failed to parse {}: {err}", path.display()))
    }

    fn constitution_pdf_path() -> Option<PathBuf> {
        if let Ok(path) = std::env::var("CONSTITUTION_PDF") {
            let path = PathBuf::from(path);
            if path.is_file() {
                return Some(path);
            }
        }
        [
            PathBuf::from(r"C:\Users\aashi\Desktop\TestDocuments\constitution.pdf"),
            PathBuf::from("../TestDocuments/constitution.pdf"),
        ]
        .into_iter()
        .find(|candidate| candidate.is_file())
    }

    #[test]
    fn test_ctc_decoder_empty() {
        let (text, conf) = decode_ctc_logits(&[], 0, 0, &[]);
        assert_eq!(text, "");
        assert_eq!(conf, 0.0);
    }

    #[test]
    fn test_ctc_decoder_basic() {
        let vocab = vec!["A".to_string(), "B".to_string(), "C".to_string()];
        let (text, conf) = decode_ctc_logits(&sharp_logits_ab(), 3, 4, &vocab);
        assert_eq!(text, "AB");
        assert!(conf > 0.9);
    }

    #[test]
    fn test_ctc_decoder_from_sharp_logits_fixture() {
        let fixture = load_logit_fixture("sharp_logits.json");
        let result = decode_ctc_logits_detailed(
            &fixture.logits,
            fixture.seq_len,
            fixture.num_classes,
            &fixture.vocab,
        );
        assert_eq!(result.text, fixture.expected_text);
        if let Some(min) = fixture.min_confidence {
            assert!(
                result.confidence >= min,
                "confidence {} < min {}",
                result.confidence,
                min
            );
        }
    }

    #[test]
    fn test_ctc_decoder_from_flat_logits_fixture() {
        let fixture = load_logit_fixture("flat_logits.json");
        let result = decode_ctc_logits_detailed(
            &fixture.logits,
            fixture.seq_len,
            fixture.num_classes,
            &fixture.vocab,
        );
        assert_eq!(result.text, fixture.expected_text);
        if let Some(max) = fixture.max_confidence {
            assert!(
                result.confidence <= max,
                "confidence {} > max {}",
                result.confidence,
                max
            );
        }
    }

    #[test]
    fn test_ctc_empty_emitted_returns_zero_confidence() {
        let vocab = vec!["A".to_string()];
        let logits = vec![10.0, 0.0, 10.0, 0.0, 10.0, 0.0];
        let result = decode_ctc_logits_detailed(&logits, 3, 2, &vocab);
        assert_eq!(result.text, "");
        assert_eq!(result.confidence, 0.0);
    }

    #[test]
    fn test_ctc_paddle_duplicate_mask_parity() {
        let vocab = vec!["A".to_string(), "B".to_string()];
        let logits = vec![
            10.0, 0.0, 0.0, 0.0, 10.0, 0.0, 0.0, 10.0, 0.0, 0.0, 0.0, 10.0, 10.0, 0.0, 0.0,
        ];
        let result = decode_ctc_logits_detailed(&logits, 5, 3, &vocab);
        assert_eq!(result.text, "AB");
        assert!(result.confidence > 0.9);
    }

    #[test]
    fn test_ctc_discriminates_peaked_vs_flat() {
        let vocab = vec!["A".to_string(), "B".to_string()];
        let peaked_result = decode_ctc_logits_detailed(&sharp_logits_ab(), 3, 4, &vocab);
        let flat_result = decode_ctc_logits_detailed(&[0.0f32; 12], 3, 4, &vocab);
        assert!(
            peaked_result.confidence > flat_result.confidence * 5.0,
            "peaked={} flat={}",
            peaked_result.confidence,
            flat_result.confidence
        );
    }

    #[test]
    fn test_ctc_probabilities_not_double_softmaxed() {
        let vocab = vec!["A".to_string(), "B".to_string()];
        let probs = vec![
            0.01, 0.97, 0.01, 0.01, 0.01, 0.97, 0.01, 0.01, 0.01, 0.01, 0.97, 0.01,
        ];
        let result = decode_ctc_logits_detailed(&probs, 3, 4, &vocab);
        assert_eq!(result.text, "AB");
        assert!(
            result.confidence > 0.9,
            "should use probability peak directly, got {}",
            result.confidence
        );
    }

    #[test]
    fn test_preprocess_detection_dimensions() {
        let img =
            image::DynamicImage::ImageRgb8(ImageBuffer::from_pixel(100, 200, Rgb([255, 255, 255])));

        let (data, target_w, target_h, scale_x, scale_y) =
            BundledOcrEngine::preprocess_detection(&img);
        assert_eq!(target_w % 32, 0);
        assert_eq!(target_h % 32, 0);
        assert_eq!(data.len(), 3 * target_w * target_h);
        assert!(scale_x > 0.0);
        assert!(scale_y > 0.0);
    }

    #[test]
    fn test_parse_rec_output_shape_rejects_unknown() {
        match parse_rec_output_shape(&[2, 3, 4, 5]) {
            Err(err) => assert!(err.to_string().contains("unsupported CRNN output shape")),
            Ok(_) => panic!("expected unsupported shape error"),
        }
    }

    #[test]
    #[ignore = "requires downloaded OCR models"]
    fn test_bundled_ocr_engine_real_models_inference() -> Result<(), Box<dyn std::error::Error>> {
        if !crate::ocr::ocr_models_exist() {
            return Ok(());
        }

        let engine = BundledOcrEngine::new().map_err(Box::<dyn std::error::Error>::from)?;
        let test_img =
            image::DynamicImage::ImageRgb8(ImageBuffer::from_pixel(300, 100, Rgb([255, 255, 255])));

        let output = engine
            .recognize(&test_img)
            .map_err(Box::<dyn std::error::Error>::from)?;
        assert!(output.avg_confidence <= 1.0);
        Ok(())
    }

    /// Phase A validation: preamble OCR block must outrank seal garbage on constitution page 1
    /// (PDF page index 1 — the preamble scan, not the cover).
    ///
    /// Recorded 2026-07-12 spike (Path A / B1): seal `DelheJeoble` conf=0.7316,
    /// preamble `WethePeopleofthe...` conf=0.9997. See PHASE_A.md.
    #[test]
    #[ignore = "requires downloaded OCR models and constitution.pdf (set CONSTITUTION_PDF)"]
    fn test_real_model_constitution_discrimination() -> Result<(), Box<dyn std::error::Error>> {
        if !crate::ocr::ocr_models_exist() {
            return Ok(());
        }
        let Some(pdf_path) = constitution_pdf_path() else {
            return Ok(());
        };

        use crate::ocr::pdf::{PdfRasterizer, PdfRasterizerConfig};

        let rasterizer = PdfRasterizer::new().map_err(Box::<dyn std::error::Error>::from)?;
        let document = rasterizer
            .load_document_from_file(&pdf_path)
            .map_err(Box::<dyn std::error::Error>::from)?;
        let page_img = rasterizer
            .render_loaded_page(&document, 1, &PdfRasterizerConfig { dpi: 300 })
            .map_err(Box::<dyn std::error::Error>::from)?;

        let engine = BundledOcrEngine::new().map_err(Box::<dyn std::error::Error>::from)?;
        let output = engine
            .recognize(&page_img)
            .map_err(Box::<dyn std::error::Error>::from)?;

        let seal = output
            .blocks
            .iter()
            .find(|b| b.text.contains("DelheJeoble") || b.text.contains("Jeoble"))
            .ok_or("expected seal garbage block on constitution page 1")?;
        let preamble = output
            .blocks
            .iter()
            .find(|b| b.text.contains("WethePeople"))
            .ok_or("expected preamble block on constitution page 1")?;

        assert!(
            preamble.confidence > seal.confidence,
            "preamble conf {} must exceed seal conf {} (Phase A ranking criterion)",
            preamble.confidence,
            seal.confidence
        );
        assert!(
            preamble.confidence > 0.9,
            "preamble should be high-confidence after B1 fix, got {}",
            preamble.confidence
        );
        assert!(
            seal.confidence < preamble.confidence,
            "seal should rank below preamble; seal={} preamble={}",
            seal.confidence,
            preamble.confidence
        );
        Ok(())
    }
}
