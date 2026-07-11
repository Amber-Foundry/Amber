pub mod bundled;
pub mod engine;
pub mod pdf;

use crate::models::download::download_and_verify;
use crate::models::registry::parse_registry_json;
pub use bundled::BundledOcrEngine;
pub use engine::{OcrEngine, OcrError, OcrOutput, OcrTextBlock, Rect};
pub use pdf::{PdfPageInfo, PdfPageType, PdfRasterizer, PdfRasterizerConfig};
use serde::Deserialize;
use std::env;
use std::path::PathBuf;

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct ModelAssetEntry {
    pub url: String,
    pub filename: String,
    pub size_mb: u32,
    pub sha256: String,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct OcrRegistry {
    pub det: ModelAssetEntry,
    pub rec: ModelAssetEntry,
    pub dict: ModelAssetEntry,
}

/// Loads the static `ocr_registry.json` embedded into the binary.
pub fn load_ocr_registry() -> Result<OcrRegistry, OcrError> {
    let json_str = include_str!("../../../ocr_registry.json");
    parse_registry_json(json_str).map_err(OcrError::IoError)
}

/// Resolves the local models directory for OCR models (`~/.amber/models/ocr/`).
pub fn ocr_models_dir() -> Result<PathBuf, OcrError> {
    let home = env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(PathBuf::from))
        .ok_or_else(|| {
            OcrError::IoError(
                "could not resolve user home directory for ~/.amber/models/ocr".to_string(),
            )
        })?;
    Ok(home.join(".amber").join("models").join("ocr"))
}

/// Checks if all OCR model files (`det.onnx`, `rec.onnx`, `ppocrv6_dict.txt`) exist in `ocr_models_dir()`.
pub fn ocr_models_exist() -> bool {
    let Ok(dir) = ocr_models_dir() else {
        return false;
    };
    let Ok(registry) = load_ocr_registry() else {
        return false;
    };

    dir.join(&registry.det.filename).is_file()
        && dir.join(&registry.rec.filename).is_file()
        && dir.join(&registry.dict.filename).is_file()
}

/// Self-contained OCR download bootstrap: ensures `det.onnx`, `rec.onnx`, and `ppocrv6_dict.txt`
/// exist in `~/.amber/models/ocr/` with verified SHA-256 hashes.
pub fn download_ocr_models_if_needed() -> Result<(), OcrError> {
    let dir = ocr_models_dir()?;
    let registry = load_ocr_registry()?;

    let assets = [&registry.det, &registry.rec, &registry.dict];

    for asset in assets {
        let dest = dir.join(&asset.filename);
        download_and_verify(&asset.url, &asset.sha256, &dest).map_err(OcrError::DownloadFailed)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_load_ocr_registry() -> Result<(), Box<dyn std::error::Error>> {
        let registry = load_ocr_registry().map_err(Box::<dyn std::error::Error>::from)?;
        assert_eq!(registry.det.filename, "det.onnx");
        assert_eq!(registry.rec.filename, "rec.onnx");
        assert_eq!(registry.dict.filename, "ppocrv6_dict.txt");
        assert!(registry.det.size_mb > 0);
        assert!(!registry.det.sha256.is_empty());
        Ok(())
    }

    #[test]
    fn test_ocr_models_dir() -> Result<(), Box<dyn std::error::Error>> {
        let dir = ocr_models_dir().map_err(Box::<dyn std::error::Error>::from)?;
        assert!(dir.to_string_lossy().contains("ocr"));
        Ok(())
    }

    #[test]
    #[ignore]
    fn test_manual_ocr_download_bootstrap() -> Result<(), Box<dyn std::error::Error>> {
        println!("Starting OCR model download bootstrap...");
        download_ocr_models_if_needed().map_err(Box::<dyn std::error::Error>::from)?;
        let dir = ocr_models_dir().map_err(Box::<dyn std::error::Error>::from)?;
        println!("OCR models directory: {}", dir.display());
        assert!(ocr_models_exist(), "OCR models should exist on disk");
        println!("Successfully verified OCR models exist on disk!");
        Ok(())
    }
}
