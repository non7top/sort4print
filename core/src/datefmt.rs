//! Date rendering for the caption.
//!
//! Deliberately not ICU: the whole point of this tool is a small self-contained
//! exe, and a full CLDR bundle would dwarf everything else in it. English and
//! Russian are compiled in; any other locale can be declared in the ini file
//! (see `config::Config`), which is also how you override the built-ins.
//!
//! Russian needs two long forms, because "October 2025" and "5 October 2025"
//! decline differently — `Октябрь 2025` but `5 октября 2025`. Hence the
//! separate `months_long_of` ("of October") table, which for English is simply
//! a copy of `months_long`.

use std::collections::BTreeMap;

/// A date as it came off the camera. Deliberately a plain record rather than a
/// chrono type: nothing here needs time zones, arithmetic or a calendar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PhotoDate {
    pub year: i32,
    /// 1-12.
    pub month: u32,
    /// 1-31.
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
}

impl PhotoDate {
    /// Parses the EXIF form `YYYY:MM:DD HH:MM:SS`. Also tolerates `-`/`/`
    /// separators and a missing time, which some phone software writes.
    pub fn parse_exif(s: &str) -> Option<PhotoDate> {
        let s = s.trim();
        let (date, time) = match s.split_once([' ', 'T']) {
            Some((d, t)) => (d, Some(t)),
            None => (s, None),
        };
        let mut parts = date.split(['-', ':', '/', '.']);
        let year: i32 = parts.next()?.parse().ok()?;
        let month: u32 = parts.next()?.parse().ok()?;
        let day: u32 = parts.next()?.parse().ok()?;
        if !(1..=12).contains(&month) || !(1..=31).contains(&day) || year <= 0 {
            return None;
        }

        let (mut hour, mut minute) = (0, 0);
        if let Some(t) = time {
            let mut tp = t.split(':');
            hour = tp.next().and_then(|v| v.trim().parse().ok()).unwrap_or(0);
            minute = tp.next().and_then(|v| v.parse().ok()).unwrap_or(0);
            if hour > 23 || minute > 59 {
                (hour, minute) = (0, 0);
            }
        }

        Some(PhotoDate {
            year,
            month,
            day,
            hour,
            minute,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Locale {
    pub id: String,
    /// Display name shown in the picker.
    pub label: String,
    pub months_short: [String; 12],
    pub months_long: [String; 12],
    /// The form used when a day number precedes the month ("5 октября").
    pub months_long_of: [String; 12],
}

impl Locale {
    fn from_lists(
        id: &str,
        label: &str,
        short: [&str; 12],
        long: [&str; 12],
        long_of: [&str; 12],
    ) -> Locale {
        Locale {
            id: id.to_string(),
            label: label.to_string(),
            months_short: short.map(str::to_string),
            months_long: long.map(str::to_string),
            months_long_of: long_of.map(str::to_string),
        }
    }
}

/// Locales available to the caption, built-ins merged with anything the ini
/// declares. An ini entry with the same id replaces the built-in outright.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Locales {
    map: BTreeMap<String, Locale>,
}

impl Default for Locales {
    fn default() -> Self {
        let mut map = BTreeMap::new();
        for l in builtin_locales() {
            map.insert(l.id.clone(), l);
        }
        Locales { map }
    }
}

impl Locales {
    pub fn insert(&mut self, locale: Locale) {
        self.map.insert(locale.id.clone(), locale);
    }

    pub fn get(&self, id: &str) -> Option<&Locale> {
        self.map.get(id)
    }

    /// Falls back to English so a typo'd locale id in the ini degrades to a
    /// readable caption instead of an empty one.
    pub fn get_or_default(&self, id: &str) -> &Locale {
        self.map
            .get(id)
            .or_else(|| self.map.get("en"))
            .or_else(|| self.map.values().next())
            .expect("at least one locale is always compiled in")
    }

    pub fn ids(&self) -> impl Iterator<Item = &str> {
        self.map.keys().map(String::as_str)
    }

    pub fn iter(&self) -> impl Iterator<Item = &Locale> {
        self.map.values()
    }
}

pub fn builtin_locales() -> Vec<Locale> {
    vec![
        Locale::from_lists(
            "en",
            "English",
            [
                "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
            ],
            [
                "January",
                "February",
                "March",
                "April",
                "May",
                "June",
                "July",
                "August",
                "September",
                "October",
                "November",
                "December",
            ],
            [
                "January",
                "February",
                "March",
                "April",
                "May",
                "June",
                "July",
                "August",
                "September",
                "October",
                "November",
                "December",
            ],
        ),
        Locale::from_lists(
            "ru",
            "Русский",
            [
                "янв", "фев", "мар", "апр", "май", "июн", "июл", "авг", "сен", "окт", "ноя", "дек",
            ],
            [
                "Январь",
                "Февраль",
                "Март",
                "Апрель",
                "Май",
                "Июнь",
                "Июль",
                "Август",
                "Сентябрь",
                "Октябрь",
                "Ноябрь",
                "Декабрь",
            ],
            [
                "января",
                "февраля",
                "марта",
                "апреля",
                "мая",
                "июня",
                "июля",
                "августа",
                "сентября",
                "октября",
                "ноября",
                "декабря",
            ],
        ),
    ]
}

/// Format presets offered in the date panel. The pattern is what actually gets
/// stored in the ini, so a preset is just a convenient way to type one.
pub const PRESETS: &[(&str, &str)] = &[
    ("Oct '25", "{MMM} '{yy}"),
    ("October 2025", "{MMMM} {yyyy}"),
    ("5 October 2025", "{d} {MMMMo} {yyyy}"),
    ("05.10.2025", "{dd}.{MM}.{yyyy}"),
    ("2025-10-05", "{yyyy}-{MM}-{dd}"),
    ("10/2025", "{MM}/{yyyy}"),
    ("2025", "{yyyy}"),
];

/// The token list, shown under the custom-pattern box.
pub const TOKEN_HINT: &str = "\
{yyyy} 2025   {yy} 25   {MMMM} October   {MMM} Oct   {MM} 10   {M} 10
{MMMMo} the form after a day number (октября)   {dd} 05   {d} 5
{HH} 09   {mm} 07";

/// Expands `{...}` tokens against a date and locale.
///
/// | token      | meaning                          | example |
/// |------------|----------------------------------|---------|
/// | `{yyyy}`   | four-digit year                  | 2025    |
/// | `{yy}`     | two-digit year                   | 25      |
/// | `{MMMM}`   | month, long, standalone          | October |
/// | `{MMMMo}`  | month, long, after a day number  | октября |
/// | `{MMM}`    | month, short                     | Oct     |
/// | `{MM}`     | month, zero-padded               | 10      |
/// | `{M}`      | month                            | 10      |
/// | `{dd}`     | day, zero-padded                 | 05      |
/// | `{d}`      | day                              | 5       |
/// | `{HH}`     | hour, zero-padded, 24h           | 09      |
/// | `{mm}`     | minute, zero-padded              | 07      |
///
/// Unknown tokens are left verbatim so a typo is visible in the preview rather
/// than silently swallowed. `{{` and `}}` are literal braces.
pub fn format_date(pattern: &str, date: PhotoDate, locale: &Locale) -> String {
    let mi = (date.month.clamp(1, 12) - 1) as usize;
    let mut out = String::with_capacity(pattern.len() + 8);
    let mut rest = pattern;

    while let Some(open) = rest.find(['{', '}']) {
        out.push_str(&rest[..open]);
        let tail = &rest[open..];

        if let Some(stripped) = tail.strip_prefix("{{") {
            out.push('{');
            rest = stripped;
            continue;
        }
        if let Some(stripped) = tail.strip_prefix("}}") {
            out.push('}');
            rest = stripped;
            continue;
        }
        if tail.starts_with('}') {
            // Stray closer: emit it and move on.
            out.push('}');
            rest = &tail[1..];
            continue;
        }

        let Some(close) = tail.find('}') else {
            // Unterminated token: the rest is literal.
            out.push_str(tail);
            return out;
        };
        let token = &tail[1..close];
        match token {
            "yyyy" => out.push_str(&format!("{:04}", date.year)),
            "yy" => out.push_str(&format!("{:02}", date.year.rem_euclid(100))),
            "MMMM" => out.push_str(&locale.months_long[mi]),
            "MMMMo" => out.push_str(&locale.months_long_of[mi]),
            "MMM" => out.push_str(&locale.months_short[mi]),
            "MM" => out.push_str(&format!("{:02}", date.month)),
            "M" => out.push_str(&date.month.to_string()),
            "dd" => out.push_str(&format!("{:02}", date.day)),
            "d" => out.push_str(&date.day.to_string()),
            "HH" => out.push_str(&format!("{:02}", date.hour)),
            "mm" => out.push_str(&format!("{:02}", date.minute)),
            _ => out.push_str(&tail[..=close]),
        }
        rest = &tail[close + 1..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn oct5() -> PhotoDate {
        PhotoDate {
            year: 2025,
            month: 10,
            day: 5,
            hour: 9,
            minute: 7,
        }
    }

    #[test]
    fn parses_exif_timestamps() {
        let d = PhotoDate::parse_exif("2025:10:05 09:07:31").unwrap();
        assert_eq!(d, oct5());
        assert_eq!(
            PhotoDate::parse_exif("2025-10-05").unwrap(),
            PhotoDate {
                hour: 0,
                minute: 0,
                ..oct5()
            }
        );
        assert!(PhotoDate::parse_exif("0000:00:00 00:00:00").is_none());
        assert!(PhotoDate::parse_exif("garbage").is_none());
    }

    #[test]
    fn the_requested_default_format() {
        let l = Locales::default();
        assert_eq!(format_date("{MMM} '{yy}", oct5(), l.get("en").unwrap()), "Oct '25");
    }

    #[test]
    fn russian_declines_the_month_correctly() {
        let l = Locales::default();
        let ru = l.get("ru").unwrap();
        assert_eq!(format_date("{MMMM} {yyyy}", oct5(), ru), "Октябрь 2025");
        assert_eq!(format_date("{d} {MMMMo} {yyyy}", oct5(), ru), "5 октября 2025");
        assert_eq!(format_date("{MMM} '{yy}", oct5(), ru), "окт '25");
    }

    #[test]
    fn every_preset_renders_in_every_builtin_locale() {
        let locales = Locales::default();
        for locale in locales.iter() {
            for (_, pattern) in PRESETS {
                let s = format_date(pattern, oct5(), locale);
                assert!(!s.is_empty());
                assert!(!s.contains('{'), "{pattern} left a token behind: {s}");
            }
        }
    }

    #[test]
    fn unknown_tokens_survive_so_typos_are_visible() {
        let l = Locales::default();
        let en = l.get("en").unwrap();
        assert_eq!(format_date("{nope}", oct5(), en), "{nope}");
        assert_eq!(format_date("{MMM", oct5(), en), "{MMM");
        assert_eq!(format_date("{{{MMM}}}", oct5(), en), "{Oct}");
    }

    #[test]
    fn padding_and_zero_padding() {
        let l = Locales::default();
        let en = l.get("en").unwrap();
        assert_eq!(format_date("{dd}.{MM}.{yyyy} {HH}:{mm}", oct5(), en), "05.10.2025 09:07");
        assert_eq!(format_date("{d}/{M}", oct5(), en), "5/10");
    }

    #[test]
    fn unknown_locale_falls_back_to_english() {
        let l = Locales::default();
        assert_eq!(l.get_or_default("zz").id, "en");
    }
}
