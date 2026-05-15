//! Stage 13: theme registry + iTerm2 color-scheme import.
//!
//! User imports `.itermcolors` files via `vibeflow --import-colors <path>`.
//! Themes land in `~/.config/vibeflow/themes/<name>.toml`. At startup,
//! `ThemeRegistry::load` scans the directory. `[colors] preset = "name"`
//! selects the default. Per-tab override via Stage 10's context menu.

pub mod iterm2;
pub mod registry;

use serde::{Deserialize, Serialize};

/// In-memory theme colors (RGB, alpha always opaque).
///
/// NOTE: the `Serialize`/`Deserialize` derives are for potential in-memory
/// use only. The on-disk TOML format is sectioned (`[ansi]` / `[special]`)
/// and does NOT match this flat layout — use [`ThemeData::to_toml`] /
/// [`ThemeData::from_toml`] for file I/O, never `toml::from_str::<ThemeData>`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ThemeData {
    pub name: String,
    pub ansi: [[f32; 4]; 16],
    pub foreground: [f32; 4],
    pub background: [f32; 4],
    pub cursor: [f32; 4],
    pub cursor_text: [f32; 4],
    #[serde(default)]
    pub bold: Option<[f32; 4]>,
    #[serde(default)]
    pub link: Option<[f32; 4]>,
    #[serde(default)]
    pub selection: Option<[f32; 4]>,
}

#[derive(Debug, thiserror::Error)]
pub enum ThemeParseError {
    #[error("invalid TOML: {0}")]
    InvalidToml(#[from] toml::de::Error),
    #[error("invalid hex color in field {field}: {value}")]
    BadHex { field: String, value: String },
}

impl ThemeData {
    /// Stage 13: serialize to the on-disk TOML format.
    pub fn to_toml(&self) -> String {
        fn hex(c: [f32; 4]) -> String {
            format!(
                "#{:02x}{:02x}{:02x}",
                (c[0].clamp(0.0, 1.0) * 255.0) as u8,
                (c[1].clamp(0.0, 1.0) * 255.0) as u8,
                (c[2].clamp(0.0, 1.0) * 255.0) as u8,
            )
        }
        // TOML basic-string escaping for `name` — themes are named from imported
        // file stems (T13), and Linux filenames may contain `"`, `\`, or newlines.
        let escaped_name = self
            .name
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('\n', "\\n")
            .replace('\r', "\\r")
            .replace('\t', "\\t");
        let mut out = format!("name = \"{}\"\n\n[ansi]\n", escaped_name);
        for (i, c) in self.ansi.iter().enumerate() {
            out.push_str(&format!("ansi_{} = \"{}\"\n", i, hex(*c)));
        }
        out.push_str(&format!(
            "\n[special]\nforeground = \"{}\"\n",
            hex(self.foreground)
        ));
        out.push_str(&format!("background = \"{}\"\n", hex(self.background)));
        out.push_str(&format!("cursor = \"{}\"\n", hex(self.cursor)));
        out.push_str(&format!("cursor_text = \"{}\"\n", hex(self.cursor_text)));
        if let Some(b) = self.bold {
            out.push_str(&format!("bold = \"{}\"\n", hex(b)));
        }
        if let Some(l) = self.link {
            out.push_str(&format!("link = \"{}\"\n", hex(l)));
        }
        if let Some(s) = self.selection {
            out.push_str(&format!("selection = \"{}\"\n", hex(s)));
        }
        out
    }

    /// Stage 13: deserialize from on-disk TOML.
    pub fn from_toml(s: &str) -> Result<Self, ThemeParseError> {
        #[derive(Deserialize)]
        struct File {
            name: String,
            ansi: AnsiMap,
            special: SpecialMap,
        }
        #[derive(Deserialize)]
        struct AnsiMap {
            ansi_0: String,
            ansi_1: String,
            ansi_2: String,
            ansi_3: String,
            ansi_4: String,
            ansi_5: String,
            ansi_6: String,
            ansi_7: String,
            ansi_8: String,
            ansi_9: String,
            ansi_10: String,
            ansi_11: String,
            ansi_12: String,
            ansi_13: String,
            ansi_14: String,
            ansi_15: String,
        }
        #[derive(Deserialize)]
        struct SpecialMap {
            foreground: String,
            background: String,
            cursor: String,
            cursor_text: String,
            #[serde(default)]
            bold: Option<String>,
            #[serde(default)]
            link: Option<String>,
            #[serde(default)]
            selection: Option<String>,
        }
        fn parse_hex(field: &str, s: &str) -> Result<[f32; 4], ThemeParseError> {
            let s = s.trim_start_matches('#');
            if s.len() != 6 {
                return Err(ThemeParseError::BadHex {
                    field: field.into(),
                    value: s.into(),
                });
            }
            let r = u8::from_str_radix(&s[0..2], 16).map_err(|_| ThemeParseError::BadHex {
                field: field.into(),
                value: s.into(),
            })?;
            let g = u8::from_str_radix(&s[2..4], 16).map_err(|_| ThemeParseError::BadHex {
                field: field.into(),
                value: s.into(),
            })?;
            let b = u8::from_str_radix(&s[4..6], 16).map_err(|_| ThemeParseError::BadHex {
                field: field.into(),
                value: s.into(),
            })?;
            Ok([r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, 1.0])
        }
        let f: File = toml::from_str(s)?;
        let ansi = [
            parse_hex("ansi_0", &f.ansi.ansi_0)?,
            parse_hex("ansi_1", &f.ansi.ansi_1)?,
            parse_hex("ansi_2", &f.ansi.ansi_2)?,
            parse_hex("ansi_3", &f.ansi.ansi_3)?,
            parse_hex("ansi_4", &f.ansi.ansi_4)?,
            parse_hex("ansi_5", &f.ansi.ansi_5)?,
            parse_hex("ansi_6", &f.ansi.ansi_6)?,
            parse_hex("ansi_7", &f.ansi.ansi_7)?,
            parse_hex("ansi_8", &f.ansi.ansi_8)?,
            parse_hex("ansi_9", &f.ansi.ansi_9)?,
            parse_hex("ansi_10", &f.ansi.ansi_10)?,
            parse_hex("ansi_11", &f.ansi.ansi_11)?,
            parse_hex("ansi_12", &f.ansi.ansi_12)?,
            parse_hex("ansi_13", &f.ansi.ansi_13)?,
            parse_hex("ansi_14", &f.ansi.ansi_14)?,
            parse_hex("ansi_15", &f.ansi.ansi_15)?,
        ];
        Ok(ThemeData {
            name: f.name,
            ansi,
            foreground: parse_hex("foreground", &f.special.foreground)?,
            background: parse_hex("background", &f.special.background)?,
            cursor: parse_hex("cursor", &f.special.cursor)?,
            cursor_text: parse_hex("cursor_text", &f.special.cursor_text)?,
            bold: f
                .special
                .bold
                .as_deref()
                .map(|s| parse_hex("bold", s))
                .transpose()?,
            link: f
                .special
                .link
                .as_deref()
                .map(|s| parse_hex("link", s))
                .transpose()?,
            selection: f
                .special
                .selection
                .as_deref()
                .map(|s| parse_hex("selection", s))
                .transpose()?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_theme() -> ThemeData {
        ThemeData {
            name: "test".into(),
            ansi: [[0.5, 0.5, 0.5, 1.0]; 16],
            foreground: [1.0, 1.0, 1.0, 1.0],
            background: [0.0, 0.0, 0.0, 1.0],
            cursor: [1.0, 0.0, 0.0, 1.0],
            cursor_text: [0.0, 0.0, 0.0, 1.0],
            bold: None,
            link: None,
            selection: None,
        }
    }

    #[test]
    fn to_toml_and_back_round_trips() {
        let t = sample_theme();
        let s = t.to_toml();
        let parsed = ThemeData::from_toml(&s).expect("roundtrip");
        assert_eq!(parsed.name, t.name);
        assert_eq!(parsed.foreground, t.foreground);
        assert_eq!(parsed.background, t.background);
        assert_eq!(
            parsed.foreground[3], 1.0,
            "alpha always reconstructed as 1.0"
        );
        for i in 0..16 {
            for c in 0..3 {
                let diff = (parsed.ansi[i][c] - t.ansi[i][c]).abs();
                assert!(diff < 0.01, "ansi[{i}][{c}] drift {diff}");
            }
        }
    }

    #[test]
    fn to_toml_escapes_name_with_special_chars() {
        let mut t = sample_theme();
        // Name contains a double-quote, a backslash, and a tab — all characters
        // that are invalid unescaped inside a TOML basic string.
        t.name = "weird\"name\\with\ttabs".to_string();
        let s = t.to_toml();
        let parsed = ThemeData::from_toml(&s).expect("escaped name must round-trip");
        assert_eq!(
            parsed.name, t.name,
            "name with special chars must survive round-trip"
        );
    }

    #[test]
    fn from_toml_rejects_invalid_hex() {
        let bad = r##"
name = "bad"
[ansi]
ansi_0 = "not-hex"
ansi_1 = "#000000"
ansi_2 = "#000000"
ansi_3 = "#000000"
ansi_4 = "#000000"
ansi_5 = "#000000"
ansi_6 = "#000000"
ansi_7 = "#000000"
ansi_8 = "#000000"
ansi_9 = "#000000"
ansi_10 = "#000000"
ansi_11 = "#000000"
ansi_12 = "#000000"
ansi_13 = "#000000"
ansi_14 = "#000000"
ansi_15 = "#000000"
[special]
foreground = "#ffffff"
background = "#000000"
cursor = "#000000"
cursor_text = "#000000"
"##;
        let r = ThemeData::from_toml(bad);
        assert!(matches!(r, Err(ThemeParseError::BadHex { .. })));
    }

    #[test]
    fn from_toml_rejects_missing_field() {
        let bad = r##"
name = "incomplete"
[ansi]
ansi_0 = "#000000"
"##;
        let r = ThemeData::from_toml(bad);
        assert!(r.is_err());
    }
}
