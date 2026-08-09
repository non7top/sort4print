//! Producing the file that goes to the printer.
//!
//! The crop is taken from the original at full resolution and never resampled —
//! the aspect ratio is already exact, so scaling would only throw away detail
//! the lab could have used. Whatever part of the crop window falls outside the
//! photo is filled with the background colour.
//!
//! `compose` is shared with the editor's preview, which is what makes the
//! preview an actual preview: same code, same geometry, smaller pixels.

use std::path::{Path, PathBuf};

use ab_glyph::FontVec;
use anyhow::{Context, Result};
use image::codecs::jpeg::JpegEncoder;
use image::{Rgba, RgbaImage};

use crate::config::{CaptionConfig, Color, Config};
use crate::cropbox::CropBox;
use crate::datefmt::format_date;
use crate::exif_data::{self, PhotoMeta};
use crate::geo::Place;
use crate::loader;
use crate::stamp::{self, StampStyle};

/// Everything needed to burn a caption in.
pub struct CaptionRender<'a> {
    pub text: &'a str,
    pub font: &'a FontVec,
    pub config: &'a CaptionConfig,
}

/// Cuts `crop` out of `src` onto a background, and stamps the caption.
///
/// `crop` is in `src`'s coordinate space, so passing a preview image with a
/// crop expressed in preview pixels yields a faithful scaled-down rehearsal of
/// the export.
pub fn compose(
    src: &RgbaImage,
    crop: &CropBox,
    background: Color,
    caption: Option<CaptionRender<'_>>,
) -> RgbaImage {
    let (cx, cy, cw, ch) = crop.to_pixel_rect();
    let mut canvas = RgbaImage::from_pixel(
        cw,
        ch,
        Rgba([background.r, background.g, background.b, 255]),
    );

    // Intersection of the window with the photo, in source pixels.
    let sx0 = cx.max(0);
    let sy0 = cy.max(0);
    let sx1 = (cx + cw as i64).min(src.width() as i64);
    let sy1 = (cy + ch as i64).min(src.height() as i64);

    if sx1 > sx0 && sy1 > sy0 {
        let copy_w = (sx1 - sx0) as usize;
        let copy_h = (sy1 - sy0) as usize;
        let dx0 = (sx0 - cx) as usize;
        let dy0 = (sy0 - cy) as usize;

        let src_stride = src.width() as usize * 4;
        let dst_stride = cw as usize * 4;
        let src_raw = src.as_raw();
        let dst_raw: &mut [u8] = &mut canvas;

        for row in 0..copy_h {
            let s = (sy0 as usize + row) * src_stride + sx0 as usize * 4;
            let d = (dy0 + row) * dst_stride + dx0 * 4;
            let n = copy_w * 4;
            dst_raw[d..d + n].copy_from_slice(&src_raw[s..s + n]);
        }
    }

    if let Some(c) = caption {
        if c.config.enabled {
            let style = StampStyle::from_config(c.config, cw, ch);
            stamp::draw_caption(&mut canvas, c.text, c.font, &style);
        }
    }

    canvas
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportOutcome {
    pub path: PathBuf,
    pub width: u32,
    pub height: u32,
}

/// Reads the source at full resolution, composes, and writes the JPEG.
///
/// Re-exporting the same source overwrites its previous output, so adjusting a
/// crop and pressing the button again replaces the file instead of littering
/// the folder with numbered variants.
pub fn export(
    source: &Path,
    output_dir: &Path,
    crop_full_res: &CropBox,
    config: &Config,
    caption_text: &str,
    font: Option<&FontVec>,
) -> Result<ExportOutcome> {
    std::fs::create_dir_all(output_dir)
        .with_context(|| format!("creating {}", output_dir.display()))?;

    let (full, _meta) = loader::load_full(source)?;

    let caption = font.map(|font| CaptionRender {
        text: caption_text,
        font,
        config: &config.caption,
    });
    let canvas = compose(&full, crop_full_res, config.background, caption);

    let rgb = image::DynamicImage::ImageRgba8(canvas).into_rgb8();
    let (width, height) = (rgb.width(), rgb.height());

    let mut buffer = Vec::with_capacity((width as usize * height as usize) / 2);
    JpegEncoder::new_with_quality(&mut buffer, config.jpeg_quality.clamp(1, 100))
        .encode_image(&rgb)
        .context("encoding JPEG")?;

    if config.preserve_exif {
        if let Some(mut app1) = exif_data::read_exif_app1(source) {
            // The pixels are already upright, so the orientation tag has to be
            // cleared or a viewer would rotate them a second time.
            exif_data::normalize_orientation(&mut app1);
            match exif_data::insert_app1(&buffer, &app1) {
                Ok(with_exif) => buffer = with_exif,
                // Metadata is a convenience; never fail an export over it.
                Err(_) => {}
            }
        }
    }

    let path = output_path(output_dir, source);
    std::fs::write(&path, &buffer).with_context(|| format!("writing {}", path.display()))?;

    Ok(ExportOutcome {
        path,
        width,
        height,
    })
}

/// Output is always a JPEG named after the source, whatever the source format.
pub fn output_path(output_dir: &Path, source: &Path) -> PathBuf {
    let stem = source
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "image".to_string());
    output_dir.join(format!("{stem}.jpg"))
}

/// The pieces a caption template can refer to.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CaptionFields {
    pub city: String,
    pub country: String,
    pub date: String,
    pub filename: String,
}

impl CaptionFields {
    /// `{place}` is the city and country joined, which is the common case and
    /// saves the template from having to handle a missing half itself.
    fn place(&self) -> String {
        match (self.city.is_empty(), self.country.is_empty()) {
            (false, false) => format!("{}, {}", self.city, self.country),
            (false, true) => self.city.clone(),
            (true, false) => self.country.clone(),
            (true, true) => String::new(),
        }
    }
}

/// Assembles the caption from settings, EXIF and the resolved (or overridden)
/// place.
pub fn caption_text(
    config: &Config,
    meta: &PhotoMeta,
    place: Option<&Place>,
    city_override: Option<&str>,
    country_override: Option<&str>,
    filename: &str,
) -> String {
    let locale = config.locales.get_or_default(&config.date_locale);
    let fields = CaptionFields {
        city: city_override
            .map(str::to_string)
            .or_else(|| place.map(|p| p.city.clone()))
            .unwrap_or_default(),
        country: country_override
            .map(str::to_string)
            .or_else(|| place.map(|p| p.country.clone()))
            .unwrap_or_default(),
        date: meta
            .date
            .map(|d| format_date(&config.date_pattern, d, locale))
            .unwrap_or_default(),
        filename: filename.to_string(),
    };
    let text = build_caption(&config.caption.template, &fields);
    if config.caption.uppercase {
        text.to_uppercase()
    } else {
        text
    }
}

/// Substitutes the placeholders and then tidies up after any that came out
/// empty, so a photo with no GPS fix produces `Oct '25` rather than `, , Oct '25`.
pub fn build_caption(template: &str, fields: &CaptionFields) -> String {
    let replaced = template
        .replace("{place}", &fields.place())
        .replace("{city}", &fields.city)
        .replace("{country}", &fields.country)
        .replace("{date}", &fields.date)
        .replace("{filename}", &fields.filename);

    replaced
        .lines()
        .map(tidy_separators)
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Collapses runs of punctuation left behind by an empty placeholder, and
/// strips them from the ends of the line.
///
/// The gap between two pieces of real text is buffered rather than emitted as
/// it is read, and only written out once something follows it. That is what
/// lets `{city}, {country}, {date}` with no country collapse to `Paris, Oct '25`
/// while `{place} — {date}` keeps the spaces the template asked for: the
/// spacing around the separator is remembered and replayed, not normalised.
fn tidy_separators(line: &str) -> String {
    const SEPARATORS: &[char] = &[',', ';', '·', '|', '–', '—', '-', '/'];

    #[derive(Default)]
    struct Gap {
        space_before: bool,
        separator: Option<char>,
        space_after: bool,
    }

    let mut out = String::with_capacity(line.len());
    let mut gap = Gap::default();
    let mut have_content = false;

    for ch in line.chars() {
        if ch.is_whitespace() {
            if gap.separator.is_some() {
                gap.space_after = true;
            } else {
                gap.space_before = true;
            }
            continue;
        }

        if SEPARATORS.contains(&ch) {
            // A run of separators collapses to the first one; the spacing of
            // the run as a whole is what gets remembered.
            if gap.separator.is_none() {
                gap.separator = Some(ch);
            }
            continue;
        }

        // Real text. Anything before the first of it is dropped outright.
        if have_content {
            match gap.separator {
                Some(separator) => {
                    if gap.space_before {
                        out.push(' ');
                    }
                    out.push(separator);
                    if gap.space_after {
                        out.push(' ');
                    }
                }
                None if gap.space_before || gap.space_after => out.push(' '),
                None => {}
            }
        }
        gap = Gap::default();
        out.push(ch);
        have_content = true;
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::datefmt::PhotoDate;

    fn solid(w: u32, h: u32, rgb: [u8; 3]) -> RgbaImage {
        RgbaImage::from_pixel(w, h, Rgba([rgb[0], rgb[1], rgb[2], 255]))
    }

    #[test]
    fn compose_crops_the_requested_region() {
        let mut src = solid(100, 100, [10, 20, 30]);
        // Mark a 10x10 patch we will crop out.
        for y in 40..50 {
            for x in 40..50 {
                src.put_pixel(x, y, Rgba([200, 100, 50, 255]));
            }
        }
        let crop = CropBox::new(40.0, 40.0, 10.0, 10.0);
        let out = compose(&src, &crop, Color::WHITE, None);
        assert_eq!((out.width(), out.height()), (10, 10));
        assert!(out.pixels().all(|p| p.0 == [200, 100, 50, 255]));
    }

    #[test]
    fn a_window_larger_than_the_photo_gets_a_background_border() {
        let src = solid(50, 50, [0, 0, 0]);
        // Centred window twice the size: a 25 px border all round.
        let crop = CropBox::new(-25.0, -25.0, 100.0, 100.0);
        let out = compose(&src, &crop, Color::WHITE, None);
        assert_eq!((out.width(), out.height()), (100, 100));
        assert_eq!(out.get_pixel(0, 0).0, [255, 255, 255, 255], "corner is border");
        assert_eq!(out.get_pixel(50, 50).0, [0, 0, 0, 255], "middle is the photo");
        assert_eq!(out.get_pixel(99, 99).0, [255, 255, 255, 255]);
        // The photo lands exactly where the geometry says.
        assert_eq!(out.get_pixel(24, 50).0, [255, 255, 255, 255]);
        assert_eq!(out.get_pixel(25, 50).0, [0, 0, 0, 255]);
        assert_eq!(out.get_pixel(74, 50).0, [0, 0, 0, 255]);
        assert_eq!(out.get_pixel(75, 50).0, [255, 255, 255, 255]);
    }

    #[test]
    fn a_window_entirely_outside_the_photo_is_all_background() {
        let src = solid(50, 50, [0, 0, 0]);
        let crop = CropBox::new(500.0, 500.0, 20.0, 20.0);
        let out = compose(&src, &crop, Color::BLACK, None);
        assert_eq!((out.width(), out.height()), (20, 20));
        assert!(out.pixels().all(|p| p.0 == [0, 0, 0, 255]));
    }

    #[test]
    fn a_window_straddling_one_edge_copies_only_the_overlap() {
        let src = solid(50, 50, [1, 2, 3]);
        let crop = CropBox::new(-10.0, 10.0, 20.0, 20.0);
        let out = compose(&src, &crop, Color::WHITE, None);
        assert_eq!(out.get_pixel(0, 0).0, [255, 255, 255, 255]);
        assert_eq!(out.get_pixel(19, 0).0, [1, 2, 3, 255]);
    }

    #[test]
    fn compose_preserves_the_requested_proportions() {
        let src = solid(400, 300, [9, 9, 9]);
        let crop = CropBox::fit_centered(400.0, 300.0, 2.0 / 3.0);
        let out = compose(&src, &crop, Color::WHITE, None);
        let ratio = out.width() as f64 / out.height() as f64;
        assert!((ratio - 2.0 / 3.0).abs() < 0.01, "got {ratio}");
    }

    #[test]
    fn caption_template_substitution() {
        let f = CaptionFields {
            city: "Paris".into(),
            country: "France".into(),
            date: "Oct '25".into(),
            filename: "IMG_1234".into(),
        };
        assert_eq!(
            build_caption("{city}, {country}, {date}", &f),
            "Paris, France, Oct '25"
        );
        assert_eq!(build_caption("{place} — {date}", &f), "Paris, France — Oct '25");
        assert_eq!(build_caption("{filename}", &f), "IMG_1234");
    }

    #[test]
    fn missing_pieces_do_not_leave_dangling_punctuation() {
        let only_date = CaptionFields {
            date: "Oct '25".into(),
            ..Default::default()
        };
        assert_eq!(build_caption("{city}, {country}, {date}", &only_date), "Oct '25");

        let no_country = CaptionFields {
            city: "Suva".into(),
            date: "Oct '25".into(),
            ..Default::default()
        };
        assert_eq!(
            build_caption("{city}, {country}, {date}", &no_country),
            "Suva, Oct '25"
        );

        let no_date = CaptionFields {
            city: "Suva".into(),
            country: "Fiji".into(),
            ..Default::default()
        };
        assert_eq!(build_caption("{city}, {country}, {date}", &no_date), "Suva, Fiji");
    }

    #[test]
    fn the_spacing_a_template_asks_for_is_preserved() {
        let f = CaptionFields {
            city: "Paris".into(),
            date: "Oct '25".into(),
            ..Default::default()
        };
        // Spaces around the separator are kept as written...
        assert_eq!(build_caption("{city} — {date}", &f), "Paris — Oct '25");
        assert_eq!(build_caption("{city} / {date}", &f), "Paris / Oct '25");
        // ...and so is their absence.
        assert_eq!(build_caption("{city}-{date}", &f), "Paris-Oct '25");
        assert_eq!(build_caption("{city}, {date}", &f), "Paris, Oct '25");
        // A plain space with no separator survives too.
        assert_eq!(build_caption("{city} {date}", &f), "Paris Oct '25");
    }

    #[test]
    fn runs_of_separators_from_empty_fields_collapse_to_one() {
        let f = CaptionFields {
            city: "Paris".into(),
            date: "Oct '25".into(),
            ..Default::default()
        };
        assert_eq!(
            build_caption("{city}, {country}, {region}, {date}", &f),
            "Paris, {region}, Oct '25",
            "unknown placeholders stay visible so the typo can be seen"
        );
        let g = CaptionFields {
            city: "Paris".into(),
            date: "Oct '25".into(),
            ..Default::default()
        };
        assert_eq!(build_caption("{city} — {country} — {date}", &g), "Paris — Oct '25");
    }

    #[test]
    fn an_entirely_empty_caption_collapses_to_nothing() {
        assert_eq!(build_caption("{city}, {country}, {date}", &CaptionFields::default()), "");
        assert_eq!(build_caption("  —  ", &CaptionFields::default()), "");
    }

    #[test]
    fn multi_line_templates_drop_only_the_empty_lines() {
        let f = CaptionFields {
            city: "Suva".into(),
            ..Default::default()
        };
        assert_eq!(build_caption("{city}\n{date}", &f), "Suva");
        let f2 = CaptionFields {
            city: "Suva".into(),
            date: "Oct '25".into(),
            ..Default::default()
        };
        assert_eq!(build_caption("{city}\n{date}", &f2), "Suva\nOct '25");
    }

    #[test]
    fn place_collapses_a_missing_half() {
        let city_only = CaptionFields {
            city: "Suva".into(),
            ..Default::default()
        };
        assert_eq!(city_only.place(), "Suva");
        let country_only = CaptionFields {
            country: "Fiji".into(),
            ..Default::default()
        };
        assert_eq!(country_only.place(), "Fiji");
        assert_eq!(CaptionFields::default().place(), "");
    }

    #[test]
    fn caption_text_uses_config_locale_and_overrides() {
        let mut config = Config::default();
        config.date_locale = "ru".into();
        config.date_pattern = "{MMM} '{yy}".into();

        let meta = PhotoMeta {
            date: Some(PhotoDate {
                year: 2025,
                month: 10,
                day: 5,
                hour: 0,
                minute: 0,
            }),
            ..Default::default()
        };
        let place = Place {
            city: "Moscow".into(),
            country: "Russia".into(),
            country_code: "RU".into(),
            distance_km: 1.0,
        };

        assert_eq!(
            caption_text(&config, &meta, Some(&place), None, None, "IMG_1"),
            "Moscow, Russia, окт '25"
        );
        // A manual override wins over the geocoded value.
        assert_eq!(
            caption_text(&config, &meta, Some(&place), Some("Москва"), Some("Россия"), "IMG_1"),
            "Москва, Россия, окт '25"
        );
    }

    #[test]
    fn caption_text_without_gps_or_date_is_empty() {
        let config = Config::default();
        let meta = PhotoMeta::default();
        assert_eq!(caption_text(&config, &meta, None, None, None, "IMG_1"), "");
    }

    #[test]
    fn uppercase_setting_is_applied() {
        let mut config = Config::default();
        config.caption.uppercase = true;
        config.caption.template = "{city}".into();
        let place = Place {
            city: "Paris".into(),
            country: "France".into(),
            country_code: "FR".into(),
            distance_km: 0.0,
        };
        assert_eq!(
            caption_text(&config, &PhotoMeta::default(), Some(&place), None, None, ""),
            "PARIS"
        );
    }

    #[test]
    fn output_is_always_a_jpg_named_after_the_source() {
        assert_eq!(
            output_path(Path::new("/out"), Path::new("/in/IMG_0042.PNG")),
            PathBuf::from("/out/IMG_0042.jpg")
        );
    }
}
