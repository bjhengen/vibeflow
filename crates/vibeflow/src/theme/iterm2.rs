//! Stage 13: iTerm2 .itermcolors plist parser.

use crate::theme::ThemeData;
use plist::Value;

#[derive(Debug, thiserror::Error)]
pub enum ItermImportError {
    #[error("not a valid plist: {0}")]
    NotAPlist(String),
    #[error("plist is not a dictionary")]
    NotADict,
    #[error("missing required key: {0}")]
    MissingKey(String),
    #[error("invalid color value for key {0}")]
    BadColorValue(String),
}

pub fn parse_itermcolors(plist_bytes: &[u8]) -> Result<ThemeData, ItermImportError> {
    let value: Value =
        plist::from_bytes(plist_bytes).map_err(|e| ItermImportError::NotAPlist(e.to_string()))?;
    let dict = value.into_dictionary().ok_or(ItermImportError::NotADict)?;

    fn read_color(dict: &plist::Dictionary, key: &str) -> Result<[f32; 4], ItermImportError> {
        let sub = dict
            .get(key)
            .ok_or_else(|| ItermImportError::MissingKey(key.to_owned()))?
            .as_dictionary()
            .ok_or_else(|| ItermImportError::BadColorValue(key.to_owned()))?;
        let r = sub
            .get("Red Component")
            .and_then(|v| v.as_real())
            .unwrap_or(0.0) as f32;
        let g = sub
            .get("Green Component")
            .and_then(|v| v.as_real())
            .unwrap_or(0.0) as f32;
        let b = sub
            .get("Blue Component")
            .and_then(|v| v.as_real())
            .unwrap_or(0.0) as f32;
        // Alpha read but not semantically used — terminal colors are always
        // opaque; ThemeData.to_toml is RGB-only and from_toml forces alpha=1.0.
        let a = sub
            .get("Alpha Component")
            .and_then(|v| v.as_real())
            .unwrap_or(1.0) as f32;
        Ok([r, g, b, a])
    }

    let mut ansi = [[0.0_f32; 4]; 16];
    for (i, slot) in ansi.iter_mut().enumerate() {
        *slot = read_color(&dict, &format!("Ansi {i} Color"))?;
    }
    let foreground = read_color(&dict, "Foreground Color")?;
    let background = read_color(&dict, "Background Color")?;
    let cursor = read_color(&dict, "Cursor Color")?;
    let cursor_text = read_color(&dict, "Cursor Text Color")?;
    let bold = read_color(&dict, "Bold Color").ok();
    let link = read_color(&dict, "Link Color").ok();
    let selection = read_color(&dict, "Selection Color").ok();

    Ok(ThemeData {
        name: String::new(), // caller sets from filename basename (T13)
        ansi,
        foreground,
        background,
        cursor,
        cursor_text,
        bold,
        link,
        selection,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &[u8] = include_bytes!("../../tests/fixtures/sample.itermcolors");

    #[test]
    fn parse_itermcolors_round_trips_solarized_dark() {
        let t = parse_itermcolors(SAMPLE).expect("parse");
        // ansi_0 = (0,0,0)
        assert!((t.ansi[0][0] - 0.0).abs() < 0.01);
        assert!((t.ansi[0][1] - 0.0).abs() < 0.01);
        assert!((t.ansi[0][2] - 0.0).abs() < 0.01);
        // ansi_1 (red) ~ (0.86, 0.19, 0.18)
        assert!((t.ansi[1][0] - 0.86).abs() < 0.02);
        assert!((t.ansi[1][1] - 0.19).abs() < 0.02);
        // Foreground ~ (0.51, 0.58, 0.58)
        assert!((t.foreground[0] - 0.51).abs() < 0.02);
        // bold/link/selection absent in fixture -> None
        assert!(t.bold.is_none());
        assert!(t.link.is_none());
        assert!(t.selection.is_none());
        // name left empty for caller (T13) to fill
        assert!(t.name.is_empty());
    }

    #[test]
    fn parse_itermcolors_rejects_not_a_plist() {
        let r = parse_itermcolors(b"this is not a plist");
        assert!(matches!(r, Err(ItermImportError::NotAPlist(_))));
    }

    #[test]
    fn parse_itermcolors_rejects_missing_required_key() {
        let xml = br#"<?xml version="1.0"?>
<plist version="1.0"><dict>
<key>Background Color</key><dict><key>Red Component</key><real>0.0</real></dict>
</dict></plist>"#;
        let r = parse_itermcolors(xml);
        assert!(matches!(r, Err(ItermImportError::MissingKey(_))));
    }
}
