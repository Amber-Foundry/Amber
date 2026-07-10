use sha2::{Digest, Sha256};
use std::fmt;
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::Path;
use std::time::Duration;

#[derive(Debug)]
pub enum DownloadError {
    Network(String),
    HashMismatch { expected: String, actual: String },
    Io(String),
    InvalidUrl(String),
}

impl fmt::Display for DownloadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DownloadError::Network(msg) => write!(f, "Network error: {msg}"),
            DownloadError::HashMismatch { expected, actual } => {
                write!(f, "Hash mismatch: expected {expected}, got {actual}")
            }
            DownloadError::Io(msg) => write!(f, "I/O error: {msg}"),
            DownloadError::InvalidUrl(msg) => write!(f, "Invalid URL: {msg}"),
        }
    }
}

impl std::error::Error for DownloadError {}

pub fn compute_sha256(path: &Path) -> Result<String, io::Error> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 8192];
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

/// Downloads a model asset from `url`, verifies its SHA-256 hash against `expected_sha256`,
/// and saves it atomically to `dest_path`.
///
/// If `dest_path` already exists and its SHA-256 matches `expected_sha256`, the download is skipped.
pub fn download_and_verify(
    url: &str,
    expected_sha256: &str,
    dest_path: &Path,
) -> Result<(), DownloadError> {
    if dest_path.is_file() {
        if let Ok(actual) = compute_sha256(dest_path) {
            if actual.eq_ignore_ascii_case(expected_sha256) {
                return Ok(());
            }
        }
    }

    let parent = dest_path.parent().ok_or_else(|| {
        DownloadError::Io(format!("Invalid destination path: {}", dest_path.display()))
    })?;
    fs::create_dir_all(parent).map_err(|err| DownloadError::Io(err.to_string()))?;

    let tmp_path = dest_path.with_extension("tmp_download");
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(120))
        .build()
        .map_err(|err| DownloadError::Network(err.to_string()))?;
    let mut response = client
        .get(url)
        .send()
        .map_err(|err| DownloadError::Network(err.to_string()))?;
    if !response.status().is_success() {
        return Err(DownloadError::Network(format!(
            "HTTP {} when fetching {}",
            response.status(),
            url
        )));
    }

    let mut tmp_file = File::create(&tmp_path).map_err(|err| DownloadError::Io(err.to_string()))?;
    if let Err(err) = io::copy(&mut response, &mut tmp_file) {
        let _ = fs::remove_file(&tmp_path);
        return Err(DownloadError::Network(err.to_string()));
    }
    drop(tmp_file);

    let actual_sha256 =
        compute_sha256(&tmp_path).map_err(|err| DownloadError::Io(err.to_string()))?;

    if !actual_sha256.eq_ignore_ascii_case(expected_sha256) {
        let _ = fs::remove_file(&tmp_path);
        return Err(DownloadError::HashMismatch {
            expected: expected_sha256.to_string(),
            actual: actual_sha256,
        });
    }

    fs::rename(&tmp_path, dest_path).map_err(|err| DownloadError::Io(err.to_string()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_sha256() -> Result<(), Box<dyn std::error::Error>> {
        let temp_dir = std::env::temp_dir();
        let test_file = temp_dir.join("amber_sha256_test.txt");
        fs::write(&test_file, b"hello world")?;

        let hash = compute_sha256(&test_file)?;
        // SHA-256 of "hello world"
        assert_eq!(
            hash,
            "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9"
        );

        let _ = fs::remove_file(test_file);
        Ok(())
    }

    #[test]
    fn test_download_error_display() {
        let err = DownloadError::HashMismatch {
            expected: "abc".to_string(),
            actual: "def".to_string(),
        };
        assert_eq!(err.to_string(), "Hash mismatch: expected abc, got def");
    }
}
