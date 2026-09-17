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

use crate::cache::{DiskCache, Kind};
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

/// The photo's upright size, from its header alone.
///
/// A few milliseconds instead of a full decode, which is what lets a cached
/// preview be used without any stored metadata beside it: the original is still
/// the authority on its own dimensions.
pub fn upright_dimensions(path: &Path) -> Result<(u32, u32)> {
    let meta = exif_data::read_meta(path);
    let (w, h) = image::ImageReader::open(path)
        .with_context(|| format!("opening {}", path.display()))?
        .with_guessed_format()
        .with_context(|| format!("identifying {}", path.display()))?
        .into_dimensions()
        .with_context(|| format!("reading the size of {}", path.display()))?;
    Ok(if meta.orientation.swaps_axes() {
        (h, w)
    } else {
        (w, h)
    })
}

/// Quality the cache stores at. High enough that the preview is not visibly
/// worse than a fresh decode, low enough that entries stay small.
const CACHE_QUALITY: u8 = 88;

/// Version byte for the meta sidecar's layout, so a future format change can
/// tell an old sidecar apart from a corrupt one instead of misreading it.
const META_VERSION: u8 = 1;

/// Packs a hit's full size and EXIF fields into the sidecar written beside a
/// cached JPEG, so a later hit can read them back instead of reopening the
/// original. See [`decode_meta`].
fn encode_meta(full_w: u32, full_h: u32, meta: &PhotoMeta) -> Vec<u8> {
    let mut out = Vec::with_capacity(32);
    out.push(META_VERSION);
    out.extend_from_slice(&full_w.to_le_bytes());
    out.extend_from_slice(&full_h.to_le_bytes());
    out.push(meta.orientation.as_u8());
    match meta.date {
        Some(date) => {
            out.push(1);
            out.extend_from_slice(&date.year.to_le_bytes());
            out.extend_from_slice(&date.month.to_le_bytes());
            out.extend_from_slice(&date.day.to_le_bytes());
            out.extend_from_slice(&date.hour.to_le_bytes());
            out.extend_from_slice(&date.minute.to_le_bytes());
        }
        None => out.push(0),
    }
    match meta.gps {
        Some((lat, lon)) => {
            out.push(1);
            out.extend_from_slice(&lat.to_le_bytes());
            out.extend_from_slice(&lon.to_le_bytes());
        }
        None => out.push(0),
    }
    out
}

/// The inverse of [`encode_meta`]. `None` on anything unrecognised — a
/// version mismatch or truncated file — so the caller falls back to the
/// original exactly as it would for a missing sidecar.
fn decode_meta(bytes: &[u8]) -> Option<(u32, u32, PhotoMeta)> {
    let mut pos = 0;
    let take = |pos: &mut usize, n: usize| -> Option<&[u8]> {
        let slice = bytes.get(*pos..*pos + n)?;
        *pos += n;
        Some(slice)
    };

    if *take(&mut pos, 1)?.first()? != META_VERSION {
        return None;
    }
    let full_w = u32::from_le_bytes(take(&mut pos, 4)?.try_into().ok()?);
    let full_h = u32::from_le_bytes(take(&mut pos, 4)?.try_into().ok()?);
    let orientation = Orientation::from_exif(*take(&mut pos, 1)?.first()? as u32);

    let date = if *take(&mut pos, 1)?.first()? == 1 {
        Some(crate::datefmt::PhotoDate {
            year: i32::from_le_bytes(take(&mut pos, 4)?.try_into().ok()?),
            month: u32::from_le_bytes(take(&mut pos, 4)?.try_into().ok()?),
            day: u32::from_le_bytes(take(&mut pos, 4)?.try_into().ok()?),
            hour: u32::from_le_bytes(take(&mut pos, 4)?.try_into().ok()?),
            minute: u32::from_le_bytes(take(&mut pos, 4)?.try_into().ok()?),
        })
    } else {
        None
    };

    let gps = if *take(&mut pos, 1)?.first()? == 1 {
        let lat = f64::from_le_bytes(take(&mut pos, 8)?.try_into().ok()?);
        let lon = f64::from_le_bytes(take(&mut pos, 8)?.try_into().ok()?);
        Some((lat, lon))
    } else {
        None
    };

    Some((
        full_w,
        full_h,
        PhotoMeta {
            date,
            gps,
            orientation,
        },
    ))
}

/// Reads a cache hit's size and EXIF fields from its sidecar, without opening
/// the original. For a hit cached before the sidecar existed, falls back to
/// the original once and backfills the sidecar so that read is the last one —
/// the existing image entry is never rewritten or regenerated.
fn cached_dims_and_meta(cache: &DiskCache, key: &str, path: &Path) -> Option<(u32, u32, PhotoMeta)> {
    if let Some(parsed) = cache.read_meta(key).and_then(|bytes| decode_meta(&bytes)) {
        return Some(parsed);
    }
    let meta = exif_data::read_meta(path);
    let (full_w, full_h) = upright_dimensions(path).ok()?;
    let _ = cache.write_meta(key, &encode_meta(full_w, full_h, &meta));
    Some((full_w, full_h, meta))
}

/// A `View` preview at `max_px`, built by shrinking a *larger* size that is
/// already cached, rather than reopening the original. `None` when nothing
/// bigger is cached either, so the caller falls through to decoding the
/// source.
///
/// This is what lets a legacy entry cached at an old, wider bucket (or one
/// simply cached back when the editor pane was wider) converge on the
/// smaller "Read all"/quick-view size for free: the pixels it would decode
/// to are already sitting on disk, just at a size nobody asked to keep.
fn downscale_from_larger_cached(
    cache: &DiskCache,
    key: &str,
    max_px: u32,
    path: &Path,
) -> Option<Preview> {
    let bytes = DiskCache::SIZE_BUCKETS
        .iter()
        .copied()
        .filter(|&size| size > max_px)
        .find_map(|size| cache.read(key, Kind::View, size))?;
    let (full_w, full_h, meta) = cached_dims_and_meta(cache, key, path)?;
    let image = decode_bytes(&bytes).ok()?;
    let shrunk = shrink_to(image, max_px).into_rgba8();

    if let Ok(out) = encode_jpeg(&shrunk, CACHE_QUALITY) {
        let _ = cache.write(key, Kind::View, max_px, &out);
    }
    let scale = if full_w > 0 {
        shrunk.width() as f64 / full_w as f64
    } else {
        1.0
    };
    Some(Preview {
        rgba: shrunk,
        full_w,
        full_h,
        scale,
        meta,
    })
}

/// A preview, from the cache when it is there and from the original when it is
/// not — in which case the result is written to the cache on the way out.
///
/// The cached image is stored already upright and already shrunk, so a hit is a
/// small JPEG decode rather than a large one plus a rotate and a resample.
pub fn load_preview_cached(
    path: &Path,
    max_px: u32,
    cache: Option<&DiskCache>,
) -> Result<Preview> {
    let key = cache.and_then(|_| DiskCache::key_for(path));

    if let (Some(cache), Some(key)) = (cache, key.as_deref()) {
        if let Some(bytes) = cache.read(key, Kind::View, max_px) {
            if let Some((full_w, full_h, meta)) = cached_dims_and_meta(cache, key, path) {
                if let Ok(rgba) = decode_bytes(&bytes) {
                    let rgba = rgba.into_rgba8();
                    let scale = if full_w > 0 {
                        rgba.width() as f64 / full_w as f64
                    } else {
                        1.0
                    };
                    return Ok(Preview {
                        rgba,
                        full_w,
                        full_h,
                        scale,
                        meta,
                    });
                }
            }
        }
        if let Some(preview) = downscale_from_larger_cached(cache, key, max_px, path) {
            return Ok(preview);
        }
    }

    let preview = load_preview(path, max_px)?;

    if let (Some(cache), Some(key)) = (cache, key.as_deref()) {
        if let Ok(bytes) = encode_jpeg(&preview.rgba, CACHE_QUALITY) {
            let _ = cache.write(key, Kind::View, max_px, &bytes);
        }
        let _ = cache.write_meta(key, &encode_meta(preview.full_w, preview.full_h, &preview.meta));
    }
    Ok(preview)
}

/// A filmstrip-sized image.
///
/// Tries the thumbnail the camera already put in the file first: it is two
/// orders of magnitude cheaper than decoding the original, and it is the reason
/// a list of eleven thousand photos can be scrolled at all. It is only accepted
/// when its shape matches the photo's, since a few cameras pad theirs to a fixed
/// size and a stretched thumbnail is worse than a slow one.
pub fn load_thumb_cached(
    path: &Path,
    max_px: u32,
    cache: Option<&DiskCache>,
) -> Result<Preview> {
    let key = cache.and_then(|_| DiskCache::key_for(path));

    if let (Some(cache), Some(key)) = (cache, key.as_deref()) {
        if let Some(bytes) = cache.read(key, Kind::Thumb, max_px) {
            if let Some((full_w, full_h, meta)) = cached_dims_and_meta(cache, key, path) {
                if let Ok(image) = decode_bytes(&bytes) {
                    let rgba = image.into_rgba8();
                    let scale = if full_w > 0 {
                        rgba.width() as f64 / full_w as f64
                    } else {
                        1.0
                    };
                    return Ok(Preview {
                        rgba,
                        full_w,
                        full_h,
                        scale,
                        meta,
                    });
                }
            }
        }
    }

    let meta = exif_data::read_meta(path);

    if let Some(embedded) = exif_data::read_exif_thumbnail(path) {
        if let Ok(image) = decode_bytes(&embedded) {
            let image = apply_orientation(image, meta.orientation);
            if let Ok((full_w, full_h)) = upright_dimensions(path) {
                if shapes_agree(image.width(), image.height(), full_w, full_h) {
                    let shrunk = shrink_to(image, max_px).into_rgba8();
                    if let (Some(cache), Some(key)) = (cache, key.as_deref()) {
                        if let Ok(bytes) = encode_jpeg(&shrunk, CACHE_QUALITY) {
                            let _ = cache.write(key, Kind::Thumb, max_px, &bytes);
                        }
                        let _ = cache.write_meta(key, &encode_meta(full_w, full_h, &meta));
                    }
                    let scale = if full_w > 0 {
                        shrunk.width() as f64 / full_w as f64
                    } else {
                        1.0
                    };
                    return Ok(Preview {
                        rgba: shrunk,
                        full_w,
                        full_h,
                        scale,
                        meta,
                    });
                }
            }
        }
    }

    // Nothing usable to shortcut with: decode the original.
    let preview = load_preview(path, max_px)?;
    if let (Some(cache), Some(key)) = (cache, key.as_deref()) {
        if let Ok(bytes) = encode_jpeg(&preview.rgba, CACHE_QUALITY) {
            let _ = cache.write(key, Kind::Thumb, max_px, &bytes);
        }
        let _ = cache.write_meta(key, &encode_meta(preview.full_w, preview.full_h, &preview.meta));
    }
    Ok(preview)
}

/// Within a few per cent is the same shape; a padded thumbnail is not.
fn shapes_agree(thumb_w: u32, thumb_h: u32, full_w: u32, full_h: u32) -> bool {
    if thumb_h == 0 || full_h == 0 {
        return false;
    }
    let a = thumb_w as f64 / thumb_h as f64;
    let b = full_w as f64 / full_h as f64;
    (a - b).abs() / b < 0.06
}

fn shrink_to(image: DynamicImage, max_px: u32) -> DynamicImage {
    let long = image.width().max(image.height());
    if long <= max_px || max_px == 0 {
        return image;
    }
    let scale = max_px as f64 / long as f64;
    let w = ((image.width() as f64 * scale).round() as u32).max(1);
    let h = ((image.height() as f64 * scale).round() as u32).max(1);
    image.resize_exact(w, h, FilterType::Triangle)
}

fn decode_bytes(bytes: &[u8]) -> Result<DynamicImage> {
    image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .context("identifying a cached image")?
        .decode()
        .context("decoding a cached image")
}

fn encode_jpeg(rgba: &image::RgbaImage, quality: u8) -> Result<Vec<u8>> {
    let rgb = DynamicImage::ImageRgba8(rgba.clone()).into_rgb8();
    let mut out = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, quality)
        .encode_image(&rgb)
        .context("encoding a cache entry")?;
    Ok(out)
}

/// The original at full resolution, rotated upright. This is what the export
/// crops, so no resampling happens anywhere in the pipeline.
pub fn load_full(path: &Path) -> Result<(RgbaImage, PhotoMeta)> {
    let meta = exif_data::read_meta(path);
    let image = decode(path)?;
    let image = apply_orientation(image, meta.orientation);
    Ok((image.into_rgba8(), meta))
}

/// Ceiling on what one decode may allocate.
///
/// The `image` crate's default cap is generous for a web service and much too
/// small for a modern phone: a 50-megapixel photo needs 200 MB as RGBA before
/// anything else, and the decode simply failed with "Memory limit exceeded".
/// This is raised to cover any real camera — 2 GiB is over 500 megapixels —
/// while still refusing a corrupt header that claims an absurd size, which
/// would otherwise abort the process on a failed allocation rather than
/// reporting a bad file.
const MAX_DECODE_BYTES: u64 = 2 * 1024 * 1024 * 1024;

fn decode(path: &Path) -> Result<DynamicImage> {
    let mut reader = image::ImageReader::open(path)
        .with_context(|| format!("opening {}", path.display()))?
        .with_guessed_format()
        .with_context(|| format!("identifying {}", path.display()))?;

    let mut limits = image::Limits::no_limits();
    limits.max_alloc = Some(MAX_DECODE_BYTES);
    reader.limits(limits);

    reader
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

    #[test]
    fn meta_sidecar_round_trips_with_date_and_gps() {
        let meta = PhotoMeta {
            date: Some(crate::datefmt::PhotoDate {
                year: 2024,
                month: 7,
                day: 3,
                hour: 14,
                minute: 5,
            }),
            gps: Some((51.5, -0.12)),
            orientation: Orientation::Rotate90,
        };
        let bytes = encode_meta(4000, 3000, &meta);
        let (full_w, full_h, decoded) = decode_meta(&bytes).expect("a freshly encoded sidecar decodes");
        assert_eq!((full_w, full_h), (4000, 3000));
        assert_eq!(decoded, meta);
    }

    #[test]
    fn meta_sidecar_round_trips_with_no_date_or_gps() {
        let meta = PhotoMeta::default();
        let bytes = encode_meta(1, 1, &meta);
        let (_, _, decoded) = decode_meta(&bytes).expect("a freshly encoded sidecar decodes");
        assert_eq!(decoded, meta);
    }

    #[test]
    fn a_truncated_or_wrong_version_sidecar_is_rejected_rather_than_panicking() {
        assert!(decode_meta(&[]).is_none());
        assert!(decode_meta(&[META_VERSION]).is_none());
        let mut bytes = encode_meta(100, 100, &PhotoMeta::default());
        bytes[0] = META_VERSION.wrapping_add(1);
        assert!(decode_meta(&bytes).is_none());
    }

    fn scratch_cache(name: &str) -> (std::path::PathBuf, DiskCache) {
        let dir = std::env::temp_dir().join(format!(
            "sort4print-loader-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        let cache = DiskCache::new(dir.join("cache"), 10_000_000);
        (dir, cache)
    }

    #[test]
    fn a_missing_size_is_served_by_shrinking_a_larger_cached_entry() {
        let (dir, cache) = scratch_cache("downscale-hit");
        // A path that does not exist: the point of this test is that the
        // downscale path never has to open it.
        let path = dir.join("nonexistent.jpg");
        let key = "abc0000000000000";

        let big = RgbaImage::from_pixel(1800, 1200, Rgba([10, 20, 30, 255]));
        let bytes = encode_jpeg(&big, CACHE_QUALITY).unwrap();
        cache.write(key, Kind::View, 1800, &bytes).unwrap();
        cache
            .write_meta(key, &encode_meta(1800, 1200, &PhotoMeta::default()))
            .unwrap();

        let preview = downscale_from_larger_cached(&cache, key, 1000, &path)
            .expect("a larger cached entry should serve a smaller request");
        assert_eq!((preview.full_w, preview.full_h), (1800, 1200));
        assert!(preview.rgba.width() <= 1000 && preview.rgba.height() <= 1000);
        // The smaller size is now cached too, produced from the larger entry
        // alone rather than by reopening the (nonexistent) original.
        assert!(cache.contains(key, Kind::View, 1000));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn nothing_larger_cached_means_no_downscale_is_produced() {
        let (dir, cache) = scratch_cache("downscale-miss");
        let path = dir.join("nonexistent.jpg");
        assert!(downscale_from_larger_cached(&cache, "0000000000000000", 1000, &path).is_none());
        std::fs::remove_dir_all(&dir).ok();
    }
}
