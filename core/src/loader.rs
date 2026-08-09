//! Getting pixels off disk and the right way up.
//!
//! Two entry points with the same orientation handling: a downscaled preview
//! for the editor, and the untouched full-resolution image for the export. The
//! preview carries the scale factor it was built with, which is what lets the
//! editor work in preview pixels and still hand the exporter a crop rectangle
//! in original pixels.

use std::path::Path;

use anyhow::{Context, Result};
use image::{imageops::FilterType, DynamicImage, RgbaImage};

use crate::exif_data::{self, Orientation, PhotoMeta};

/// File extensions the folder scanner accepts.
///
/// HEIC/HEIF — what an iPhone shoots by default — is deliberately absent:
/// decoding it needs libheif, a C dependency that would end the "one small
/// self-contained exe" property. Set the phone to "Most Compatible", or
/// convert first.
pub const SUPPORTED_EXTENSIONS: &[&str] = &[
    "jpg", "jpeg", "jpe", "png", "tif", "tiff", "webp", "bmp",
];

pub fn is_supported(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| SUPPORTED_EXTENSIONS.contains(&e.to_lowercase().as_str()))
        .unwrap_or(false)
}

/// Every supported image in a folder, sorted by name, non-recursive.
pub fn scan_folder(dir: &Path) -> Result<Vec<std::path::PathBuf>> {
    let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(dir)
        .with_context(|| format!("reading {}", dir.display()))?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file() && is_supported(p))
        .collect();
    files.sort_by_key(|p| {
        p.file_name()
            .map(|n| n.to_string_lossy().to_lowercase())
            .unwrap_or_default()
    });
    Ok(files)
}

#[derive(Debug, Clone)]
pub struct Preview {
    pub rgba: RgbaImage,
    /// Size of the upright full-resolution image.
    pub full_w: u32,
    pub full_h: u32,
    /// preview pixels per full-resolution pixel, i.e. <= 1.
    pub scale: f64,
    pub meta: PhotoMeta,
}

impl Preview {
    pub fn to_full(&self, v: f64) -> f64 {
        v / self.scale
    }

    pub fn to_preview(&self, v: f64) -> f64 {
        v * self.scale
    }
}

/// Decodes, rotates upright, and shrinks so the long side is at most
/// `max_px`. An image already smaller than that is left alone rather than
/// upscaled.
pub fn load_preview(path: &Path, max_px: u32) -> Result<Preview> {
    let meta = exif_data::read_meta(path);
    let image = decode(path)?;
    let image = apply_orientation(image, meta.orientation);

    let (full_w, full_h) = (image.width(), image.height());
    let long = full_w.max(full_h);
    let (rgba, scale) = if long > max_px && max_px > 0 {
        let scale = max_px as f64 / long as f64;
        let w = ((full_w as f64 * scale).round() as u32).max(1);
        let h = ((full_h as f64 * scale).round() as u32).max(1);
        // Triangle rather than Lanczos: this is a screen preview that gets
        // rebuilt on every folder step, and the crop it feeds is applied to the
        // original anyway.
        (image.resize_exact(w, h, FilterType::Triangle).into_rgba8(), scale)
    } else {
        (image.into_rgba8(), 1.0)
    };

    Ok(Preview {
        rgba,
        full_w,
        full_h,
        scale,
        meta,
    })
}

/// The original at full resolution, rotated upright. This is what the export
/// crops, so no resampling happens anywhere in the pipeline.
pub fn load_full(path: &Path) -> Result<(RgbaImage, PhotoMeta)> {
    let meta = exif_data::read_meta(path);
    let image = decode(path)?;
    let image = apply_orientation(image, meta.orientation);
    Ok((image.into_rgba8(), meta))
}

fn decode(path: &Path) -> Result<DynamicImage> {
    image::ImageReader::open(path)
        .with_context(|| format!("opening {}", path.display()))?
        .with_guessed_format()
        .with_context(|| format!("identifying {}", path.display()))?
        .decode()
        .with_context(|| format!("decoding {}", path.display()))
}

pub fn apply_orientation(image: DynamicImage, orientation: Orientation) -> DynamicImage {
    match orientation {
        Orientation::Normal => image,
        Orientation::FlipHorizontal => image.fliph(),
        Orientation::Rotate180 => image.rotate180(),
        Orientation::FlipVertical => image.flipv(),
        Orientation::Transpose => image.rotate90().fliph(),
        Orientation::Rotate90 => image.rotate90(),
        Orientation::Transverse => image.rotate270().fliph(),
        Orientation::Rotate270 => image.rotate270(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgba, RgbaImage};

    #[test]
    fn extension_filter() {
        assert!(is_supported(Path::new("a/b/IMG_0001.JPG")));
        assert!(is_supported(Path::new("x.jpeg")));
        assert!(!is_supported(Path::new("x.heic")), "HEIC is not decodable here");
        assert!(!is_supported(Path::new("x.txt")));
        assert!(!is_supported(Path::new("noextension")));
    }

    #[test]
    fn orientation_six_rotates_a_landscape_into_a_portrait() {
        let img = DynamicImage::ImageRgba8(RgbaImage::from_pixel(40, 20, Rgba([1, 2, 3, 255])));
        let out = apply_orientation(img, Orientation::Rotate90);
        assert_eq!((out.width(), out.height()), (20, 40));
    }

    #[test]
    fn orientation_one_is_a_no_op() {
        let img = DynamicImage::ImageRgba8(RgbaImage::from_pixel(40, 20, Rgba([1, 2, 3, 255])));
        let out = apply_orientation(img, Orientation::Normal);
        assert_eq!((out.width(), out.height()), (40, 20));
    }

    #[test]
    fn flips_move_a_marker_pixel_to_the_expected_corner() {
        let mut base = RgbaImage::from_pixel(4, 2, Rgba([0, 0, 0, 255]));
        base.put_pixel(0, 0, Rgba([255, 0, 0, 255]));

        let flipped = apply_orientation(DynamicImage::ImageRgba8(base.clone()), Orientation::FlipHorizontal);
        assert_eq!(flipped.to_rgba8().get_pixel(3, 0).0[0], 255);

        let flipped = apply_orientation(DynamicImage::ImageRgba8(base.clone()), Orientation::FlipVertical);
        assert_eq!(flipped.to_rgba8().get_pixel(0, 1).0[0], 255);

        let rotated = apply_orientation(DynamicImage::ImageRgba8(base), Orientation::Rotate180);
        assert_eq!(rotated.to_rgba8().get_pixel(3, 1).0[0], 255);
    }

    #[test]
    fn preview_scale_maps_back_to_original_pixels() {
        let p = Preview {
            rgba: RgbaImage::new(1, 1),
            full_w: 4000,
            full_h: 3000,
            scale: 0.25,
            meta: PhotoMeta::default(),
        };
        assert!((p.to_full(100.0) - 400.0).abs() < 1e-9);
        assert!((p.to_preview(400.0) - 100.0).abs() < 1e-9);
    }

    #[test]
    fn scanning_a_missing_folder_is_an_error_not_a_panic() {
        assert!(scan_folder(Path::new("/nonexistent/folder")).is_err());
    }
}
