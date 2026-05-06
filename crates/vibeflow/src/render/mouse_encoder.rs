//! Encodes mouse events into SGR (preferred) or X10 (legacy) escape
//! sequences for terminal mouse-mode passthrough into vim/htop/tmux/etc.
//!
//! All functions take 0-indexed grid coordinates from
//! `alacritty_terminal::index::Point`; SGR/X10 wire format expects
//! 1-indexed coordinates, so the encoders +1 internally.
//!
//! # SGR format (preferred — `TermMode::SGR_MOUSE` set)
//! `\x1b[<{button};{col};{row}M` for press / drag (capital M)
//! `\x1b[<{button};{col};{row}m` for release (lowercase m)
//!
//! Button codes:
//! * 0 = left, 1 = middle, 2 = right
//! * +32 (bit 5) = motion-while-pressed (drag)
//!
//! # X10 format (legacy — `SGR_MOUSE` clear)
//! `\x1b[M{btn+32}{col+32}{row+32}` — three raw bytes after the `M`. No
//! release distinction in pure X10; the modes that do drag tracking layer
//! it on, but at that point apps usually negotiate SGR.

use alacritty_terminal::index::Point;

/// Mouse button identifier. Matches `winit::event::MouseButton`'s three
/// canonical buttons; we don't pass through Other(_) buttons.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    Left,
    Middle,
    Right,
}

impl Button {
    fn code(self) -> u32 {
        match self {
            Button::Left => 0,
            Button::Middle => 1,
            Button::Right => 2,
        }
    }
}

/// Encode a press event. SGR uses the `M` terminator; X10 has no release
/// distinction so this is the same byte sequence as a release would be.
#[must_use]
pub fn encode_press(button: Button, point: Point, sgr: bool) -> Vec<u8> {
    if sgr {
        sgr_format(button.code(), point, b'M')
    } else {
        x10_format(button.code(), point)
    }
}

/// Encode a release event. SGR uses the `m` terminator (lowercase); X10
/// uses button code 3 (release marker).
#[must_use]
pub fn encode_release(button: Button, point: Point, sgr: bool) -> Vec<u8> {
    if sgr {
        sgr_format(button.code(), point, b'm')
    } else {
        x10_format(3, point) // X10's release sentinel
    }
}

/// Encode a drag (motion-while-pressed) event. Adds the +32 motion bit.
#[must_use]
pub fn encode_drag(button: Button, point: Point, sgr: bool) -> Vec<u8> {
    let code = button.code() + 32;
    if sgr {
        sgr_format(code, point, b'M')
    } else {
        x10_format(code, point)
    }
}

fn sgr_format(code: u32, point: Point, terminator: u8) -> Vec<u8> {
    // 1-indexed coordinates per the SGR spec.
    let col = (point.column.0 as u32) + 1;
    let row = (point.line.0 as u32) + 1;
    let body = format!("\x1b[<{code};{col};{row}");
    let mut out = body.into_bytes();
    out.push(terminator);
    out
}

fn x10_format(code: u32, point: Point) -> Vec<u8> {
    // 1-indexed + 32 offset. Caps at 255-32 = 223 cols/rows in pure X10.
    // For terminals beyond 223 the spec is ambiguous; SGR is the modern
    // workaround. We saturating-add for safety.
    let col_byte = (((point.column.0 as u32) + 1).saturating_add(32)).min(255) as u8;
    let row_byte = (((point.line.0 as u32) + 1).saturating_add(32)).min(255) as u8;
    let code_byte = (code.saturating_add(32)).min(255) as u8;
    vec![0x1b, b'[', b'M', code_byte, col_byte, row_byte]
}

#[cfg(test)]
mod tests {
    use super::*;
    use alacritty_terminal::index::{Column, Line};

    fn pt(line: i32, col: usize) -> Point {
        Point::new(Line(line), Column(col))
    }

    // SGR tests — verbatim byte sequences against the spec.

    #[test]
    fn sgr_left_press_at_origin() {
        // (line 0, col 0) → 1-indexed → "1;1"
        assert_eq!(
            encode_press(Button::Left, pt(0, 0), true),
            b"\x1b[<0;1;1M".to_vec()
        );
    }

    #[test]
    fn sgr_left_release_uses_lowercase_m() {
        assert_eq!(
            encode_release(Button::Left, pt(5, 10), true),
            b"\x1b[<0;11;6m".to_vec()
        );
    }

    #[test]
    fn sgr_left_drag_adds_motion_bit() {
        // 0 (left) + 32 (motion) = 32
        assert_eq!(
            encode_drag(Button::Left, pt(0, 0), true),
            b"\x1b[<32;1;1M".to_vec()
        );
    }

    #[test]
    fn sgr_middle_button_code_is_1() {
        assert_eq!(
            encode_press(Button::Middle, pt(0, 0), true),
            b"\x1b[<1;1;1M".to_vec()
        );
    }

    #[test]
    fn sgr_right_button_code_is_2() {
        assert_eq!(
            encode_press(Button::Right, pt(0, 0), true),
            b"\x1b[<2;1;1M".to_vec()
        );
    }

    // X10 tests

    #[test]
    fn x10_left_press_at_origin() {
        // \x1b [ M (button+32) (col+1+32) (row+1+32)
        // button=0+32=32, col=1+32=33, row=1+32=33
        assert_eq!(
            encode_press(Button::Left, pt(0, 0), false),
            vec![0x1b, b'[', b'M', 32, 33, 33]
        );
    }

    #[test]
    fn x10_release_uses_button_code_3() {
        // X10 release uses the special button code 3 (35 after +32 offset).
        assert_eq!(
            encode_release(Button::Left, pt(0, 0), false),
            vec![0x1b, b'[', b'M', 35, 33, 33]
        );
    }

    #[test]
    fn x10_oversized_grid_saturates() {
        // Grid coord 250 → 1+250+32 = 283 → saturates to 255 in u8.
        let bytes = encode_press(Button::Left, pt(250, 250), false);
        assert_eq!(bytes[0..3], [0x1b, b'[', b'M']);
        assert_eq!(bytes[4], 255); // col saturates
        assert_eq!(bytes[5], 255); // row saturates
    }
}
