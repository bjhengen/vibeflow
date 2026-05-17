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
    #[error("invalid color value: {0}")]
    BadColorValue(String),
}

pub fn parse_itermcolors(plist_bytes: &[u8]) -> Result<ThemeData, ItermImportError> {
    let value: Value =
        plist::from_bytes(plist_bytes).map_err(|e| ItermImportError::NotAPlist(e.to_string()))?;
    let dict = value.into_dictionary().ok_or(ItermImportError::NotADict)?;

    /// Read one color component (Red/Green/Blue/Alpha) from a color sub-dict.
    /// A *missing* component defaults silently (some valid iTerm2 exports omit
    /// Alpha; rare ones omit a channel). A *present-but-wrong-type* component
    /// is a real corruption signal and surfaces as `BadColorValue` so the
    /// `--import-colors` CLI (T13) can tell the user the file is malformed.
    fn component(
        sub: &plist::Dictionary,
        comp: &str,
        outer_key: &str,
        default: f64,
    ) -> Result<f32, ItermImportError> {
        match sub.get(comp) {
            None => Ok(default as f32),
            Some(v) => v
                .as_real()
                .map(|f| f as f32)
                .ok_or_else(|| ItermImportError::BadColorValue(format!("{outer_key} / {comp}"))),
        }
    }

    fn read_color(dict: &plist::Dictionary, key: &str) -> Result<[f32; 4], ItermImportError> {
        let sub = dict
            .get(key)
            .ok_or_else(|| ItermImportError::MissingKey(key.to_owned()))?
            .as_dictionary()
            .ok_or_else(|| ItermImportError::BadColorValue(key.to_owned()))?;
        let r = component(sub, "Red Component", key, 0.0)?;
        let g = component(sub, "Green Component", key, 0.0)?;
        let b = component(sub, "Blue Component", key, 0.0)?;
        // Alpha read but not semantically used — terminal colors are always
        // opaque; ThemeData.to_toml is RGB-only and from_toml forces alpha=1.0.
        let a = component(sub, "Alpha Component", key, 1.0)?;
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
        name: String::new(), // T13 MUST set this from the file basename before
        // calling to_toml; writing with an empty name is a bug.
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

    #[test]
    fn parse_itermcolors_rejects_wrong_type_color_component() {
        // Red Component is a <string>, not a <real> — corruption signal.
        let xml = br#"<?xml version="1.0"?>
<plist version="1.0"><dict>
<key>Ansi 0 Color</key><dict>
<key>Red Component</key><string>nope</string>
<key>Green Component</key><real>0.0</real>
<key>Blue Component</key><real>0.0</real>
</dict></dict></plist>"#;
        let r = parse_itermcolors(xml);
        assert!(
            matches!(r, Err(ItermImportError::BadColorValue(_))),
            "wrong-type component must surface as BadColorValue, not silent 0.0"
        );
    }
}
