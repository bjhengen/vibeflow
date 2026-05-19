//! Embedded window icon for the vibeflow terminal.
//!
//! The 256x256 RGBA8 PNG at `crates/vibeflow/assets/icon.png` is embedded
//! at compile time via `include_bytes!` so the published crate is
//! self-contained. Decode failure is non-fatal — `load_icon()` returns
//! `None` and the window is created without an icon (logged at WARN).

use winit::window::Icon;

const ICON_PNG: &[u8] = include_bytes!("../assets/icon.png");

/// Decode the embedded PNG into a winit `Icon`. Returns `None` on any
/// decode/`Icon::from_rgba` failure; failure is non-fatal at startup.
pub fn load_icon() -> Option<Icon> {
    decode_to_icon(ICON_PNG)
}

/// Internal helper, separated so tests can exercise the failure path
/// against arbitrary input without modifying the embedded asset.
fn decode_to_icon(bytes: &[u8]) -> Option<Icon> {
    let decoder = png::Decoder::new(bytes);
    let mut reader = decoder.read_info().ok()?;
    let mut buf = vec![0u8; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).ok()?;
    // Truncate to the actual frame bytes (next_frame may write less than capacity).
    buf.truncate(info.buffer_size());
    // Require RGBA8 — what our committed icon.png is encoded as. Anything
    // else means the asset was regenerated incorrectly; surface as None.
    if info.color_type != png::ColorType::Rgba || info.bit_depth != png::BitDepth::Eight {
        return None;
    }
    Icon::from_rgba(buf, info.width, info.height).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_icon_decodes_to_256x256_rgba() {
        // load_icon() returns Some for the committed crates/vibeflow/assets/icon.png.
        // We verify Some-ness here; the dimensions/format are pinned by the
        // separate decode_to_icon round-trip test below using the same bytes.
        assert!(load_icon().is_some(), "embedded icon failed to decode");
    }

    #[test]
    fn decode_round_trip_reports_256x256_rgba8() {
        // Decode the embedded bytes directly and verify the PNG header reports
        // the expected geometry, independent of winit's Icon validation.
        let decoder = png::Decoder::new(ICON_PNG);
        let reader = decoder.read_info().expect("png header");
        let info = reader.info();
        assert_eq!(info.width, 256);
        assert_eq!(info.height, 256);
        assert_eq!(info.color_type, png::ColorType::Rgba);
        assert_eq!(info.bit_depth, png::BitDepth::Eight);
    }

    #[test]
    fn bad_bytes_returns_none() {
        assert!(decode_to_icon(b"not-a-png-at-all").is_none());
    }

    #[test]
    fn empty_bytes_returns_none() {
        assert!(decode_to_icon(b"").is_none());
    }
}
