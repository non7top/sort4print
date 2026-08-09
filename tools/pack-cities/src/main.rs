//! Converts the GeoNames dumps into the compact blob that gets baked into the
//! exe with `include_bytes!`.
//!
//! Run it through the build container:
//!
//!     ./x pack-cities
//!
//! Input files come from <https://download.geonames.org/export/dump/> and are
//! licensed CC BY 4.0; the attribution lives in the README and the About panel.

use std::collections::HashMap;
use std::io::Write;

use anyhow::{bail, Context, Result};

/// Mirror of `sort4print_core::geo`'s reader. Kept duplicated on purpose: the
/// packer is a build-time tool and must not drag the GUI crate's dependency
/// tree along, so the two ends only agree on this format comment.
///
/// ```text
/// magic   b"S4PC"
/// u8      format version (1)
/// u16     country count
///   [u8;2] ISO 3166-1 alpha-2
///   u8     name length
///   ..     name, UTF-8
/// u32     city count
///   f32    latitude
///   f32    longitude
///   u16    index into the country table
///   u8     name length
///   ..     name, UTF-8
/// ```
/// All integers little-endian.
const VERSION: u8 = 1;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [cities_path, countries_path, out_path] = match args.as_slice() {
        [a, b, c] => [a.clone(), b.clone(), c.clone()],
        _ => bail!("usage: pack-cities <cities15000.txt> <countryInfo.txt> <out.bin>"),
    };

    let countries = read_countries(&countries_path)?;
    let cities = read_cities(&cities_path, &countries)?;
    println!(
        "packing {} cities across {} countries",
        cities.len(),
        countries.len()
    );

    let mut out = Vec::new();
    out.extend_from_slice(b"S4PC");
    out.push(VERSION);

    out.extend_from_slice(&(countries.len() as u16).to_le_bytes());
    for c in &countries {
        out.extend_from_slice(c.code.as_bytes());
        push_str(&mut out, &c.name)?;
    }

    out.extend_from_slice(&(cities.len() as u32).to_le_bytes());
    for c in &cities {
        out.extend_from_slice(&c.lat.to_le_bytes());
        out.extend_from_slice(&c.lon.to_le_bytes());
        out.extend_from_slice(&c.country_idx.to_le_bytes());
        push_str(&mut out, &c.name)?;
    }

    let mut f = std::fs::File::create(&out_path)
        .with_context(|| format!("creating {out_path}"))?;
    f.write_all(&out)?;
    println!("wrote {out_path} ({} KiB)", out.len() / 1024);
    Ok(())
}

fn push_str(out: &mut Vec<u8>, s: &str) -> Result<()> {
    let bytes = s.as_bytes();
    if bytes.len() > u8::MAX as usize {
        bail!("name too long for the format: {s:?}");
    }
    out.push(bytes.len() as u8);
    out.extend_from_slice(bytes);
    Ok(())
}

struct Country {
    code: String,
    name: String,
}

struct City {
    lat: f32,
    lon: f32,
    country_idx: u16,
    name: String,
}

fn read_countries(path: &str) -> Result<Vec<Country>> {
    let text = std::fs::read_to_string(path).with_context(|| format!("reading {path}"))?;
    let mut out = Vec::new();
    for line in text.lines() {
        if line.starts_with('#') || line.trim().is_empty() {
            continue;
        }
        let cols: Vec<&str> = line.split('\t').collect();
        // ISO, ISO3, ISO-Numeric, fips, Country, ...
        let (Some(code), Some(name)) = (cols.first(), cols.get(4)) else {
            continue;
        };
        if code.len() != 2 || name.is_empty() {
            continue;
        }
        out.push(Country {
            code: code.to_string(),
            name: truncate_utf8(name, u8::MAX as usize),
        });
    }
    if out.is_empty() {
        bail!("no countries parsed from {path}");
    }
    Ok(out)
}

fn read_cities(path: &str, countries: &[Country]) -> Result<Vec<City>> {
    let index: HashMap<&str, u16> = countries
        .iter()
        .enumerate()
        .map(|(i, c)| (c.code.as_str(), i as u16))
        .collect();

    let text = std::fs::read_to_string(path).with_context(|| format!("reading {path}"))?;
    let mut out = Vec::new();
    let mut skipped = 0usize;
    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let cols: Vec<&str> = line.split('\t').collect();
        // 1 name, 4 latitude, 5 longitude, 8 country code
        let (Some(name), Some(lat), Some(lon), Some(cc)) =
            (cols.get(1), cols.get(4), cols.get(5), cols.get(8))
        else {
            skipped += 1;
            continue;
        };
        let (Ok(lat), Ok(lon)) = (lat.parse::<f32>(), lon.parse::<f32>()) else {
            skipped += 1;
            continue;
        };
        let Some(&country_idx) = index.get(cc) else {
            skipped += 1;
            continue;
        };
        out.push(City {
            lat,
            lon,
            country_idx,
            name: truncate_utf8(name, u8::MAX as usize),
        });
    }
    if skipped > 0 {
        println!("skipped {skipped} unparseable/unknown-country rows");
    }
    if out.is_empty() {
        bail!("no cities parsed from {path}");
    }
    Ok(out)
}

/// Clip to a byte budget without splitting a UTF-8 sequence.
fn truncate_utf8(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].to_string()
}
