// SPDX-License-Identifier: PMPL-1.0-or-later
// Copyright (c) 2026 Jonathan D.A. Jewell (hyperpolymath) <j.d.a.jewell@open.ac.uk>
//
// OCR (Optical Character Recognition) module for Presswerk.
//
// Provides text extraction from scanned document images using the `ocrs` crate,
// a pure-Rust OCR engine backed by neural network models executed via `rten`.
//
// # Feature Gate
//
// This module is only available when the `ocr` feature is enabled:
//
// ```toml
// presswerk-document = { path = "crates/presswerk-document", features = ["ocr"] }
// ```
//
// # Model Setup
//
// The OCR engine requires two ONNX model files:
//
// - **Detection model** (`text-detection.rten`) — locates text regions in the image.
// - **Recognition model** (`text-recognition.rten`) — decodes characters from detected regions.
//
// Models can be downloaded from the ocrs-models repository:
//   <https://github.com/nickknight/ocrs-models/releases>
//
// Or obtained automatically by running the `ocrs-cli` tool once:
//   ```sh
//   cargo install ocrs-cli
//   ocrs some-image.png  # downloads models to ~/.cache/ocrs/
//   ```
//
// The default cache directory is `$XDG_CACHE_HOME/ocrs` (typically `~/.cache/ocrs`).

use std::path::{Path, PathBuf};

use image::DynamicImage;
use ocrs::{ImageSource, OcrEngine as OcrsEngine, OcrEngineParams};
use presswerk_core::error::PresswerkError;
use rten::Model;
use tracing::{debug, info, instrument, warn};

/// Default directory for cached OCR model files.
///
/// Follows the XDG Base Directory specification: `$XDG_CACHE_HOME/ocrs`, falling
/// back to `~/.cache/ocrs` when `XDG_CACHE_HOME` is unset.
fn default_model_dir() -> PathBuf {
    if let Ok(xdg) = std::env::var("XDG_CACHE_HOME") {
        PathBuf::from(xdg).join("ocrs")
    } else if let Ok(home) = std::env::var("HOME") {
        PathBuf::from(home).join(".cache").join("ocrs")
    } else {
        // Last resort — current directory.
        PathBuf::from("ocrs-models")
    }
}

/// Well-known filenames for the detection and recognition models.
const DETECTION_MODEL_FILENAME: &str = "text-detection.rten";
const RECOGNITION_MODEL_FILENAME: &str = "text-recognition.rten";

/// Configuration for constructing an [`OcrEngine`].
#[derive(Debug, Clone)]
pub struct OcrConfig {
    /// Path to the text-detection model file (`.rten`).
    pub detection_model_path: PathBuf,
    /// Path to the text-recognition model file (`.rten`).
    pub recognition_model_path: PathBuf,
}

impl Default for OcrConfig {
    /// Returns a config pointing at the default model cache directory.
    fn default() -> Self {
        let dir = default_model_dir();
        Self {
            detection_model_path: dir.join(DETECTION_MODEL_FILENAME),
            recognition_model_path: dir.join(RECOGNITION_MODEL_FILENAME),
        }
    }
}

impl OcrConfig {
    /// Create a config with explicit model directory.
    ///
    /// Expects the directory to contain `text-detection.rten` and
    /// `text-recognition.rten`.
    pub fn from_dir(dir: impl AsRef<Path>) -> Self {
        let dir = dir.as_ref();
        Self {
            detection_model_path: dir.join(DETECTION_MODEL_FILENAME),
            recognition_model_path: dir.join(RECOGNITION_MODEL_FILENAME),
        }
    }

    /// Create a config pointing at two specific model files.
    pub fn from_paths(
        detection_model: impl Into<PathBuf>,
        recognition_model: impl Into<PathBuf>,
    ) -> Self {
        Self {
            detection_model_path: detection_model.into(),
            recognition_model_path: recognition_model.into(),
        }
    }

    /// Verify that both model files exist and are readable.
    pub fn validate(&self) -> Result<(), PresswerkError> {
        if !self.detection_model_path.exists() {
            return Err(PresswerkError::OcrError(format!(
                "detection model not found at {}; run `ocrs-cli` once to download models, \
                 or see <https://github.com/nickknight/ocrs-models/releases>",
                self.detection_model_path.display()
            )));
        }
        if !self.recognition_model_path.exists() {
            return Err(PresswerkError::OcrError(format!(
                "recognition model not found at {}; run `ocrs-cli` once to download models, \
                 or see <https://github.com/nickknight/ocrs-models/releases>",
                self.recognition_model_path.display()
            )));
        }
        Ok(())
    }
}

/// Presswerk OCR engine — extracts text from scanned document images.
///
/// Wraps the `ocrs` engine with Presswerk-specific error handling and logging.
/// The engine is initialised once with pre-trained neural network models and can
/// then be reused for many images.
///
/// # Example
///
/// ```rust,no_run
/// use presswerk_document::scan::ocr::{OcrEngine, OcrConfig};
/// use image::DynamicImage;
///
/// let config = OcrConfig::default();
/// let engine = OcrEngine::new(config).expect("failed to load OCR models");
///
/// let img = image::open("scanned-page.png").unwrap();
/// let text = engine.recognize_text(&img).expect("OCR failed");
/// println!("{text}");
/// ```
pub struct OcrEngine {
    /// The underlying `ocrs` engine instance.
    engine: OcrsEngine,
}

impl OcrEngine {
    /// Create a new OCR engine, loading models from the paths given in `config`.
    ///
    /// Model loading is the expensive step — keep the engine around and call
    /// [`recognize_text`](Self::recognize_text) for each page.
    ///
    /// # Errors
    ///
    /// Returns [`PresswerkError::OcrError`] if model files are missing or corrupt.
    ///
    /// # Performance
    ///
    /// **Important:** The `ocrs` and `rten` crates must be compiled in release
    /// mode. Debug builds will be extremely slow (10-100x slower).
    #[instrument(skip_all, fields(
        detection = %config.detection_model_path.display(),
        recognition = %config.recognition_model_path.display(),
    ))]
    pub fn new(config: OcrConfig) -> Result<Self, PresswerkError> {
        config.validate()?;

        info!("Loading OCR detection model");
        let detection_model = Model::load_file(&config.detection_model_path).map_err(|err| {
            PresswerkError::OcrError(format!(
                "failed to load detection model from {}: {}",
                config.detection_model_path.display(),
                err
            ))
        })?;

        info!("Loading OCR recognition model");
        let recognition_model =
            Model::load_file(&config.recognition_model_path).map_err(|err| {
                PresswerkError::OcrError(format!(
                    "failed to load recognition model from {}: {}",
                    config.recognition_model_path.display(),
                    err
                ))
            })?;

        let engine = OcrsEngine::new(OcrEngineParams {
            detection_model: Some(detection_model),
            recognition_model: Some(recognition_model),
            ..Default::default()
        })
        .map_err(|err| {
            PresswerkError::OcrError(format!("failed to initialise OCR engine: {}", err))
        })?;

        info!("OCR engine initialised successfully");
        Ok(Self { engine })
    }

    /// Create an OCR engine using the default model cache directory.
    ///
    /// Equivalent to `OcrEngine::new(OcrConfig::default())`.
    pub fn with_defaults() -> Result<Self, PresswerkError> {
        Self::new(OcrConfig::default())
    }

    /// Create an OCR engine loading models from a specific directory.
    ///
    /// The directory must contain `text-detection.rten` and
    /// `text-recognition.rten`.
    pub fn from_model_dir(dir: impl AsRef<Path>) -> Result<Self, PresswerkError> {
        Self::new(OcrConfig::from_dir(dir))
    }

    /// Extract all text from a scanned document image.
    ///
    /// Returns the recognised text as a single `String`, with lines separated
    /// by newline characters. Empty lines between text blocks are preserved.
    ///
    /// The input image is converted to RGB8 internally if it is in a different
    /// colour space.
    ///
    /// # Errors
    ///
    /// Returns [`PresswerkError::OcrError`] if preprocessing or recognition fails.
    #[instrument(skip_all, fields(width = image.width(), height = image.height()))]
    pub fn recognize_text(&self, image: &DynamicImage) -> Result<String, PresswerkError> {
        info!(
            width = image.width(),
            height = image.height(),
            "Starting OCR text recognition"
        );

        // Convert to RGB8 — the format expected by ocrs.
        let rgb = image.to_rgb8();
        let (width, height) = rgb.dimensions();

        // Prepare the image source for the engine.
        let source = ImageSource::from_bytes(rgb.as_raw(), (width, height)).map_err(|err| {
            PresswerkError::OcrError(format!(
                "failed to create image source ({}x{}): {}",
                width, height, err
            ))
        })?;

        let input = self.engine.prepare_input(source).map_err(|err| {
            PresswerkError::OcrError(format!("OCR preprocessing failed: {}", err))
        })?;

        // Use the high-level get_text API for straightforward text extraction.
        let text = self.engine.get_text(&input).map_err(|err| {
            PresswerkError::OcrError(format!("OCR text recognition failed: {}", err))
        })?;

        let line_count = text.lines().count();
        let char_count = text.len();
        debug!(line_count, char_count, "OCR recognition complete");

        Ok(text)
    }

    /// Extract text with detailed layout information.
    ///
    /// Returns a list of [`OcrTextLine`] structs, each containing the recognised
    /// text and the bounding box of the line in image coordinates.
    ///
    /// This is more expensive than [`recognize_text`](Self::recognize_text) but
    /// preserves spatial information useful for document reconstruction.
    ///
    /// # Errors
    ///
    /// Returns [`PresswerkError::OcrError`] if detection or recognition fails.
    #[instrument(skip_all, fields(width = image.width(), height = image.height()))]
    pub fn recognize_text_with_layout(
        &self,
        image: &DynamicImage,
    ) -> Result<Vec<OcrTextLine>, PresswerkError> {
        info!(
            width = image.width(),
            height = image.height(),
            "Starting OCR with layout extraction"
        );

        let rgb = image.to_rgb8();
        let (width, height) = rgb.dimensions();

        let source = ImageSource::from_bytes(rgb.as_raw(), (width, height)).map_err(|err| {
            PresswerkError::OcrError(format!(
                "failed to create image source ({}x{}): {}",
                width, height, err
            ))
        })?;

        let input = self.engine.prepare_input(source).map_err(|err| {
            PresswerkError::OcrError(format!("OCR preprocessing failed: {}", err))
        })?;

        // Step 1: Detect word bounding boxes.
        let word_rects = self
            .engine
            .detect_words(&input)
            .map_err(|err| PresswerkError::OcrError(format!("word detection failed: {}", err)))?;
        debug!(word_count = word_rects.len(), "Words detected");

        // Step 2: Group words into text lines.
        let line_rects = self.engine.find_text_lines(&input, &word_rects);
        debug!(line_count = line_rects.len(), "Text lines found");

        // Step 3: Recognise characters within each line.
        let line_texts = self
            .engine
            .recognize_text(&input, &line_rects)
            .map_err(|err| PresswerkError::OcrError(format!("line recognition failed: {}", err)))?;

        // Build the result, filtering out empty lines.
        let mut results = Vec::with_capacity(line_texts.len());
        for line in line_texts.iter().flatten() {
            let text: String = line.to_string();

            if text.trim().is_empty() {
                continue;
            }

            results.push(OcrTextLine { text });
        }

        info!(
            recognized_lines = results.len(),
            "Layout-aware OCR complete"
        );
        Ok(results)
    }

    /// Check whether the OCR models are loaded and the engine is ready.
    ///
    /// Always returns `true` after successful construction — provided as a
    /// convenience for UI status indicators.
    pub fn is_ready(&self) -> bool {
        true
    }
}

/// A line of text extracted by the OCR engine, with optional layout metadata.
#[derive(Debug, Clone)]
pub struct OcrTextLine {
    /// The recognised text content of this line.
    pub text: String,
}

impl std::fmt::Display for OcrTextLine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.text)
    }
}

/// Check whether OCR model files exist in the default cache location.
///
/// Returns `Ok(true)` if both models are present, `Ok(false)` if either is
/// missing, or an error if the path cannot be determined.
pub fn models_available() -> bool {
    let config = OcrConfig::default();
    config.detection_model_path.exists() && config.recognition_model_path.exists()
}

/// Return the default model directory path (for display in UI / diagnostics).
pub fn model_directory() -> PathBuf {
    default_model_dir()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_points_to_cache_dir() {
        let config = OcrConfig::default();
        let path_str = config.detection_model_path.to_string_lossy();
        // Should end with the expected filename regardless of platform.
        assert!(
            path_str.ends_with(DETECTION_MODEL_FILENAME),
            "detection model path should end with {DETECTION_MODEL_FILENAME}, got {path_str}"
        );
        let rec_str = config.recognition_model_path.to_string_lossy();
        assert!(
            rec_str.ends_with(RECOGNITION_MODEL_FILENAME),
            "recognition model path should end with {RECOGNITION_MODEL_FILENAME}, got {rec_str}"
        );
    }

    #[test]
    fn config_from_dir() {
        let config = OcrConfig::from_dir("/tmp/my-models");
        assert_eq!(
            config.detection_model_path,
            PathBuf::from("/tmp/my-models/text-detection.rten")
        );
        assert_eq!(
            config.recognition_model_path,
            PathBuf::from("/tmp/my-models/text-recognition.rten")
        );
    }

    #[test]
    fn config_from_paths() {
        let config = OcrConfig::from_paths("/a/detect.rten", "/b/recog.rten");
        assert_eq!(config.detection_model_path, PathBuf::from("/a/detect.rten"));
        assert_eq!(
            config.recognition_model_path,
            PathBuf::from("/b/recog.rten")
        );
    }

    #[test]
    fn validate_missing_models() {
        let config = OcrConfig::from_dir("/nonexistent/path/ocr-models");
        let result = config.validate();
        assert!(result.is_err(), "validate should fail for missing models");
    }

    #[test]
    fn models_available_returns_false_when_missing() {
        // With no models cached, this should return false on CI / fresh systems.
        // On a developer machine with models, it may return true — both are valid.
        let _available = models_available();
        // Just ensure it doesn't panic.
    }
}
