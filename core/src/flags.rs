//! Country flags, baked into the executable.
//!
//! There is no font route to this, which is worth writing down because it is
//! the obvious first idea. The Unicode way to write a flag is a pair of
//! regional-indicator letters, and Windows deliberately renders those as two
//! boxed letters rather than a flag — Segoe UI Emoji ships no flag glyphs at
//! all. A colour emoji font could be carried instead, but the caption renderer
//! rasterises glyph *outlines*, and a colour font stores its flags as embedded
//! bitmaps or layered paint tables, neither of which an outline rasteriser can
//! see.
//!
//! So flags are drawings. Public-domain SVGs, gzipped and packed into one blob
//! that `include_bytes!` puts in the binary, exactly as the city database is:
//! no files to install and nothing to download. They stay as drawings rather
//! than pictures of a chosen size because a print can be any size at any
//! density — each flag is rendered at exactly the height the caption needs, so
//! it is never soft on a big print and never wasteful on a small one. The whole
//! set costs less compressed than one fixed-size raster of it would.

use std::collections::BTreeMap;
use std::sync::{Mutex, OnceLock};

use anyhow::{bail, Context, Result};
use image::RgbaImage;

/// The packed blob produced by `tools/pack-flags`. See that crate for the
/// format; the two must be changed together.
static EMBEDDED: &[u8] = include_bytes!("../../assets/flags.bin");

const MAGIC: &[u8; 4] = b"S4PF";
const VERSION: u8 = 2;

/// Rendered flags held at once.
///
/// A session asks for a handful of sizes — the preview's, the export's — for
/// one country at a time, so this only has to stop the map growing without
/// bound over a long sitting.
const RENDER_CACHE: usize = 24;

pub struct FlagSet {
    /// Lower-case alpha-2 code to the gzipped SVG, which lives in the binary.
    svg: BTreeMap<String, &'static [u8]>,
    /// Rendered on demand and kept, since a caption asks for the same flag at
    /// the same size on every frame it is drawn.
    rendered: Mutex<BTreeMap<(String, u32), std::sync::Arc<RgbaImage>>>,
}

impl FlagSet {
    pub fn embedded() -> &'static FlagSet {
        static SET: OnceLock<FlagSet> = OnceLock::new();
        SET.get_or_init(|| {
            FlagSet::parse(EMBEDDED).expect("embedded flags.bin is corrupt; re-run make pack-flags")
        })
    }

    pub fn parse(bytes: &'static [u8]) -> Result<FlagSet> {
        if bytes.get(..4) != Some(&MAGIC[..]) {
            bail!("bad magic, not a sort4print flag blob");
        }
        let version = *bytes.get(4).context("flag blob truncated")?;
        if version != VERSION {
            bail!("flag blob version {version}, expected {VERSION}");
        }
        let count = u16::from_le_bytes(
            bytes
                .get(5..7)
                .context("flag blob truncated")?
                .try_into()
                .expect("two bytes"),
        );

        let mut svg = BTreeMap::new();
        let mut at = 7;
        for _ in 0..count {
            let code = std::str::from_utf8(bytes.get(at..at + 2).context("flag blob truncated")?)
                .context("flag code is not UTF-8")?
                .to_ascii_lowercase();
            at += 2;
            let length = u32::from_le_bytes(
                bytes
                    .get(at..at + 4)
                    .context("flag blob truncated")?
                    .try_into()
                    .expect("four bytes"),
            ) as usize;
            at += 4;
            let drawing = bytes.get(at..at + length).context("flag blob truncated")?;
            at += length;
            svg.insert(code, drawing);
        }

        Ok(FlagSet {
            svg,
            rendered: Mutex::new(BTreeMap::new()),
        })
    }

    pub fn len(&self) -> usize {
        self.svg.len()
    }

    pub fn is_empty(&self) -> bool {
        self.svg.is_empty()
    }

    pub fn has(&self, country_code: &str) -> bool {
        self.svg.contains_key(&country_code.to_ascii_lowercase())
    }

    /// The flag for an ISO 3166-1 alpha-2 code, drawn `height_px` tall. Its
    /// width follows the flag's own proportions, which run from 3:2 through 2:1
    /// and, for Qatar, 11:28.
    ///
    /// `None` for a code with no flag here, which is not an error: the caption
    /// simply goes without one.
    pub fn image(&self, country_code: &str, height_px: u32) -> Option<std::sync::Arc<RgbaImage>> {
        let code = country_code.to_ascii_lowercase();
        if code.len() != 2 || height_px == 0 {
            return None;
        }
        let height_px = height_px.min(4096);
        let key = (code.clone(), height_px);

        let mut rendered = self.rendered.lock().ok()?;
        if let Some(image) = rendered.get(&key) {
            return Some(image.clone());
        }

        let drawing = self.svg.get(&code)?;
        let image = std::sync::Arc::new(render(drawing, height_px)?);

        // Nothing clever: the working set is one or two sizes of one country.
        if rendered.len() >= RENDER_CACHE {
            rendered.clear();
        }
        rendered.insert(key, image.clone());
        Some(image)
    }

    pub fn codes(&self) -> impl Iterator<Item = &str> {
        self.svg.keys().map(String::as_str)
    }
}

/// Rasterises a gzipped SVG to the given height.
///
/// usvg reads the gzip itself, which is why nothing here needs a decompressor.
fn render(gzipped_svg: &[u8], height_px: u32) -> Option<RgbaImage> {
    use resvg::{tiny_skia, usvg};

    let tree = usvg::Tree::from_data(gzipped_svg, &usvg::Options::default()).ok()?;
    let size = tree.size();
    if size.height() <= 0.0 || size.width() <= 0.0 {
        return None;
    }

    let scale = height_px as f32 / size.height();
    let width_px = ((size.width() * scale).round() as u32).clamp(1, 8192);

    let mut pixmap = tiny_skia::Pixmap::new(width_px, height_px)?;
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );

    // tiny-skia works in premultiplied alpha and the image crate does not, so
    // the pixels have to be divided back out or every flag with a transparent
    // edge would come out darkened.
    let mut out = RgbaImage::new(width_px, height_px);
    for (source, target) in pixmap.pixels().iter().zip(out.pixels_mut()) {
        let straight = source.demultiply();
        *target = image::Rgba([
            straight.red(),
            straight.green(),
            straight.blue(),
            straight.alpha(),
        ]);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_embedded_set_covers_the_world() {
        let flags = FlagSet::embedded();
        assert!(flags.len() > 200, "unexpectedly few flags: {}", flags.len());
        for code in ["fr", "ru", "us", "jp", "fj", "pt"] {
            assert!(flags.has(code), "no flag for {code}");
        }
    }

    #[test]
    fn a_flag_renders_at_the_size_asked_for() {
        let flags = FlagSet::embedded();
        let france = flags.image("FR", 200).expect("France has a flag");
        assert_eq!(france.height(), 200);
        let aspect = france.width() as f64 / france.height() as f64;
        assert!(
            (aspect - 1.5).abs() < 0.05,
            "the French flag is 3:2, got {aspect}"
        );
        // Opaque and not blank.
        assert!(france.pixels().all(|p| p.0[3] > 250));
        assert!(france.pixels().any(|p| p.0[0] > 150), "no red in it");
    }

    /// The point of keeping them as drawings: a big print asks for a big flag
    /// and gets a sharp one, not the same picture stretched.
    #[test]
    fn the_same_flag_renders_at_any_size() {
        let flags = FlagSet::embedded();
        for height in [24, 64, 200, 900] {
            let flag = flags.image("jp", height).expect("Japan has a flag");
            assert_eq!(flag.height(), height, "asked for {height}");
            assert!(flag.width() > 0);
        }
    }

    /// Japan's flag is a red disc on white, so it is the one that proves the
    /// premultiplied pixels are being divided back out: get that wrong and the
    /// white comes out grey.
    #[test]
    fn white_stays_white() {
        let flags = FlagSet::embedded();
        let japan = flags.image("jp", 120).expect("Japan has a flag");
        let corner = japan.get_pixel(2, 2);
        assert!(
            corner.0[0] > 245 && corner.0[1] > 245 && corner.0[2] > 245,
            "the corner should be white, got {:?}",
            corner.0
        );
    }

    #[test]
    fn the_case_of_the_code_does_not_matter() {
        let flags = FlagSet::embedded();
        assert!(flags.image("de", 64).is_some());
        assert!(flags.image("DE", 64).is_some());
        assert!(flags.has("De"));
    }

    #[test]
    fn rendering_is_only_done_once_per_size() {
        let flags = FlagSet::embedded();
        let first = flags.image("it", 77).expect("Italy has a flag");
        let second = flags.image("it", 77).expect("still there");
        assert!(
            std::sync::Arc::ptr_eq(&first, &second),
            "the second call should hand back the same rendering"
        );
        let other = flags.image("it", 78).expect("a different size");
        assert!(
            !std::sync::Arc::ptr_eq(&first, &other),
            "a different size is a different rendering"
        );
    }

    #[test]
    fn an_unknown_code_is_simply_no_flag() {
        let flags = FlagSet::embedded();
        assert!(flags.image("zz", 64).is_none());
        assert!(flags.image("", 64).is_none());
        assert!(flags.image("france", 64).is_none());
        assert!(flags.image("fr", 0).is_none(), "nor is a zero-height flag");
        assert!(!flags.has("zz"));
    }

    #[test]
    fn a_corrupt_blob_is_refused() {
        assert!(FlagSet::parse(b"nope").is_err());
        assert!(FlagSet::parse(b"S4PF\x09\x00\x00").is_err(), "wrong version");
        // A count that promises more than the blob holds.
        assert!(FlagSet::parse(b"S4PF\x02\x10\x00").is_err());
    }

    /// Every country the city database can name should have a flag to go with
    /// it, or the setting would silently do nothing for some photos.
    #[test]
    fn the_countries_in_the_city_database_are_covered() {
        let flags = FlagSet::embedded();
        let cities = crate::geo::CityDb::embedded();

        let missing: Vec<&str> = cities
            .country_codes()
            .filter(|code| !flags.has(code))
            .collect();

        // A handful of territories in GeoNames have no flag of their own; the
        // point of this test is that it is a handful and named, not a surprise.
        assert!(
            missing.len() < 12,
            "too many countries without a flag: {missing:?}"
        );
    }
}
