//! Reading what the phone wrote into the file: capture time, GPS fix, and the
//! orientation flag that decides which way is up.
//!
//! Also handles carrying the original EXIF block into the exported crop. That
//! block is copied verbatim apart from the orientation tag, which is forced to
//! "normal" — by export time the pixels have already been rotated, so leaving
//! the original value there would make viewers rotate them a second time.

use std::io::BufReader;
use std::path::Path;

use anyhow::{Context, Result};
use exif::{In, Tag, Value};

use crate::datefmt::PhotoDate;

/// How the stored pixels must be transformed to be displayed upright.
/// The numbering is EXIF's own (tag 0x0112).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Orientation {
    #[default]
    Normal,
    FlipHorizontal,
    Rotate180,
    FlipVertical,
    Transpose,
    Rotate90,
    Transverse,
    Rotate270,
}

impl Orientation {
    pub fn from_exif(value: u32) -> Orientation {
        match value {
            2 => Orientation::FlipHorizontal,
            3 => Orientation::Rotate180,
            4 => Orientation::FlipVertical,
            5 => Orientation::Transpose,
            6 => Orientation::Rotate90,
            7 => Orientation::Transverse,
            8 => Orientation::Rotate270,
            _ => Orientation::Normal,
        }
    }

    /// True when applying it exchanges width and height.
    pub fn swaps_axes(self) -> bool {
        matches!(
            self,
            Orientation::Transpose
                | Orientation::Rotate90
                | Orientation::Transverse
                | Orientation::Rotate270
        )
    }

    pub fn is_identity(self) -> bool {
        self == Orientation::Normal
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct PhotoMeta {
    pub date: Option<PhotoDate>,
    /// Decimal degrees, north and east positive.
    pub gps: Option<(f64, f64)>,
    pub orientation: Orientation,
}

/// Never fails on a missing or broken EXIF block — a photo without metadata is
/// perfectly usable, it just gets an empty caption and no rotation.
pub fn read_meta(path: &Path) -> PhotoMeta {
    read_meta_inner(path).unwrap_or_default()
}

fn read_meta_inner(path: &Path) -> Result<PhotoMeta> {
    let file = std::fs::File::open(path)?;
    let mut reader = BufReader::new(file);
    let exif = exif::Reader::new().read_from_container(&mut reader)?;

    let date = [Tag::DateTimeOriginal, Tag::DateTimeDigitized, Tag::DateTime]
        .iter()
        .find_map(|tag| {
            let field = exif.get_field(*tag, In::PRIMARY)?;
            PhotoDate::parse_exif(&field.display_value().to_string())
        });

    let orientation = exif
        .get_field(Tag::Orientation, In::PRIMARY)
        .and_then(|f| f.value.get_uint(0))
        .map(Orientation::from_exif)
        .unwrap_or_default();

    let lat = coordinate(&exif, Tag::GPSLatitude, Tag::GPSLatitudeRef, 'S');
    let lon = coordinate(&exif, Tag::GPSLongitude, Tag::GPSLongitudeRef, 'W');
    let gps = match (lat, lon) {
        // A literal 0,0 fix is Null Island: phones write it when the GPS block
        // exists but never got a lock, so it is noise rather than a location.
        (Some(lat), Some(lon)) if lat != 0.0 || lon != 0.0 => Some((lat, lon)),
        _ => None,
    };

    Ok(PhotoMeta {
        date,
        gps,
        orientation,
    })
}

/// EXIF stores coordinates as three rationals (degrees, minutes, seconds) plus
/// a separate hemisphere letter.
fn coordinate(exif: &exif::Exif, tag: Tag, ref_tag: Tag, negative_ref: char) -> Option<f64> {
    let field = exif.get_field(tag, In::PRIMARY)?;
    let Value::Rational(ref parts) = field.value else {
        return None;
    };
    let value = |i: usize| parts.get(i).map(|r| r.to_f64()).unwrap_or(0.0);
    let degrees = value(0) + value(1) / 60.0 + value(2) / 3600.0;
    if !degrees.is_finite() || degrees > 180.0 {
        return None;
    }

    let hemisphere = exif
        .get_field(ref_tag, In::PRIMARY)
        .map(|f| f.display_value().to_string())
        .unwrap_or_default()
        .trim()
        .chars()
        .next()
        .map(|c| c.to_ascii_uppercase());

    Some(if hemisphere == Some(negative_ref) {
        -degrees
    } else {
        degrees
    })
}

const EXIF_HEADER: &[u8; 6] = b"Exif\0\0";

/// Extracts the APP1/Exif segment payload (including the `Exif\0\0` header)
/// from a JPEG, ready to be spliced into the exported file.
pub fn read_exif_app1(path: &Path) -> Option<Vec<u8>> {
    let bytes = std::fs::read(path).ok()?;
    extract_app1(&bytes).map(|s| s.to_vec())
}

fn extract_app1(bytes: &[u8]) -> Option<&[u8]> {
    if bytes.get(..2)? != [0xFF, 0xD8] {
        return None;
    }
    let mut pos = 2;
    loop {
        // Segments are 0xFF followed by a marker; fill bytes of 0xFF are legal.
        if *bytes.get(pos)? != 0xFF {
            return None;
        }
        while *bytes.get(pos)? == 0xFF {
            pos += 1;
        }
        let marker = *bytes.get(pos)?;
        pos += 1;
        // Start of scan or end of image: no metadata segments beyond here.
        if marker == 0xDA || marker == 0xD9 {
            return None;
        }
        let len = u16::from_be_bytes(bytes.get(pos..pos + 2)?.try_into().ok()?) as usize;
        if len < 2 {
            return None;
        }
        let payload = bytes.get(pos + 2..pos + len)?;
        if marker == 0xE1 && payload.starts_with(EXIF_HEADER) {
            return Some(payload);
        }
        pos += len;
    }
}

/// Rewrites the orientation tag inside an APP1 payload to 1 ("normal").
///
/// Returns false when the tag is absent or the block is not walkable, which is
/// harmless: an absent tag already means normal.
pub fn normalize_orientation(app1: &mut [u8]) -> bool {
    let Some(tiff) = app1.get_mut(EXIF_HEADER.len()..) else {
        return false;
    };
    let big_endian = match tiff.get(..2) {
        Some(b"MM") => true,
        Some(b"II") => false,
        _ => return false,
    };
    let u16_at = |b: &[u8], o: usize| -> Option<u16> {
        let raw: [u8; 2] = b.get(o..o + 2)?.try_into().ok()?;
        Some(if big_endian {
            u16::from_be_bytes(raw)
        } else {
            u16::from_le_bytes(raw)
        })
    };
    let u32_at = |b: &[u8], o: usize| -> Option<u32> {
        let raw: [u8; 4] = b.get(o..o + 4)?.try_into().ok()?;
        Some(if big_endian {
            u32::from_be_bytes(raw)
        } else {
            u32::from_le_bytes(raw)
        })
    };

    if u16_at(tiff, 2) != Some(42) {
        return false;
    }
    let Some(ifd0) = u32_at(tiff, 4).map(|v| v as usize) else {
        return false;
    };
    let Some(count) = u16_at(tiff, ifd0) else {
        return false;
    };

    for i in 0..count as usize {
        let entry = ifd0 + 2 + i * 12;
        if u16_at(tiff, entry) != Some(0x0112) {
            continue;
        }
        // Type SHORT, count 1: the value lives inline in the entry's last four
        // bytes, first two of them for a SHORT.
        if u16_at(tiff, entry + 2) != Some(3) {
            return false;
        }
        let value_off = entry + 8;
        let Some(slot) = tiff.get_mut(value_off..value_off + 2) else {
            return false;
        };
        let one = if big_endian {
            1u16.to_be_bytes()
        } else {
            1u16.to_le_bytes()
        };
        slot.copy_from_slice(&one);
        return true;
    }
    false
}

/// Splices an APP1 payload into a freshly encoded JPEG, directly after SOI.
/// Any APP1 the encoder itself wrote is dropped first so the file ends up with
/// exactly one.
pub fn insert_app1(jpeg: &[u8], app1: &[u8]) -> Result<Vec<u8>> {
    anyhow::ensure!(
        jpeg.get(..2) == Some(&[0xFF, 0xD8][..]),
        "encoded image is not a JPEG"
    );
    let segment_len = app1.len() + 2;
    anyhow::ensure!(
        segment_len <= u16::MAX as usize,
        "EXIF block is too large for one APP1 segment"
    );

    let body = strip_existing_app1(jpeg).context("walking the encoded JPEG")?;

    let mut out = Vec::with_capacity(jpeg.len() + segment_len + 4);
    out.extend_from_slice(&[0xFF, 0xD8]);
    out.extend_from_slice(&[0xFF, 0xE1]);
    out.extend_from_slice(&(segment_len as u16).to_be_bytes());
    out.extend_from_slice(app1);
    out.extend_from_slice(&body);
    Ok(out)
}

/// Everything after SOI, minus any APP1 segments.
///
/// Walking stops at the start of scan: from there on the bytes are entropy
/// coded and no longer segment-structured.
fn strip_existing_app1(jpeg: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(jpeg.len());
    let mut pos = 2;
    loop {
        if jpeg.get(pos) != Some(&0xFF) {
            out.extend_from_slice(jpeg.get(pos..)?);
            return Some(out);
        }
        let mut marker_pos = pos;
        while *jpeg.get(marker_pos)? == 0xFF {
            marker_pos += 1;
        }
        let marker = *jpeg.get(marker_pos)?;
        if marker == 0xDA || marker == 0xD9 {
            out.extend_from_slice(jpeg.get(pos..)?);
            return Some(out);
        }
        let len_pos = marker_pos + 1;
        let len = u16::from_be_bytes(jpeg.get(len_pos..len_pos + 2)?.try_into().ok()?) as usize;
        if len < 2 {
            return None;
        }
        let next = len_pos + len;
        if marker != 0xE1 {
            out.extend_from_slice(jpeg.get(pos..next)?);
        }
        pos = next;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn orientation_axis_swaps() {
        assert!(Orientation::from_exif(6).swaps_axes());
        assert!(Orientation::from_exif(8).swaps_axes());
        assert!(!Orientation::from_exif(3).swaps_axes());
        assert!(Orientation::from_exif(1).is_identity());
        // Unknown values degrade to normal rather than erroring.
        assert!(Orientation::from_exif(99).is_identity());
    }

    #[test]
    fn missing_file_yields_empty_metadata() {
        let meta = read_meta(Path::new("/nonexistent/photo.jpg"));
        assert_eq!(meta, PhotoMeta::default());
    }

    /// Minimal little-endian TIFF with a single IFD0 entry: Orientation = 6.
    fn app1_with_orientation(value: u16) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(EXIF_HEADER);
        v.extend_from_slice(b"II");
        v.extend_from_slice(&42u16.to_le_bytes());
        v.extend_from_slice(&8u32.to_le_bytes()); // IFD0 offset
        v.extend_from_slice(&1u16.to_le_bytes()); // entry count
        v.extend_from_slice(&0x0112u16.to_le_bytes()); // tag
        v.extend_from_slice(&3u16.to_le_bytes()); // type SHORT
        v.extend_from_slice(&1u32.to_le_bytes()); // count
        v.extend_from_slice(&value.to_le_bytes());
        v.extend_from_slice(&[0, 0]); // padding of the 4-byte value slot
        v.extend_from_slice(&0u32.to_le_bytes()); // next IFD
        v
    }

    #[test]
    fn orientation_tag_is_reset_in_place() {
        let mut app1 = app1_with_orientation(6);
        assert!(normalize_orientation(&mut app1));
        // The value slot sits 8 bytes into the entry, which starts at 6+10.
        assert_eq!(&app1[24..26], &1u16.to_le_bytes());
        // Idempotent, and still reports success on an already-normal block.
        assert!(normalize_orientation(&mut app1));
    }

    #[test]
    fn normalize_declines_garbage_without_panicking() {
        assert!(!normalize_orientation(&mut []));
        assert!(!normalize_orientation(&mut b"Exif\0\0XX".to_vec()));
        assert!(!normalize_orientation(&mut b"Exif\0\0II".to_vec()));
    }

    #[test]
    fn app1_is_spliced_after_soi() {
        // SOI, a stub APP0, then "scan" bytes.
        let jpeg = [
            0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x04, 0xAA, 0xBB, 0xFF, 0xDA, 0x01, 0x02,
        ];
        let app1 = app1_with_orientation(1);
        let out = insert_app1(&jpeg, &app1).unwrap();
        assert_eq!(&out[..2], &[0xFF, 0xD8]);
        assert_eq!(&out[2..4], &[0xFF, 0xE1]);
        let len = u16::from_be_bytes([out[4], out[5]]) as usize;
        assert_eq!(len, app1.len() + 2);
        assert_eq!(&out[6..6 + app1.len()], &app1[..]);
        // The original body follows untouched.
        assert!(out.ends_with(&[0xFF, 0xDA, 0x01, 0x02]));
        assert_eq!(extract_app1(&out).unwrap(), &app1[..]);
    }

    #[test]
    fn insert_rejects_non_jpeg_input() {
        assert!(insert_app1(b"not a jpeg", &[1, 2, 3]).is_err());
    }

    #[test]
    fn extract_finds_nothing_in_a_jpeg_without_exif() {
        let jpeg = [0xFF, 0xD8, 0xFF, 0xDA, 0x00];
        assert!(extract_app1(&jpeg).is_none());
    }
}
