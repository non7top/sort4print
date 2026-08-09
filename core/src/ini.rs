//! A very small INI reader/writer.
//!
//! No dependency, because the format we need is tiny and predictable: sections
//! in `[brackets]`, `key = value`, `#` or `;` comments, UTF-8 (BOM tolerated).
//! Values are taken verbatim after the first `=`, so a caption template may
//! contain `#`, `;`, quotes and commas without any escaping.
//!
//! Writing regenerates the file from the current settings together with the
//! explanatory comments defined in `config`; hand-written comments in an
//! existing file are therefore not preserved across a save.

use std::fmt::Write as _;

#[derive(Debug, Default, Clone)]
pub struct Ini {
    /// Sections in file order; the leading unnamed section has key "".
    sections: Vec<(String, Vec<(String, String)>)>,
}

impl Ini {
    pub fn new() -> Ini {
        Ini::default()
    }

    pub fn parse(text: &str) -> Ini {
        let mut ini = Ini::new();
        let mut current = String::new();
        for raw in text.strip_prefix('\u{feff}').unwrap_or(text).lines() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
                continue;
            }
            if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
                current = name.trim().to_string();
                ini.ensure_section(&current);
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            ini.set(&current, key.trim(), value.trim());
        }
        ini
    }

    fn ensure_section(&mut self, section: &str) -> &mut Vec<(String, String)> {
        if let Some(idx) = self.sections.iter().position(|(n, _)| n == section) {
            return &mut self.sections[idx].1;
        }
        self.sections.push((section.to_string(), Vec::new()));
        &mut self.sections.last_mut().unwrap().1
    }

    pub fn set(&mut self, section: &str, key: &str, value: &str) {
        let entries = self.ensure_section(section);
        match entries.iter_mut().find(|(k, _)| k == key) {
            Some(entry) => entry.1 = value.to_string(),
            None => entries.push((key.to_string(), value.to_string())),
        }
    }

    pub fn get(&self, section: &str, key: &str) -> Option<&str> {
        self.sections
            .iter()
            .find(|(n, _)| n == section)?
            .1
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    pub fn section_names(&self) -> impl Iterator<Item = &str> {
        self.sections.iter().map(|(n, _)| n.as_str())
    }

    pub fn has_section(&self, section: &str) -> bool {
        self.sections.iter().any(|(n, _)| n == section)
    }

    /// `get` with a fallback, for the many settings that have a sane default.
    pub fn get_or<'a>(&'a self, section: &str, key: &str, default: &'a str) -> &'a str {
        self.get(section, key).unwrap_or(default)
    }

    pub fn get_parsed<T: std::str::FromStr>(&self, section: &str, key: &str) -> Option<T> {
        self.get(section, key)?.parse().ok()
    }

    /// Accepts the spellings people actually type in a config file.
    pub fn get_bool(&self, section: &str, key: &str, default: bool) -> bool {
        match self.get(section, key) {
            Some(v) => matches!(
                v.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            ),
            None => default,
        }
    }

    pub fn to_string_with_comments(&self, comments: &dyn Fn(&str, &str) -> Option<String>) -> String {
        let mut out = String::new();
        for (name, entries) in &self.sections {
            if !name.is_empty() {
                if !out.is_empty() {
                    out.push('\n');
                }
                let _ = writeln!(out, "[{name}]");
            }
            for (key, value) in entries {
                if let Some(comment) = comments(name, key) {
                    for line in comment.lines() {
                        let _ = writeln!(out, "# {line}");
                    }
                }
                let _ = writeln!(out, "{key} = {value}");
            }
        }
        out
    }
}

impl std::fmt::Display for Ini {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.to_string_with_comments(&|_, _| None))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_sections_keys_and_comments() {
        let ini = Ini::parse(
            "\u{feff}# leading comment\n\
             [caption]\n\
             ; another comment\n\
             template = {city}, {country}, {date}\n\
             enabled = yes\n\
             \n\
             [date]\n\
             locale=ru\n",
        );
        assert_eq!(
            ini.get("caption", "template"),
            Some("{city}, {country}, {date}")
        );
        assert!(ini.get_bool("caption", "enabled", false));
        assert_eq!(ini.get("date", "locale"), Some("ru"));
        assert_eq!(ini.get("date", "missing"), None);
    }

    #[test]
    fn values_keep_comment_characters() {
        let ini = Ini::parse("[caption]\ntemplate = a # b ; c\n");
        assert_eq!(ini.get("caption", "template"), Some("a # b ; c"));
    }

    #[test]
    fn set_overwrites_in_place_and_round_trips() {
        let mut ini = Ini::parse("[a]\nx = 1\ny = 2\n");
        ini.set("a", "x", "9");
        ini.set("b", "z", "3");
        let text = ini.to_string();
        let reparsed = Ini::parse(&text);
        assert_eq!(reparsed.get("a", "x"), Some("9"));
        assert_eq!(reparsed.get("a", "y"), Some("2"));
        assert_eq!(reparsed.get("b", "z"), Some("3"));
    }

    #[test]
    fn junk_lines_are_ignored_rather_than_fatal() {
        let ini = Ini::parse("garbage\n[s]\nno_equals_here\nk = v\n");
        assert_eq!(ini.get("s", "k"), Some("v"));
    }

    #[test]
    fn comments_are_emitted_when_writing() {
        let mut ini = Ini::new();
        ini.set("date", "locale", "en");
        let text = ini.to_string_with_comments(&|s, k| {
            (s == "date" && k == "locale").then(|| "which month names to use".to_string())
        });
        assert!(text.contains("# which month names to use"));
        assert!(text.contains("locale = en"));
    }
}
