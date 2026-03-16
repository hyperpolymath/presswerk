// SPDX-License-Identifier: PMPL-1.0-or-later
// Copyright (c) 2026 Jonathan D.A. Jewell (hyperpolymath) <j.d.a.jewell@open.ac.uk>
//
// Scan enhancement pipeline — binarization, contrast boosting, edge-aware
// cleanup, and scan-to-PDF conversion for scanned document images.

use image::{DynamicImage, GrayImage, Luma, Rgba, RgbaImage};
use imageproc::edges::canny;
use imageproc::filter::gaussian_blur_f32;
use imageproc::geometric_transformations::{Interpolation, Projection, warp_into};
use imageproc::hough::{LineDetectionOptions, PolarLine, detect_lines};
use presswerk_core::PaperSize;
use presswerk_core::error::PresswerkError;
use tracing::{debug, info, instrument, warn};

use crate::image::processor::ImageProcessor;
use crate::pdf::writer::PdfWriter;

/// Enhances scanned document images for print-quality output.
///
/// Provides a pipeline of operations commonly needed when scanning physical
/// documents: grayscale conversion, contrast enhancement, adaptive binarization,
/// and perspective correction. The final output can be exported directly to PDF.
pub struct ScanEnhancer {
    /// The working image (kept as `DynamicImage` for flexibility).
    image: DynamicImage,
    /// Target paper size for PDF output.
    paper_size: PaperSize,
}

impl ScanEnhancer {
    // -- Construction ---------------------------------------------------------

    /// Create an enhancer from raw image bytes (JPEG, PNG, TIFF, etc.).
    #[instrument(skip(data), fields(data_len = data.len()))]
    pub fn from_bytes(data: &[u8], paper_size: PaperSize) -> Result<Self, PresswerkError> {
        let image = image::load_from_memory(data).map_err(|err| {
            PresswerkError::ImageError(format!("failed to decode scan image: {}", err))
        })?;
        info!(
            width = image.width(),
            height = image.height(),
            "Scan image loaded"
        );
        Ok(Self { image, paper_size })
    }

    /// Create an enhancer from a file path.
    #[instrument(skip_all, fields(path = %path.as_ref().display()))]
    pub fn open(
        path: impl AsRef<std::path::Path>,
        paper_size: PaperSize,
    ) -> Result<Self, PresswerkError> {
        let image = image::open(path.as_ref()).map_err(|err| {
            PresswerkError::ImageError(format!(
                "failed to open scan image {}: {}",
                path.as_ref().display(),
                err
            ))
        })?;
        Ok(Self { image, paper_size })
    }

    /// Create an enhancer wrapping an existing `DynamicImage`.
    pub fn from_dynamic(image: DynamicImage, paper_size: PaperSize) -> Self {
        Self { image, paper_size }
    }

    // -- Accessors ------------------------------------------------------------

    /// Borrow the current working image.
    pub fn as_dynamic(&self) -> &DynamicImage {
        &self.image
    }

    /// Consume the enhancer and return the underlying image.
    pub fn into_dynamic(self) -> DynamicImage {
        self.image
    }

    // -- Binarization ---------------------------------------------------------

    /// Apply adaptive thresholding to produce a black-and-white image.
    ///
    /// Uses a local mean approach: for each pixel, the threshold is the mean
    /// intensity within a `block_radius` neighbourhood, minus a constant `c`.
    /// Pixels darker than the local threshold become black; others become white.
    ///
    /// A typical `block_radius` is 15 and `c` is 10.
    #[instrument(skip(self), fields(block_radius, c))]
    pub fn binarize(self, block_radius: u32, c: i32) -> Self {
        info!(block_radius, c, "Applying adaptive binarization");

        let gray = self.image.to_luma8();
        let (width, height) = gray.dimensions();

        // Compute the integral image for fast local mean calculation.
        let integral = compute_integral_image(&gray);

        let mut output = GrayImage::new(width, height);

        for y in 0..height {
            for x in 0..width {
                let local_mean = region_mean(&integral, width, height, x, y, block_radius);
                let threshold = (local_mean as i32 - c).clamp(0, 255) as u8;
                let pixel_val = gray.get_pixel(x, y).0[0];
                let binary = if pixel_val < threshold { 0u8 } else { 255u8 };
                output.put_pixel(x, y, Luma([binary]));
            }
        }

        debug!("Binarization complete");
        Self {
            image: DynamicImage::ImageLuma8(output),
            paper_size: self.paper_size,
        }
    }

    /// Simple global (Otsu-style) binarization using a fixed threshold.
    ///
    /// Computes the threshold automatically from the image histogram via Otsu's
    /// method.
    #[instrument(skip(self))]
    pub fn binarize_otsu(self) -> Self {
        info!("Applying Otsu binarization");

        let gray = self.image.to_luma8();
        let threshold = otsu_threshold(&gray);
        debug!(threshold, "Otsu threshold computed");

        let (width, height) = gray.dimensions();
        let mut output = GrayImage::new(width, height);

        for y in 0..height {
            for x in 0..width {
                let val = gray.get_pixel(x, y).0[0];
                let binary = if val < threshold { 0u8 } else { 255u8 };
                output.put_pixel(x, y, Luma([binary]));
            }
        }

        Self {
            image: DynamicImage::ImageLuma8(output),
            paper_size: self.paper_size,
        }
    }

    // -- Enhancement pipeline -------------------------------------------------

    /// Run the full scan enhancement pipeline:
    ///
    /// 1. Convert to grayscale
    /// 2. Boost contrast (factor 1.4)
    /// 3. Adaptive binarization (block_radius=15, c=10)
    ///
    /// This is the recommended single-call method for typical scanned documents.
    #[instrument(skip(self))]
    pub fn enhance_scan(self) -> Self {
        info!("Running full scan enhancement pipeline");

        let paper_size = self.paper_size;

        // Step 1: Grayscale conversion.
        let processor = ImageProcessor::from_dynamic(self.image)
            .grayscale()
            .adjust_contrast(1.4);

        // Step 2+3: Re-wrap and binarize.
        let enhanced = Self {
            image: processor.into_dynamic(),
            paper_size,
        };

        enhanced.binarize(15, 10)
    }

    // -- Perspective correction -----------------------------------------------

    /// Attempt perspective correction on a scanned document.
    ///
    /// Detects the document quadrilateral using edge detection and the Hough
    /// line transform, then warps it to a rectangle matching the configured
    /// paper size.
    ///
    /// ## Pipeline
    ///
    /// 1. Convert to grayscale
    /// 2. Gaussian blur (sigma 2.0) for noise reduction
    /// 3. Canny edge detection
    /// 4. Hough line detection to find dominant straight edges
    /// 5. Classify lines as roughly horizontal or roughly vertical
    /// 6. Select the four dominant edges (top/bottom horizontal, left/right vertical)
    /// 7. Compute corner points from pairwise line intersections
    /// 8. Compute a projective transformation mapping the quadrilateral to a
    ///    rectangle sized for the target paper dimensions
    /// 9. Apply the warp via `imageproc::geometric_transformations::warp_into`
    ///
    /// If any step fails to produce a clean quadrilateral (e.g. no clear
    /// document borders), the original image is returned unchanged.
    #[instrument(skip(self))]
    pub fn correct_perspective(self) -> Self {
        info!("Starting perspective correction pipeline");

        let (orig_w, orig_h) = (self.image.width(), self.image.height());

        // Step 1: Convert to grayscale.
        let gray = self.image.to_luma8();
        debug!(width = orig_w, height = orig_h, "Converted to grayscale");

        // Step 2: Gaussian blur for noise reduction.
        let blurred = gaussian_blur_f32(&gray, 2.0);
        debug!("Applied Gaussian blur (sigma=2.0)");

        // Step 3: Canny edge detection.
        let edges = canny(&blurred, 50.0, 150.0);
        debug!("Canny edge detection complete");

        // Step 4: Hough line detection.
        // Use a vote threshold proportional to the image diagonal so that
        // detection scales with image resolution. The suppression radius
        // prevents near-duplicate lines.
        let diagonal = ((orig_w as f64).powi(2) + (orig_h as f64).powi(2)).sqrt();
        let vote_threshold = (diagonal * 0.25).max(80.0) as u32;
        let options = LineDetectionOptions {
            vote_threshold,
            suppression_radius: 8,
        };
        let lines = detect_lines(&edges, options);
        debug!(
            line_count = lines.len(),
            vote_threshold, "Hough lines detected"
        );

        if lines.len() < 4 {
            warn!(
                line_count = lines.len(),
                "Too few lines detected for perspective correction; returning unchanged"
            );
            return self;
        }

        // Step 5: Classify lines as horizontal or vertical.
        // angle_in_degrees is 0..180: ~0 or ~180 → horizontal, ~90 → vertical.
        let (horizontal, vertical) = classify_lines(&lines);
        debug!(
            horizontal = horizontal.len(),
            vertical = vertical.len(),
            "Lines classified"
        );

        if horizontal.len() < 2 || vertical.len() < 2 {
            warn!(
                horizontal = horizontal.len(),
                vertical = vertical.len(),
                "Insufficient horizontal/vertical lines; returning unchanged"
            );
            return self;
        }

        // Step 6: Find the four dominant edges.
        // For horizontals, pick the topmost (smallest y-intercept) and
        // bottommost (largest y-intercept). For verticals, pick leftmost
        // and rightmost.
        let top_line = find_extreme_line(&horizontal, orig_w, orig_h, EdgeKind::Top);
        let bottom_line = find_extreme_line(&horizontal, orig_w, orig_h, EdgeKind::Bottom);
        let left_line = find_extreme_line(&vertical, orig_w, orig_h, EdgeKind::Left);
        let right_line = find_extreme_line(&vertical, orig_w, orig_h, EdgeKind::Right);

        // Step 7: Compute the four corner points from line intersections.
        let corners = match compute_quad_corners(&top_line, &bottom_line, &left_line, &right_line) {
            Some(c) => c,
            None => {
                warn!("Could not compute all four corner intersections; returning unchanged");
                return self;
            }
        };

        debug!(
            top_left = ?corners[0],
            top_right = ?corners[1],
            bottom_right = ?corners[2],
            bottom_left = ?corners[3],
            "Quadrilateral corners computed"
        );

        // Sanity check: the detected quad should be at least 10% of the image
        // area to avoid spurious micro-rectangles.
        let quad_area = shoelace_area(&corners);
        let img_area = orig_w as f32 * orig_h as f32;
        if quad_area < img_area * 0.10 {
            warn!(
                quad_area,
                min_area = img_area * 0.10,
                "Detected quadrilateral too small; returning unchanged"
            );
            return self;
        }

        // Step 8: Compute the target rectangle dimensions.
        // Use the paper size at 300 DPI to determine the output rectangle,
        // but keep the original image dimensions if they are smaller (avoid
        // upscaling). Fall back to the original image size if paper-based
        // pixels would be larger.
        let (paper_w_mm, paper_h_mm) = self.paper_size.dimensions_mm();
        let paper_w_px = (paper_w_mm as f32 * 300.0 / 25.4).round() as u32;
        let paper_h_px = (paper_h_mm as f32 * 300.0 / 25.4).round() as u32;
        let out_w = paper_w_px.min(orig_w);
        let out_h = paper_h_px.min(orig_h);

        let dest: [(f32, f32); 4] = [
            (0.0, 0.0),                   // top-left
            (out_w as f32, 0.0),          // top-right
            (out_w as f32, out_h as f32), // bottom-right
            (0.0, out_h as f32),          // bottom-left
        ];

        let src: [(f32, f32); 4] = [corners[0], corners[1], corners[2], corners[3]];

        // Step 9: Build the projective transform and warp.
        // from_control_points computes the mapping from `src` to `dest`.
        let projection = match Projection::from_control_points(src, dest) {
            Some(p) => p,
            None => {
                warn!("Failed to compute projective transform; returning unchanged");
                return self;
            }
        };

        let rgba_input = self.image.to_rgba8();
        let default_pixel = Rgba([255u8, 255, 255, 255]);
        let mut output = RgbaImage::new(out_w, out_h);

        warp_into(
            &rgba_input,
            &projection,
            Interpolation::Bilinear,
            default_pixel,
            &mut output,
        );

        info!(out_w, out_h, "Perspective correction applied");

        Self {
            image: DynamicImage::ImageRgba8(output),
            paper_size: self.paper_size,
        }
    }

    // -- Scan to PDF ----------------------------------------------------------

    /// Convert the (possibly enhanced) scan image to a print-ready PDF.
    ///
    /// The image is encoded as PNG, then embedded in a single-page PDF sized to
    /// the configured paper size.
    #[instrument(skip(self))]
    pub fn scan_to_pdf(&self) -> Result<Vec<u8>, PresswerkError> {
        info!(paper = ?self.paper_size, "Converting scan to PDF");

        let png_bytes = ImageProcessor::from_dynamic(self.image.clone()).to_png_bytes()?;

        let mut writer = PdfWriter::new(self.paper_size);
        writer.set_title("Presswerk Scan");
        let pdf_bytes = writer.create_from_image(&png_bytes)?;

        debug!(pdf_bytes = pdf_bytes.len(), "Scan-to-PDF complete");
        Ok(pdf_bytes)
    }

    /// Run the full enhancement pipeline and then convert to PDF in one call.
    #[instrument(skip(self))]
    pub fn enhance_and_convert(self) -> Result<Vec<u8>, PresswerkError> {
        info!("Running enhance + scan-to-PDF");
        let enhanced = self.enhance_scan();
        enhanced.scan_to_pdf()
    }
}

// -- Integral image helpers ---------------------------------------------------

/// Compute the integral (summed-area table) of a grayscale image.
///
/// `integral[y * (width+1) + x]` contains the sum of all pixel values in the
/// rectangle [0, 0) to (x, y) (exclusive on both axes). The table has
/// dimensions `(width+1) x (height+1)` with a zero-padded border.
fn compute_integral_image(gray: &GrayImage) -> Vec<u64> {
    let (w, h) = gray.dimensions();
    let stride = (w + 1) as usize;
    let mut table = vec![0u64; stride * (h + 1) as usize];

    for y in 0..h {
        let mut row_sum: u64 = 0;
        for x in 0..w {
            row_sum += gray.get_pixel(x, y).0[0] as u64;
            let idx = (y + 1) as usize * stride + (x + 1) as usize;
            let above = y as usize * stride + (x + 1) as usize;
            table[idx] = row_sum + table[above];
        }
    }

    table
}

/// Compute the mean pixel value within a square region centred on (cx, cy)
/// with the given radius, using the precomputed integral image.
fn region_mean(
    integral: &[u64],
    img_width: u32,
    img_height: u32,
    cx: u32,
    cy: u32,
    radius: u32,
) -> f64 {
    let stride = (img_width + 1) as usize;

    // Clamp the region to image bounds.
    let x1 = cx.saturating_sub(radius) as usize;
    let y1 = cy.saturating_sub(radius) as usize;
    let x2 = ((cx + radius + 1) as usize).min(img_width as usize);
    let y2 = ((cy + radius + 1) as usize).min(img_height as usize);

    let area = ((x2 - x1) * (y2 - y1)) as f64;
    if area == 0.0 {
        return 128.0;
    }

    // Summed-area table lookup: S = I[y2][x2] - I[y1][x2] - I[y2][x1] + I[y1][x1]
    let sum = integral[y2 * stride + x2] as f64
        - integral[y1 * stride + x2] as f64
        - integral[y2 * stride + x1] as f64
        + integral[y1 * stride + x1] as f64;

    sum / area
}

/// Compute the Otsu threshold for a grayscale image.
///
/// Finds the threshold value that minimises the intra-class variance of the
/// black and white pixel groups.
fn otsu_threshold(gray: &GrayImage) -> u8 {
    // Build histogram.
    let mut histogram = [0u64; 256];
    for pixel in gray.pixels() {
        histogram[pixel.0[0] as usize] += 1;
    }

    let total_pixels = gray.width() as u64 * gray.height() as u64;
    if total_pixels == 0 {
        return 128;
    }

    let mut sum_total: f64 = 0.0;
    for (i, &count) in histogram.iter().enumerate() {
        sum_total += i as f64 * count as f64;
    }

    let mut sum_background: f64 = 0.0;
    let mut weight_background: u64 = 0;
    let mut max_variance: f64 = 0.0;
    let mut best_threshold: u8 = 0;

    for (t, &count) in histogram.iter().enumerate() {
        weight_background += count;
        if weight_background == 0 {
            continue;
        }
        let weight_foreground = total_pixels - weight_background;
        if weight_foreground == 0 {
            break;
        }

        sum_background += t as f64 * count as f64;
        let mean_background = sum_background / weight_background as f64;
        let mean_foreground = (sum_total - sum_background) / weight_foreground as f64;

        let between_variance = weight_background as f64
            * weight_foreground as f64
            * (mean_background - mean_foreground).powi(2);

        if between_variance > max_variance {
            max_variance = between_variance;
            best_threshold = t as u8;
        }
    }

    best_threshold
}

// -- Perspective correction helpers -------------------------------------------

/// Which document edge a line corresponds to.
#[derive(Debug, Clone, Copy)]
enum EdgeKind {
    Top,
    Bottom,
    Left,
    Right,
}

/// Classify Hough lines as roughly horizontal or roughly vertical.
///
/// A line with `angle_in_degrees` in [0, 30] or [150, 180] is treated as
/// horizontal. A line in [60, 120] is treated as vertical. Lines in the
/// intermediate zones are discarded.
fn classify_lines(lines: &[PolarLine]) -> (Vec<PolarLine>, Vec<PolarLine>) {
    let mut horizontal = Vec::new();
    let mut vertical = Vec::new();

    for line in lines {
        let angle = line.angle_in_degrees;
        if angle <= 30 || angle >= 150 {
            horizontal.push(*line);
        } else if (60..=120).contains(&angle) {
            vertical.push(*line);
        }
        // Lines in (30, 60) or (120, 150) are ambiguous — skip them.
    }

    (horizontal, vertical)
}

/// Select the extreme line in a set, according to the requested edge.
///
/// For horizontal lines, "top" means the one closest to y=0 and "bottom"
/// means the one closest to the image bottom. For vertical lines, "left"
/// means closest to x=0, "right" means closest to the image right edge.
///
/// The metric used is the signed distance `r` of the `PolarLine`.
fn find_extreme_line(
    lines: &[PolarLine],
    _img_width: u32,
    _img_height: u32,
    kind: EdgeKind,
) -> PolarLine {
    match kind {
        EdgeKind::Top | EdgeKind::Left => {
            // Smallest `r` (closest to origin).
            *lines
                .iter()
                .min_by(|a, b| a.r.partial_cmp(&b.r).unwrap_or(std::cmp::Ordering::Equal))
                .expect("lines slice must be non-empty")
        }
        EdgeKind::Bottom | EdgeKind::Right => {
            // Largest `r` (farthest from origin).
            *lines
                .iter()
                .max_by(|a, b| a.r.partial_cmp(&b.r).unwrap_or(std::cmp::Ordering::Equal))
                .expect("lines slice must be non-empty")
        }
    }
}

/// Compute the four corners of the document quadrilateral by intersecting
/// the top/bottom horizontal lines with the left/right vertical lines.
///
/// Returns `[top_left, top_right, bottom_right, bottom_left]`, or `None` if
/// any intersection is degenerate (parallel lines).
fn compute_quad_corners(
    top: &PolarLine,
    bottom: &PolarLine,
    left: &PolarLine,
    right: &PolarLine,
) -> Option<[(f32, f32); 4]> {
    let top_left = intersect_polar_lines(top, left)?;
    let top_right = intersect_polar_lines(top, right)?;
    let bottom_right = intersect_polar_lines(bottom, right)?;
    let bottom_left = intersect_polar_lines(bottom, left)?;
    Some([top_left, top_right, bottom_right, bottom_left])
}

/// Compute the intersection of two lines given in polar (Hough) form.
///
/// A `PolarLine` with parameters `(r, theta)` represents the line
///   `x * cos(theta) + y * sin(theta) = r`
///
/// Returns `None` if the lines are (nearly) parallel.
fn intersect_polar_lines(a: &PolarLine, b: &PolarLine) -> Option<(f32, f32)> {
    let theta_a = (a.angle_in_degrees as f64).to_radians();
    let theta_b = (b.angle_in_degrees as f64).to_radians();

    let cos_a = theta_a.cos();
    let sin_a = theta_a.sin();
    let cos_b = theta_b.cos();
    let sin_b = theta_b.sin();

    let denom = cos_a * sin_b - sin_a * cos_b;
    if denom.abs() < 1e-6 {
        return None; // Lines are nearly parallel.
    }

    let r_a = a.r as f64;
    let r_b = b.r as f64;

    let x = (r_a * sin_b - r_b * sin_a) / denom;
    let y = (r_b * cos_a - r_a * cos_b) / denom;

    Some((x as f32, y as f32))
}

/// Compute the area of a quadrilateral given by four vertices using the
/// shoelace formula. The vertices should be in order (CW or CCW).
fn shoelace_area(corners: &[(f32, f32); 4]) -> f32 {
    let n = corners.len();
    let mut area = 0.0f32;
    for i in 0..n {
        let j = (i + 1) % n;
        area += corners[i].0 * corners[j].1;
        area -= corners[j].0 * corners[i].1;
    }
    area.abs() / 2.0
}

// -- Tests --------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use image::{GrayImage, Luma, RgbaImage};

    /// Verify that `correct_perspective` on a blank image does not panic and
    /// returns an image of the same dimensions (fallback path, since a uniform
    /// image has no detectable edges).
    #[test]
    fn correct_perspective_blank_image_returns_unchanged() {
        let img = DynamicImage::ImageLuma8(GrayImage::from_pixel(200, 300, Luma([200u8])));
        let enhancer = ScanEnhancer::from_dynamic(img, PaperSize::A4);

        let result = enhancer.correct_perspective();
        let out = result.as_dynamic();

        // A blank image has no edges, so the fallback should return it as-is.
        assert_eq!(out.width(), 200);
        assert_eq!(out.height(), 300);
    }

    /// Verify that `correct_perspective` on a small RGBA image does not panic.
    #[test]
    fn correct_perspective_small_rgba_no_panic() {
        let img =
            DynamicImage::ImageRgba8(RgbaImage::from_pixel(50, 50, Rgba([128, 128, 128, 255])));
        let enhancer = ScanEnhancer::from_dynamic(img, PaperSize::Letter);
        // Should not panic — just fall back gracefully.
        let _result = enhancer.correct_perspective();
    }

    /// Verify the shoelace area computation for a known rectangle.
    #[test]
    fn shoelace_area_rectangle() {
        let corners = [(0.0, 0.0), (10.0, 0.0), (10.0, 5.0), (0.0, 5.0)];
        let area = shoelace_area(&corners);
        assert!((area - 50.0).abs() < 1e-3, "Expected 50.0, got {}", area);
    }

    /// Verify that two perpendicular polar lines intersect correctly.
    #[test]
    fn intersect_polar_lines_perpendicular() {
        // Horizontal line at y=100: angle=90, r=100.
        let h = PolarLine {
            r: 100.0,
            angle_in_degrees: 90,
        };
        // Vertical line at x=50: angle=0, r=50.
        let v = PolarLine {
            r: 50.0,
            angle_in_degrees: 0,
        };

        let pt = intersect_polar_lines(&h, &v).expect("should intersect");
        assert!(
            (pt.0 - 50.0).abs() < 0.5 && (pt.1 - 100.0).abs() < 0.5,
            "Expected (~50, ~100), got {:?}",
            pt
        );
    }

    /// Verify that two parallel lines return `None`.
    #[test]
    fn intersect_polar_lines_parallel_returns_none() {
        let a = PolarLine {
            r: 50.0,
            angle_in_degrees: 0,
        };
        let b = PolarLine {
            r: 100.0,
            angle_in_degrees: 0,
        };
        assert!(intersect_polar_lines(&a, &b).is_none());
    }

    /// Verify classification of lines into horizontal and vertical buckets.
    #[test]
    fn classify_lines_basic() {
        let lines = vec![
            PolarLine {
                r: 10.0,
                angle_in_degrees: 0,
            }, // horizontal
            PolarLine {
                r: 20.0,
                angle_in_degrees: 5,
            }, // horizontal
            PolarLine {
                r: 30.0,
                angle_in_degrees: 90,
            }, // vertical
            PolarLine {
                r: 40.0,
                angle_in_degrees: 85,
            }, // vertical
            PolarLine {
                r: 50.0,
                angle_in_degrees: 45,
            }, // ambiguous — discarded
            PolarLine {
                r: 60.0,
                angle_in_degrees: 170,
            }, // horizontal
        ];

        let (horiz, vert) = classify_lines(&lines);
        assert_eq!(horiz.len(), 3);
        assert_eq!(vert.len(), 2);
    }

    /// Create a synthetic image with a clear white rectangle on a dark
    /// background and verify that `correct_perspective` produces an output
    /// (exercising more of the pipeline even if the warp is imperfect).
    #[test]
    fn correct_perspective_synthetic_rectangle() {
        let (w, h) = (400u32, 500u32);
        let mut img = GrayImage::from_pixel(w, h, Luma([30u8]));

        // Draw a white rectangle from (50,60) to (350,440).
        for y in 60..440 {
            for x in 50..350 {
                img.put_pixel(x, y, Luma([240u8]));
            }
        }

        let dyn_img = DynamicImage::ImageLuma8(img);
        let enhancer = ScanEnhancer::from_dynamic(dyn_img, PaperSize::A4);
        let result = enhancer.correct_perspective();

        // The method should not panic. The output may or may not have been
        // warped (depending on whether edge detection finds a clean quad),
        // but it must always return a valid image.
        assert!(result.as_dynamic().width() > 0);
        assert!(result.as_dynamic().height() > 0);
    }
}
