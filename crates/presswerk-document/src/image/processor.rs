// SPDX-License-Identifier: PMPL-1.0-or-later
// Copyright (c) 2026 Jonathan D.A. Jewell (hyperpolymath) <j.d.a.jewell@open.ac.uk>
//
// Image processor — resize, rotate, crop, grayscale, brightness/contrast
// adjustment. Operates on in-memory images using the `image` and `imageproc`
// crates.

use image::{DynamicImage, ImageFormat, RgbaImage};
use imageproc::geometric_transformations::{self, Interpolation};
use presswerk_core::error::PresswerkError;
use tracing::{debug, info, instrument};

/// Image processing pipeline operating on a single in-memory image.
///
/// All operations are non-destructive: each method consumes `self` and returns a
/// new `ImageProcessor` wrapping the transformed image, enabling method chaining.
///
/// ```ignore
/// let result = ImageProcessor::open("photo.jpg")?
///     .resize(800, 600)
///     .rotate(90.0)
///     .grayscale()
///     .adjust_brightness(10)
///     .to_png_bytes()?;
/// ```
pub struct ImageProcessor {
    /// The current working image.
    image: DynamicImage,
}

impl ImageProcessor {
    // -- Construction ---------------------------------------------------------

    /// Load an image from a file path.
    #[instrument(skip_all, fields(path = %path.as_ref().display()))]
    pub fn open(path: impl AsRef<std::path::Path>) -> Result<Self, PresswerkError> {
        let img = image::open(path.as_ref()).map_err(|err| {
            PresswerkError::ImageError(format!(
                "failed to open {}: {}",
                path.as_ref().display(),
                err
            ))
        })?;
        info!(width = img.width(), height = img.height(), "Image loaded");
        Ok(Self { image: img })
    }

    /// Create a processor from raw encoded bytes (JPEG, PNG, etc.).
    #[instrument(skip(data), fields(data_len = data.len()))]
    pub fn from_bytes(data: &[u8]) -> Result<Self, PresswerkError> {
        let img = image::load_from_memory(data).map_err(|err| {
            PresswerkError::ImageError(format!("failed to decode image: {}", err))
        })?;
        debug!(
            width = img.width(),
            height = img.height(),
            "Image decoded from bytes"
        );
        Ok(Self { image: img })
    }

    /// Wrap an already-decoded `DynamicImage`.
    pub fn from_dynamic(image: DynamicImage) -> Self {
        Self { image }
    }

    // -- Accessors ------------------------------------------------------------

    /// Current image width in pixels.
    pub fn width(&self) -> u32 {
        self.image.width()
    }

    /// Current image height in pixels.
    pub fn height(&self) -> u32 {
        self.image.height()
    }

    /// Borrow the underlying `DynamicImage`.
    pub fn as_dynamic(&self) -> &DynamicImage {
        &self.image
    }

    /// Consume the processor and return the underlying `DynamicImage`.
    pub fn into_dynamic(self) -> DynamicImage {
        self.image
    }

    // -- Transformations (consume self, return new Self) -----------------------

    /// Resize the image to fit within `max_width` x `max_height`, preserving
    /// aspect ratio. Uses Lanczos3 filtering for high-quality downscaling.
    #[instrument(skip(self), fields(max_width, max_height))]
    pub fn resize(self, max_width: u32, max_height: u32) -> Self {
        info!(
            from_w = self.image.width(),
            from_h = self.image.height(),
            max_width,
            max_height,
            "Resizing image"
        );
        let resized =
            self.image
                .resize(max_width, max_height, image::imageops::FilterType::Lanczos3);
        debug!(
            new_w = resized.width(),
            new_h = resized.height(),
            "Resize complete"
        );
        Self { image: resized }
    }

    /// Resize the image to exactly `width` x `height`, ignoring aspect ratio.
    pub fn resize_exact(self, width: u32, height: u32) -> Self {
        let resized = self
            .image
            .resize_exact(width, height, image::imageops::FilterType::Lanczos3);
        Self { image: resized }
    }

    /// Rotate the image by an arbitrary angle in degrees (clockwise).
    ///
    /// For 90/180/270 degree rotations, lossless rotation is used. For other
    /// angles, affine transformation with bilinear interpolation is applied and
    /// the canvas expands to contain the rotated image.
    #[instrument(skip(self), fields(degrees))]
    pub fn rotate(self, degrees: f32) -> Self {
        info!(degrees, "Rotating image");

        // Fast-path for exact multiples of 90.
        let normalised = degrees.rem_euclid(360.0);
        if (normalised - 90.0).abs() < 0.01 {
            return Self {
                image: self.image.rotate90(),
            };
        }
        if (normalised - 180.0).abs() < 0.01 {
            return Self {
                image: self.image.rotate180(),
            };
        }
        if (normalised - 270.0).abs() < 0.01 {
            return Self {
                image: self.image.rotate270(),
            };
        }
        if normalised.abs() < 0.01 || (normalised - 360.0).abs() < 0.01 {
            return self;
        }

        // General rotation via imageproc's rotate_about_center helper.
        let rgba = self.image.to_rgba8();
        let radians = degrees.to_radians();
        let default_pixel = image::Rgba([255u8, 255, 255, 0]);

        let rotated: RgbaImage = geometric_transformations::rotate_about_center(
            &rgba,
            radians,
            Interpolation::Bilinear,
            default_pixel,
        );

        debug!("General rotation applied");
        Self {
            image: DynamicImage::ImageRgba8(rotated),
        }
    }

    /// Crop a rectangular region from the image.
    ///
    /// `x` and `y` are the top-left corner; `width` and `height` define the
    /// size of the crop rectangle. Values are clamped to image bounds.
    #[instrument(skip(self), fields(x, y, width, height))]
    pub fn crop(self, x: u32, y: u32, width: u32, height: u32) -> Self {
        let img_w = self.image.width();
        let img_h = self.image.height();

        let safe_x = x.min(img_w.saturating_sub(1));
        let safe_y = y.min(img_h.saturating_sub(1));
        let safe_w = width.min(img_w - safe_x);
        let safe_h = height.min(img_h - safe_y);

        info!(safe_x, safe_y, safe_w, safe_h, "Cropping image");

        let cropped = self.image.crop_imm(safe_x, safe_y, safe_w, safe_h);
        Self { image: cropped }
    }

    /// Convert the image to grayscale (luma).
    #[instrument(skip(self))]
    pub fn grayscale(self) -> Self {
        info!("Converting to grayscale");
        Self {
            image: self.image.grayscale(),
        }
    }

    /// Adjust brightness by `value` (-255..=255).
    ///
    /// Positive values brighten, negative values darken. The value is clamped to
    /// [-255, 255].
    #[instrument(skip(self), fields(value))]
    pub fn adjust_brightness(self, value: i32) -> Self {
        let clamped = value.clamp(-255, 255);
        info!(clamped, "Adjusting brightness");

        let rgba = self.image.to_rgba8();

        // Manual per-pixel brightness adjustment.
        let brightened = image::ImageBuffer::from_fn(rgba.width(), rgba.height(), |x, y| {
            let pixel = rgba.get_pixel(x, y);
            let image::Rgba([r, g, b, a]) = *pixel;
            let adjust = |channel: u8| -> u8 {
                let val = channel as i32 + clamped;
                val.clamp(0, 255) as u8
            };
            image::Rgba([adjust(r), adjust(g), adjust(b), a])
        });
        Self {
            image: DynamicImage::ImageRgba8(brightened),
        }
    }

    /// Adjust contrast by a factor. Values > 1.0 increase contrast; values
    /// < 1.0 decrease it. A value of 1.0 is a no-op.
    #[instrument(skip(self), fields(factor))]
    pub fn adjust_contrast(self, factor: f32) -> Self {
        info!(factor, "Adjusting contrast");

        let rgba = self.image.to_rgba8();

        let contrasted = image::ImageBuffer::from_fn(rgba.width(), rgba.height(), |x, y| {
            let pixel = rgba.get_pixel(x, y);
            let image::Rgba([r, g, b, a]) = *pixel;
            let adjust = |channel: u8| -> u8 {
                let val = factor * (channel as f32 - 128.0) + 128.0;
                val.clamp(0.0, 255.0) as u8
            };
            image::Rgba([adjust(r), adjust(g), adjust(b), a])
        });

        Self {
            image: DynamicImage::ImageRgba8(contrasted),
        }
    }

    // -- Output ---------------------------------------------------------------

    /// Encode the current image as PNG bytes.
    pub fn to_png_bytes(&self) -> Result<Vec<u8>, PresswerkError> {
        encode_to_format(&self.image, ImageFormat::Png)
    }

    /// Encode the current image as JPEG bytes with the given quality (1-100).
    pub fn to_jpeg_bytes(&self, quality: u8) -> Result<Vec<u8>, PresswerkError> {
        let mut buffer = Vec::new();
        let rgb = self.image.to_rgb8();
        let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buffer, quality);
        rgb.write_with_encoder(encoder)
            .map_err(|err| PresswerkError::ImageError(format!("JPEG encoding failed: {}", err)))?;
        Ok(buffer)
    }

    /// Write the image to a file. The format is inferred from the file extension.
    pub fn save(&self, path: impl AsRef<std::path::Path>) -> Result<(), PresswerkError> {
        self.image.save(path.as_ref()).map_err(|err| {
            PresswerkError::ImageError(format!(
                "failed to save image to {}: {}",
                path.as_ref().display(),
                err
            ))
        })
    }
}

/// Encode a `DynamicImage` into the specified format, returning the raw bytes.
fn encode_to_format(image: &DynamicImage, format: ImageFormat) -> Result<Vec<u8>, PresswerkError> {
    let mut buffer = Vec::new();
    let mut cursor = std::io::Cursor::new(&mut buffer);
    image
        .write_to(&mut cursor, format)
        .map_err(|err| PresswerkError::ImageError(format!("image encoding failed: {}", err)))?;
    Ok(buffer)
}
