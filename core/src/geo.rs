//! Offline reverse geocoding.
//!
//! GPS coordinates from EXIF are matched against a GeoNames extract (every
//! settlement above ~15 000 inhabitants) baked into the binary. That is a
//! deliberate accuracy trade: a photo taken in a village resolves to the
//! nearest sizeable town, which is usually what you want on a print anyway, and
//! the UI always lets you override the pick. `nearest_n` feeds that override
//! list, `search` feeds free-text lookup for photos with no GPS at all.

use std::sync::OnceLock;

use anyhow::{bail, Context, Result};

/// The packed blob produced by `tools/pack-cities`. See that crate for the
/// format description; the two must be changed together.
static EMBEDDED: &[u8] = include_bytes!("../../assets/cities.bin");

const MAGIC: &[u8; 4] = b"S4PC";
const VERSION: u8 = 1;

/// Mean Earth radius, kilometres.
const EARTH_RADIUS_KM: f64 = 6371.0088;

#[derive(Debug, Clone, PartialEq)]
pub struct Place {
    pub city: String,
    pub country: String,
    pub country_code: String,
    /// Great-circle distance from the queried point, kilometres.
    pub distance_km: f64,
}

struct CityRec {
    lat: f32,
    lon: f32,
    country_idx: u16,
    name: String,
}

pub struct CityDb {
    /// (alpha-2 code, display name)
    countries: Vec<(String, String)>,
    cities: Vec<CityRec>,
}

impl CityDb {
    /// The database compiled into the executable.
    pub fn embedded() -> &'static CityDb {
        static DB: OnceLock<CityDb> = OnceLock::new();
        DB.get_or_init(|| {
            CityDb::parse(EMBEDDED).expect("embedded cities.bin is corrupt; re-run ./x pack-cities")
        })
    }

    pub fn parse(bytes: &[u8]) -> Result<CityDb> {
        let mut r = Reader::new(bytes);
        if r.take(4)? != MAGIC {
            bail!("bad magic, not a sort4print city blob");
        }
        let version = r.u8()?;
        if version != VERSION {
            bail!("city blob version {version}, expected {VERSION}");
        }

        let country_count = r.u16()? as usize;
        let mut countries = Vec::with_capacity(country_count);
        for _ in 0..country_count {
            let code = std::str::from_utf8(r.take(2)?)
                .context("country code is not UTF-8")?
                .to_string();
            countries.push((code, r.string()?));
        }

        let city_count = r.u32()? as usize;
        let mut cities = Vec::with_capacity(city_count);
        for _ in 0..city_count {
            let lat = r.f32()?;
            let lon = r.f32()?;
            let country_idx = r.u16()?;
            if country_idx as usize >= countries.len() {
                bail!("city references country {country_idx} outside the table");
            }
            cities.push(CityRec {
                lat,
                lon,
                country_idx,
                name: r.string()?,
            });
        }

        Ok(CityDb { countries, cities })
    }

    pub fn len(&self) -> usize {
        self.cities.len()
    }

    pub fn is_empty(&self) -> bool {
        self.cities.is_empty()
    }

    /// Closest settlement to the given point, or `None` on an empty database.
    pub fn nearest(&self, lat: f64, lon: f64) -> Option<Place> {
        let mut best: Option<(f64, usize)> = None;
        for (i, c) in self.cities.iter().enumerate() {
            let d = cheap_sq_distance(lat, lon, c.lat as f64, c.lon as f64);
            if best.is_none_or(|(bd, _)| d < bd) {
                best = Some((d, i));
            }
        }
        best.map(|(_, i)| self.place(i, lat, lon))
    }

    /// The `n` closest settlements, nearest first. Used for the override list.
    pub fn nearest_n(&self, lat: f64, lon: f64, n: usize) -> Vec<Place> {
        if n == 0 {
            return Vec::new();
        }
        // n is a handful, so an insertion-sorted top-n beats sorting 34k rows.
        let mut top: Vec<(f64, usize)> = Vec::with_capacity(n + 1);
        for (i, c) in self.cities.iter().enumerate() {
            let d = cheap_sq_distance(lat, lon, c.lat as f64, c.lon as f64);
            if top.len() == n && d >= top[top.len() - 1].0 {
                continue;
            }
            let pos = top.partition_point(|(bd, _)| *bd < d);
            top.insert(pos, (d, i));
            top.truncate(n);
        }
        top.into_iter().map(|(_, i)| self.place(i, lat, lon)).collect()
    }

    /// Case-insensitive substring search over city names, prefix matches first.
    /// `distance_km` is meaningless here and reported as 0.
    pub fn search(&self, query: &str, limit: usize) -> Vec<Place> {
        let q = query.trim().to_lowercase();
        if q.is_empty() || limit == 0 {
            return Vec::new();
        }
        let mut prefix = Vec::new();
        let mut contains = Vec::new();
        for (i, c) in self.cities.iter().enumerate() {
            let name = c.name.to_lowercase();
            if name.starts_with(&q) {
                prefix.push(i);
            } else if name.contains(&q) {
                contains.push(i);
            }
            if prefix.len() >= limit {
                break;
            }
        }
        prefix
            .into_iter()
            .chain(contains)
            .take(limit)
            .map(|i| {
                let c = &self.cities[i];
                self.place(i, c.lat as f64, c.lon as f64)
            })
            .collect()
    }

    fn place(&self, i: usize, from_lat: f64, from_lon: f64) -> Place {
        let c = &self.cities[i];
        let (code, country) = &self.countries[c.country_idx as usize];
        Place {
            city: c.name.clone(),
            country: country.clone(),
            country_code: code.clone(),
            distance_km: haversine_km(from_lat, from_lon, c.lat as f64, c.lon as f64),
        }
    }
}

/// Monotonic stand-in for distance: equirectangular projection, squared, no
/// square root and no trig per candidate beyond one cosine. Only the ordering
/// matters while scanning, and the winner's real distance is computed once with
/// `haversine_km`.
fn cheap_sq_distance(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    let dlat = lat2 - lat1;
    let mut dlon = lon2 - lon1;
    // Wrap the antimeridian so a photo at +179° does not score Alaska as far away.
    if dlon > 180.0 {
        dlon -= 360.0;
    } else if dlon < -180.0 {
        dlon += 360.0;
    }
    let x = dlon * lat1.to_radians().cos();
    x * x + dlat * dlat
}

pub fn haversine_km(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    let (p1, p2) = (lat1.to_radians(), lat2.to_radians());
    let dp = (lat2 - lat1).to_radians();
    let dl = (lon2 - lon1).to_radians();
    let a = (dp / 2.0).sin().powi(2) + p1.cos() * p2.cos() * (dl / 2.0).sin().powi(2);
    2.0 * EARTH_RADIUS_KM * a.sqrt().asin()
}

struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Reader { bytes, pos: 0 }
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self.pos.checked_add(n).context("city blob offset overflow")?;
        let slice = self
            .bytes
            .get(self.pos..end)
            .context("city blob truncated")?;
        self.pos = end;
        Ok(slice)
    }

    fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }

    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }

    fn f32(&mut self) -> Result<f32> {
        Ok(f32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }

    fn string(&mut self) -> Result<String> {
        let len = self.u8()? as usize;
        Ok(std::str::from_utf8(self.take(len)?)
            .context("string in city blob is not UTF-8")?
            .to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_db_loads() {
        let db = CityDb::embedded();
        assert!(db.len() > 20_000, "unexpectedly small database: {}", db.len());
    }

    #[test]
    fn finds_well_known_cities() {
        let db = CityDb::embedded();
        // Red Square.
        let p = db.nearest(55.7539, 37.6208).unwrap();
        assert_eq!(p.city, "Moscow");
        assert_eq!(p.country_code, "RU");
        assert!(p.distance_km < 15.0, "{} km off", p.distance_km);

        // Eiffel Tower.
        let p = db.nearest(48.8584, 2.2945).unwrap();
        assert_eq!(p.country_code, "FR");
        assert!(p.distance_km < 15.0);
    }

    #[test]
    fn nearest_n_is_ordered_and_bounded() {
        let db = CityDb::embedded();
        let list = db.nearest_n(48.8584, 2.2945, 5);
        assert_eq!(list.len(), 5);
        for w in list.windows(2) {
            assert!(w[0].distance_km <= w[1].distance_km);
        }
    }

    #[test]
    fn antimeridian_does_not_confuse_the_scan() {
        let db = CityDb::embedded();
        // Suva, Fiji sits at 178.4E; a point just east of the date line must
        // not be dragged to the other hemisphere by the longitude delta.
        let p = db.nearest(-18.1416, 178.4419).unwrap();
        assert_eq!(p.country_code, "FJ");
    }

    #[test]
    fn search_prefers_prefix_matches() {
        let db = CityDb::embedded();
        let hits = db.search("Berl", 5);
        assert!(!hits.is_empty());
        assert!(hits[0].city.to_lowercase().starts_with("berl"));
    }

    #[test]
    fn rejects_a_corrupt_blob() {
        assert!(CityDb::parse(b"nope").is_err());
        assert!(CityDb::parse(b"S4PC\x02").is_err());
    }

    #[test]
    fn haversine_matches_known_distance() {
        // Moscow -> Paris is about 2486 km.
        let d = haversine_km(55.7558, 37.6173, 48.8566, 2.3522);
        assert!((d - 2486.0).abs() < 25.0, "got {d}");
    }
}
