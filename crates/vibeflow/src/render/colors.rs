//! ANSI / 256-indexed / truecolor → RGB resolution. Pure logic.
//!
//! `alacritty_terminal::vte::ansi::Color` carries three variants. To produce
//! GPU-ready RGB we need a default ANSI palette (the Term struct doesn't fill
//! one — it only stores OSC-4 overrides). [`resolve_color`] handles all three
//! variants against the default palette + any overrides.

use alacritty_terminal::term::color::Colors;
use alacritty_terminal::vte::ansi::{Color, NamedColor, Rgb};

/// 256-entry default ANSI / 256-color palette. Slots 0..=15 are the classic
/// ANSI 16 (xterm defaults), 16..=231 are the 6×6×6 color cube, 232..=255 are
/// the 24-step grayscale ramp.
///
/// This is the fallback used when [`alacritty_terminal::term::color::Colors`]
/// has no override set for a slot. Most apps don't emit OSC 4 palette
/// overrides, so this fallback covers the common case.
#[must_use]
pub fn default_palette() -> [Rgb; 256] {
    let mut palette = [Rgb { r: 0, g: 0, b: 0 }; 256];

    // ANSI 0..=7 (xterm normal-intensity defaults).
    palette[0] = Rgb {
        r: 0x00,
        g: 0x00,
        b: 0x00,
    };
    palette[1] = Rgb {
        r: 0xcd,
        g: 0x00,
        b: 0x00,
    };
    palette[2] = Rgb {
        r: 0x00,
        g: 0xcd,
        b: 0x00,
    };
    palette[3] = Rgb {
        r: 0xcd,
        g: 0xcd,
        b: 0x00,
    };
    palette[4] = Rgb {
        r: 0x00,
        g: 0x00,
        b: 0xee,
    };
    palette[5] = Rgb {
        r: 0xcd,
        g: 0x00,
        b: 0xcd,
    };
    palette[6] = Rgb {
        r: 0x00,
        g: 0xcd,
        b: 0xcd,
    };
    palette[7] = Rgb {
        r: 0xe5,
        g: 0xe5,
        b: 0xe5,
    };
    // ANSI 8..=15 (xterm bright defaults).
    palette[8] = Rgb {
        r: 0x7f,
        g: 0x7f,
        b: 0x7f,
    };
    palette[9] = Rgb {
        r: 0xff,
        g: 0x00,
        b: 0x00,
    };
    palette[10] = Rgb {
        r: 0x00,
        g: 0xff,
        b: 0x00,
    };
    palette[11] = Rgb {
        r: 0xff,
        g: 0xff,
        b: 0x00,
    };
    palette[12] = Rgb {
        r: 0x5c,
        g: 0x5c,
        b: 0xff,
    };
    palette[13] = Rgb {
        r: 0xff,
        g: 0x00,
        b: 0xff,
    };
    palette[14] = Rgb {
        r: 0x00,
        g: 0xff,
        b: 0xff,
    };
    palette[15] = Rgb {
        r: 0xff,
        g: 0xff,
        b: 0xff,
    };

    // 6×6×6 color cube (indices 16..=231). xterm uses {0, 95, 135, 175, 215, 255}
    // for each of the six steps per channel.
    const CUBE_STEPS: [u8; 6] = [0, 95, 135, 175, 215, 255];
    let mut idx = 16usize;
    #[allow(clippy::needless_range_loop)]
    for r in 0..6 {
        for g in 0..6 {
            for b in 0..6 {
                palette[idx] = Rgb {
                    r: CUBE_STEPS[r],
                    g: CUBE_STEPS[g],
                    b: CUBE_STEPS[b],
                };
                idx += 1;
            }
        }
    }
    debug_assert_eq!(idx, 232);

    // Grayscale ramp (indices 232..=255). xterm uses 8 + 10*i for the i-th step.
    for i in 0..24 {
        let v = 8 + 10 * i;
        palette[232 + i as usize] = Rgb { r: v, g: v, b: v };
    }

    palette
}

/// Resolve an alacritty `Color` to a concrete RGB triple, using the override
/// table if it has the requested slot set, falling back to the built-in
/// [`default_palette`] for `Indexed` / `Named` color values, and to the
/// caller-supplied `fg_default` / `bg_default` for the special
/// `NamedColor::Foreground` / `NamedColor::Background` semantic slots.
///
/// `Spec(rgb)` is passed through unchanged.
#[must_use]
pub fn resolve_color(color: Color, colors: &Colors, fg_default: Rgb, bg_default: Rgb) -> Rgb {
    match color {
        Color::Spec(rgb) => rgb,
        Color::Indexed(idx) => colors[idx as usize].unwrap_or(default_palette()[idx as usize]),
        Color::Named(named) => named_color_to_rgb(named, colors, fg_default, bg_default),
    }
}

fn named_color_to_rgb(named: NamedColor, colors: &Colors, fg_default: Rgb, bg_default: Rgb) -> Rgb {
    if let Some(rgb) = colors[named] {
        return rgb;
    }
    // `NamedColor` discriminants are NOT all in the 0..=15 ANSI range — Dim*
    // variants live at 259..=266 and Foreground/Background/Cursor at 256..=258.
    // We must explicitly handle each non-ANSI variant before falling through
    // to the default-palette index, otherwise we'd panic at runtime when an
    // app uses dim-coloured text (e.g. `ls`'s SGR dim attribute on errors).
    match named {
        NamedColor::Foreground | NamedColor::DimForeground | NamedColor::BrightForeground => {
            fg_default
        }
        NamedColor::Background => bg_default,
        // Cursor + selection-bg/fg also live in the special slot range; treat
        // them as transparent fallbacks via the fg/bg defaults for now. Stage 6
        // adds proper handling.
        NamedColor::Cursor => fg_default,
        // Dim variants — map to the corresponding normal ANSI index in the
        // default palette. Stage 7 may darken them further (75% of normal).
        NamedColor::DimBlack => default_palette()[0],
        NamedColor::DimRed => default_palette()[1],
        NamedColor::DimGreen => default_palette()[2],
        NamedColor::DimYellow => default_palette()[3],
        NamedColor::DimBlue => default_palette()[4],
        NamedColor::DimMagenta => default_palette()[5],
        NamedColor::DimCyan => default_palette()[6],
        NamedColor::DimWhite => default_palette()[7],
        // Everything else is in the 0..=15 ANSI range (Black through
        // BrightWhite). `NamedColor`'s repr is the palette index for those, so
        // we can index the default palette directly.
        other => default_palette()[other as usize],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_colors() -> Colors {
        // Default `Colors::default()` has every slot None.
        Colors::default()
    }

    #[test]
    fn default_palette_first_eight_match_xterm_basics() {
        // The classic ANSI 0..=7 palette, xterm defaults.
        assert_eq!(
            default_palette()[0],
            Rgb {
                r: 0x00,
                g: 0x00,
                b: 0x00
            }
        ); // black
        assert_eq!(
            default_palette()[1],
            Rgb {
                r: 0xcd,
                g: 0x00,
                b: 0x00
            }
        ); // red
        assert_eq!(
            default_palette()[2],
            Rgb {
                r: 0x00,
                g: 0xcd,
                b: 0x00
            }
        ); // green
        assert_eq!(
            default_palette()[3],
            Rgb {
                r: 0xcd,
                g: 0xcd,
                b: 0x00
            }
        ); // yellow
        assert_eq!(
            default_palette()[4],
            Rgb {
                r: 0x00,
                g: 0x00,
                b: 0xee
            }
        ); // blue
        assert_eq!(
            default_palette()[5],
            Rgb {
                r: 0xcd,
                g: 0x00,
                b: 0xcd
            }
        ); // magenta
        assert_eq!(
            default_palette()[6],
            Rgb {
                r: 0x00,
                g: 0xcd,
                b: 0xcd
            }
        ); // cyan
        assert_eq!(
            default_palette()[7],
            Rgb {
                r: 0xe5,
                g: 0xe5,
                b: 0xe5
            }
        ); // white
    }

    #[test]
    fn default_palette_bright_colors_are_brighter() {
        // Bright variants are at indices 8..=15.
        for i in 0..8 {
            let normal = default_palette()[i];
            let bright = default_palette()[i + 8];
            // Sum of channels is monotonically nondecreasing for the bright variant.
            let normal_sum = normal.r as u32 + normal.g as u32 + normal.b as u32;
            let bright_sum = bright.r as u32 + bright.g as u32 + bright.b as u32;
            assert!(
                bright_sum >= normal_sum,
                "bright[{}] ({bright_sum}) should be ≥ normal[{}] ({normal_sum})",
                i + 8,
                i
            );
        }
    }

    #[test]
    fn default_palette_color_cube_at_index_16_is_pure_black() {
        // The 6×6×6 color cube starts at index 16. (16, 0, 0, 0).
        assert_eq!(default_palette()[16], Rgb { r: 0, g: 0, b: 0 });
    }

    #[test]
    fn default_palette_grayscale_ramp_starts_dark_ends_light() {
        // The grayscale ramp occupies 232..=255.
        let dark = default_palette()[232];
        let light = default_palette()[255];
        assert!(dark.r < 20, "expected near-black at 232, got {dark:?}");
        assert!(light.r > 220, "expected near-white at 255, got {light:?}");
        // Ramp is monotonically nondecreasing.
        for i in 232..255 {
            let lo = default_palette()[i].r;
            let hi = default_palette()[i + 1].r;
            assert!(hi >= lo, "ramp not monotonic at {i}: {lo} > {hi}");
        }
    }

    #[test]
    fn resolve_color_spec_passes_rgb_unchanged() {
        let rgb = Rgb {
            r: 0x12,
            g: 0x34,
            b: 0x56,
        };
        let resolved = resolve_color(
            Color::Spec(rgb),
            &empty_colors(),
            Rgb {
                r: 0xff,
                g: 0xff,
                b: 0xff,
            }, // fg fallback
            Rgb {
                r: 0x00,
                g: 0x00,
                b: 0x00,
            }, // bg fallback
        );
        assert_eq!(resolved, rgb);
    }

    #[test]
    fn resolve_color_indexed_uses_default_palette_when_overrides_empty() {
        // Index 1 is red in the default palette.
        let resolved = resolve_color(
            Color::Indexed(1),
            &empty_colors(),
            Rgb {
                r: 0xff,
                g: 0xff,
                b: 0xff,
            },
            Rgb {
                r: 0x00,
                g: 0x00,
                b: 0x00,
            },
        );
        assert_eq!(
            resolved,
            Rgb {
                r: 0xcd,
                g: 0x00,
                b: 0x00
            }
        );
    }

    #[test]
    fn resolve_color_indexed_prefers_override_when_set() {
        let mut colors = Colors::default();
        colors[1usize] = Some(Rgb {
            r: 0xab,
            g: 0xcd,
            b: 0xef,
        });
        let resolved = resolve_color(
            Color::Indexed(1),
            &colors,
            Rgb {
                r: 0xff,
                g: 0xff,
                b: 0xff,
            },
            Rgb {
                r: 0x00,
                g: 0x00,
                b: 0x00,
            },
        );
        assert_eq!(
            resolved,
            Rgb {
                r: 0xab,
                g: 0xcd,
                b: 0xef
            }
        );
    }

    #[test]
    fn resolve_color_named_foreground_uses_fg_fallback_when_unset() {
        let fg = Rgb {
            r: 0xee,
            g: 0xee,
            b: 0xee,
        };
        let resolved = resolve_color(
            Color::Named(NamedColor::Foreground),
            &empty_colors(),
            fg,
            Rgb {
                r: 0x00,
                g: 0x00,
                b: 0x00,
            },
        );
        assert_eq!(resolved, fg);
    }

    #[test]
    fn resolve_color_named_background_uses_bg_fallback_when_unset() {
        let bg = Rgb {
            r: 0x0e,
            g: 0x0e,
            b: 0x12,
        };
        let resolved = resolve_color(
            Color::Named(NamedColor::Background),
            &empty_colors(),
            Rgb {
                r: 0xff,
                g: 0xff,
                b: 0xff,
            },
            bg,
        );
        assert_eq!(resolved, bg);
    }

    #[test]
    fn resolve_color_named_red_uses_default_palette_when_unset() {
        // NamedColor::Red is index 1 in the ANSI palette.
        let resolved = resolve_color(
            Color::Named(NamedColor::Red),
            &empty_colors(),
            Rgb {
                r: 0xff,
                g: 0xff,
                b: 0xff,
            },
            Rgb {
                r: 0x00,
                g: 0x00,
                b: 0x00,
            },
        );
        assert_eq!(
            resolved,
            Rgb {
                r: 0xcd,
                g: 0x00,
                b: 0x00
            }
        );
    }

    #[test]
    fn resolve_color_dim_red_does_not_panic() {
        // `NamedColor::DimRed` has discriminant 260, which is past the end of
        // the 256-entry default palette. The match arms must explicitly map
        // each Dim* variant before the catch-all `other => default_palette[idx]`.
        let resolved = resolve_color(
            Color::Named(NamedColor::DimRed),
            &empty_colors(),
            Rgb {
                r: 0xff,
                g: 0xff,
                b: 0xff,
            },
            Rgb {
                r: 0x00,
                g: 0x00,
                b: 0x00,
            },
        );
        // We map DimRed to the same RGB as normal Red in the default palette.
        assert_eq!(
            resolved,
            Rgb {
                r: 0xcd,
                g: 0x00,
                b: 0x00
            }
        );
    }

    #[test]
    fn resolve_color_dim_white_does_not_panic() {
        let resolved = resolve_color(
            Color::Named(NamedColor::DimWhite),
            &empty_colors(),
            Rgb {
                r: 0xff,
                g: 0xff,
                b: 0xff,
            },
            Rgb {
                r: 0x00,
                g: 0x00,
                b: 0x00,
            },
        );
        assert_eq!(
            resolved,
            Rgb {
                r: 0xe5,
                g: 0xe5,
                b: 0xe5
            }
        );
    }
}
