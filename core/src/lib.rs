//! Everything sort4print does that is not drawing widgets.
//!
//! Kept apart from the GUI crate for one practical reason: it builds and tests
//! natively on Linux with no windowing system, so the crop maths, EXIF parsing,
//! geocoding, caption layout and export pipeline are all covered by tests that
//! run in the same disposable container that cross-compiles the Windows exe.

pub mod config;
pub mod cropbox;
pub mod datefmt;
pub mod exif_data;
pub mod export;
pub mod fonts;
pub mod geo;
pub mod ini;
pub mod loader;
pub mod sidecar;
pub mod stamp;

pub use config::{AspectRatio, CaptionConfig, Color, Config, Corner, NavMode};
pub use cropbox::{Constraints, CropBox, Handle};
pub use sidecar::{PhotoState, Sidecar};
pub use datefmt::{format_date, Locale, Locales, PhotoDate};
pub use exif_data::{Orientation, PhotoMeta};
pub use fonts::{FontCatalog, FontFace};
pub use geo::{CityDb, Place};
pub use loader::Preview;

pub const APP_NAME: &str = "sort4print";
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Shown in the About panel. The city database is a derived work and its
/// licence requires the credit to travel with the program.
pub const ATTRIBUTION: &str = "\
City and country data derived from GeoNames (geonames.org), \
licensed CC BY 4.0.";
