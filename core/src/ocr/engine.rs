use crate::models::download::DownloadError;
use image::DynamicImage;
use serde::{Deserialize, Serialize};
use std::fmt;

/// Standard bounding rectangle for OCR text block detection.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Rect {
    pub fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }
}

/// A recognized text block with bounding box and confidence score in [0.0, 1.0].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OcrTextBlock {
    pub text: String,
    pub bbox: Rect,
    /// Model confidence rating for this text block, constrained to [0.0, 1.0].
    pub confidence: f32,
}

impl OcrTextBlock {
    pub fn new(text: impl Into<String>, bbox: Rect, confidence: f32) -> Self {
        debug_assert!(
            (0.0..=1.0).contains(&confidence),
            "Confidence rating must be between 0.0 and 1.0, got {confidence}"
        );
        Self {
            text: text.into(),
            bbox,
            confidence: confidence.clamp(0.0, 1.0),
        }
    }
}

/// Aggregated output from an OCR recognition pass over an image.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OcrOutput {
    pub blocks: Vec<OcrTextBlock>,
    /// Average confidence rating across all text blocks, constrained to [0.0, 1.0].
    pub avg_confidence: f32,
}

impl OcrOutput {
    pub fn new(blocks: Vec<OcrTextBlock>) -> Self {
        let avg_confidence = if blocks.is_empty() {
            1.0
        } else {
            let total: f32 = blocks.iter().map(|b| b.confidence).sum();
            total / (blocks.len() as f32)
        };
        Self {
            blocks,
            avg_confidence: avg_confidence.clamp(0.0, 1.0),
        }
    }
}

#[derive(Debug)]
pub enum OcrError {
    ModelNotFound(String),
    InferenceFailed(String),
    DownloadFailed(DownloadError),
    IoError(String),
    Cancelled,
}

impl fmt::Display for OcrError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OcrError::ModelNotFound(msg) => write!(f, "OCR model not found: {msg}"),
            OcrError::InferenceFailed(msg) => write!(f, "OCR inference failed: {msg}"),
            OcrError::DownloadFailed(err) => write!(f, "OCR download failed: {err}"),
            OcrError::IoError(msg) => write!(f, "OCR I/O error: {msg}"),
            OcrError::Cancelled => write!(f, "OCR task cancelled"),
        }
    }
}

impl std::error::Error for OcrError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            OcrError::DownloadFailed(err) => Some(err),
            _ => None,
        }
    }
}

impl From<ort::Error> for OcrError {
    fn from(err: ort::Error) -> Self {
        OcrError::InferenceFailed(err.to_string())
    }
}

/// Abstract contract for OCR execution backends.
pub trait OcrEngine: Send + Sync {
    /// Recognizes text in `image` and returns detected text blocks with bounding boxes and confidence scores.
    fn recognize(&self, image: &DynamicImage) -> Result<OcrOutput, OcrError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rect_and_text_block() {
        let rect = Rect::new(10.0, 20.0, 100.0, 50.0);
        let block = OcrTextBlock::new("Hello World", rect.clone(), 0.95);
        assert_eq!(block.text, "Hello World");
        assert_eq!(block.bbox, rect);
        assert_eq!(block.confidence, 0.95);
    }

    #[test]
    fn test_ocr_output_avg_confidence() {
        let b1 = OcrTextBlock::new("Text 1", Rect::new(0.0, 0.0, 10.0, 10.0), 0.8);
        let b2 = OcrTextBlock::new("Text 2", Rect::new(0.0, 10.0, 10.0, 10.0), 0.9);
        let output = OcrOutput::new(vec![b1, b2]);
        assert_eq!(output.blocks.len(), 2);
        assert!((output.avg_confidence - 0.85).abs() < 1e-5);
    }

    #[test]
    fn test_ocr_output_empty_blocks() {
        let output = OcrOutput::new(vec![]);
        assert_eq!(output.blocks.len(), 0);
        assert_eq!(output.avg_confidence, 1.0);
    }

    #[test]
    fn test_ocr_error_display() {
        let err = OcrError::ModelNotFound("det.onnx missing".to_string());
        assert_eq!(err.to_string(), "OCR model not found: det.onnx missing");
    }
}
