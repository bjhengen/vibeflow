//! Keyboard shortcut dispatch. Single source of truth for the
//! Ctrl+Shift+... and Super+... shortcut table.
//!
//! `match_shortcut` consumes a winit logical key + modifier set and returns
//! `Some(Shortcut)` if the combo matches one of vibeflow's shortcuts;
//! otherwise `None`, in which case the caller should fall through to the
//! ordinary typed-input path (so a literal `T` keystroke still reaches the
//! PTY when no Ctrl+Shift / Super modifier is held).

use winit::keyboard::{Key, ModifiersState, NamedKey};

/// Discrete shortcut actions vibeflow's `window.rs` dispatches. Stage 9 will
/// extend this enum with config-driven entries; for now it's hard-coded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shortcut {
    NewTab,
    CloseTab,
    NextTab,
    PrevTab,
    RestartTab,
    Copy,
    Paste,
}

/// Match a winit key event against the shortcut table. Returns `None` if the
/// modifier combo doesn't match any shortcut — the caller should then fall
/// through to typed-input dispatch.
///
/// Matching rule:
/// * `Ctrl+Shift+Key` is the primary form.
/// * `Super+Key` (no Shift required, except for `Tab` directionals) is an
///   alias — gives Mac users via VNC the muscle-memory of `Cmd+...`.
/// * `Alt` and other non-listed modifiers must be UNSET. Pressing
///   `Ctrl+Shift+Alt+T` is NOT a new-tab shortcut — it falls through.
#[must_use]
pub fn match_shortcut(key: &Key, modifiers: ModifiersState) -> Option<Shortcut> {
    let ctrl = modifiers.control_key();
    let shift = modifiers.shift_key();
    let alt = modifiers.alt_key();
    let supr = modifiers.super_key();

    // Reject any combo with Alt set — we don't bind Alt-anything in Stage 8.
    if alt {
        return None;
    }

    // The two valid modifier shapes for the non-Tab shortcuts:
    //   * Ctrl+Shift  (standard Linux terminal binding)
    //   * Super alone (Mac-via-VNC alias)
    // Tab is the exception: prev-tab needs Shift either way.
    let ctrl_shift = ctrl && shift && !supr;
    let super_only = supr && !ctrl && !shift;
    let super_shift = supr && shift && !ctrl;

    match key {
        Key::Character(c) if ctrl_shift || super_only => match c.as_str() {
            "T" | "t" => Some(Shortcut::NewTab),
            "W" | "w" => Some(Shortcut::CloseTab),
            "R" | "r" => Some(Shortcut::RestartTab),
            "C" | "c" => Some(Shortcut::Copy),
            "V" | "v" => Some(Shortcut::Paste),
            _ => None,
        },
        Key::Named(NamedKey::Tab) => {
            // Ctrl+Tab → next; Ctrl+Shift+Tab → prev.
            // Super+Tab → next; Super+Shift+Tab → prev.
            if ctrl && !shift && !supr {
                Some(Shortcut::NextTab)
            } else if ctrl && shift && !supr {
                Some(Shortcut::PrevTab)
            } else if supr && !shift && !ctrl {
                Some(Shortcut::NextTab)
            } else if super_shift {
                Some(Shortcut::PrevTab)
            } else {
                None
            }
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use winit::keyboard::SmolStr;

    fn ch(s: &str) -> Key {
        Key::Character(SmolStr::new(s))
    }

    fn mods(ctrl: bool, shift: bool, alt: bool, supr: bool) -> ModifiersState {
        let mut m = ModifiersState::empty();
        if ctrl {
            m |= ModifiersState::CONTROL;
        }
        if shift {
            m |= ModifiersState::SHIFT;
        }
        if alt {
            m |= ModifiersState::ALT;
        }
        if supr {
            m |= ModifiersState::SUPER;
        }
        m
    }

    // Ctrl+Shift form

    #[test]
    fn ctrl_shift_t_is_new_tab() {
        assert_eq!(
            match_shortcut(&ch("T"), mods(true, true, false, false)),
            Some(Shortcut::NewTab)
        );
    }

    #[test]
    fn ctrl_shift_lowercase_t_is_new_tab() {
        // winit may deliver lowercase or uppercase depending on Shift state;
        // accept both so layout differences don't bite us.
        assert_eq!(
            match_shortcut(&ch("t"), mods(true, true, false, false)),
            Some(Shortcut::NewTab)
        );
    }

    #[test]
    fn ctrl_shift_w_is_close_tab() {
        assert_eq!(
            match_shortcut(&ch("W"), mods(true, true, false, false)),
            Some(Shortcut::CloseTab)
        );
    }

    #[test]
    fn ctrl_shift_r_is_restart_tab() {
        assert_eq!(
            match_shortcut(&ch("R"), mods(true, true, false, false)),
            Some(Shortcut::RestartTab)
        );
    }

    #[test]
    fn ctrl_shift_c_is_copy() {
        assert_eq!(
            match_shortcut(&ch("C"), mods(true, true, false, false)),
            Some(Shortcut::Copy)
        );
    }

    #[test]
    fn ctrl_shift_v_is_paste() {
        assert_eq!(
            match_shortcut(&ch("V"), mods(true, true, false, false)),
            Some(Shortcut::Paste)
        );
    }

    #[test]
    fn ctrl_tab_is_next_tab() {
        assert_eq!(
            match_shortcut(&Key::Named(NamedKey::Tab), mods(true, false, false, false)),
            Some(Shortcut::NextTab)
        );
    }

    #[test]
    fn ctrl_shift_tab_is_prev_tab() {
        assert_eq!(
            match_shortcut(&Key::Named(NamedKey::Tab), mods(true, true, false, false)),
            Some(Shortcut::PrevTab)
        );
    }

    // Super-alias form

    #[test]
    fn super_t_is_new_tab() {
        assert_eq!(
            match_shortcut(&ch("T"), mods(false, false, false, true)),
            Some(Shortcut::NewTab)
        );
    }

    #[test]
    fn super_v_is_paste() {
        assert_eq!(
            match_shortcut(&ch("V"), mods(false, false, false, true)),
            Some(Shortcut::Paste)
        );
    }

    #[test]
    fn super_tab_is_next_tab() {
        assert_eq!(
            match_shortcut(&Key::Named(NamedKey::Tab), mods(false, false, false, true)),
            Some(Shortcut::NextTab)
        );
    }

    #[test]
    fn super_shift_tab_is_prev_tab() {
        assert_eq!(
            match_shortcut(&Key::Named(NamedKey::Tab), mods(false, true, false, true)),
            Some(Shortcut::PrevTab)
        );
    }

    // Negative cases

    #[test]
    fn plain_t_is_none() {
        assert_eq!(
            match_shortcut(&ch("T"), mods(false, false, false, false)),
            None
        );
    }

    #[test]
    fn ctrl_t_without_shift_is_none() {
        // Ctrl+T alone is not a vibeflow shortcut; bash uses it for transpose-chars.
        assert_eq!(
            match_shortcut(&ch("T"), mods(true, false, false, false)),
            None
        );
    }

    #[test]
    fn ctrl_shift_alt_t_is_none() {
        // Alt set → reject. Don't false-positive on Ctrl+Shift+Alt+T.
        assert_eq!(
            match_shortcut(&ch("T"), mods(true, true, true, false)),
            None
        );
    }

    #[test]
    fn ctrl_shift_x_is_none() {
        // Unbound character with the right modifiers still returns None.
        assert_eq!(
            match_shortcut(&ch("X"), mods(true, true, false, false)),
            None
        );
    }

    #[test]
    fn super_with_ctrl_is_none() {
        // Super+Ctrl combo is ambiguous; reject to keep the table boring.
        assert_eq!(
            match_shortcut(&ch("T"), mods(true, false, false, true)),
            None
        );
    }
}
