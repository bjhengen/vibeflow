//! Keyboard shortcut dispatch. Stage 8 hard-coded the modifier+key → action
//! match; Stage 9 makes the table data-driven via `ShortcutTable`, populated
//! from `Config.shortcuts`. The default table reproduces Stage 8's bindings
//! exactly so behavior without a config file is unchanged.

use std::collections::HashMap;

use winit::keyboard::{Key, ModifiersState, NamedKey};

/// Discrete shortcut actions vibeflow's `window.rs` dispatches.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Shortcut {
    NewTab,
    CloseTab,
    NextTab,
    PrevTab,
    RestartTab,
    Copy,
    Paste,
    /// Stage 9: open the inline rename input on the active tab.
    RenameTab,
}

/// Keyed lookup table. Constructed via `ShortcutTable::default()` for the
/// built-in bindings, or from a `Config.shortcuts` via
/// `ShortcutTable::from_bindings`.
#[derive(Debug, Clone, Default)]
pub struct ShortcutTable {
    /// (modifiers, key-discriminant) -> action. Multiple chord entries can
    /// map to the same action.
    by_chord: HashMap<ChordKey, Shortcut>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct ChordKey {
    modifiers_bits: u32,
    key: ChordKeyDisc,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum ChordKeyDisc {
    /// ASCII lowercase letter.
    Char(char),
    Tab,
    Function(u8),
}

impl ShortcutTable {
    /// Default Stage 8 bindings — what users get without a config file.
    #[must_use]
    pub fn with_default_bindings() -> Self {
        let pairs: &[(Shortcut, &[(ModifiersState, ChordKeyDisc)])] = &[
            (
                Shortcut::NewTab,
                &[
                    (
                        ModifiersState::CONTROL.union(ModifiersState::SHIFT),
                        ChordKeyDisc::Char('t'),
                    ),
                    (ModifiersState::SUPER, ChordKeyDisc::Char('t')),
                ],
            ),
            (
                Shortcut::CloseTab,
                &[
                    (
                        ModifiersState::CONTROL.union(ModifiersState::SHIFT),
                        ChordKeyDisc::Char('w'),
                    ),
                    (ModifiersState::SUPER, ChordKeyDisc::Char('w')),
                ],
            ),
            (
                Shortcut::NextTab,
                &[
                    (ModifiersState::CONTROL, ChordKeyDisc::Tab),
                    (ModifiersState::SUPER, ChordKeyDisc::Tab),
                ],
            ),
            (
                Shortcut::PrevTab,
                &[
                    (
                        ModifiersState::CONTROL.union(ModifiersState::SHIFT),
                        ChordKeyDisc::Tab,
                    ),
                    (
                        ModifiersState::SUPER.union(ModifiersState::SHIFT),
                        ChordKeyDisc::Tab,
                    ),
                ],
            ),
            (
                Shortcut::RestartTab,
                &[
                    (
                        ModifiersState::CONTROL.union(ModifiersState::SHIFT),
                        ChordKeyDisc::Char('r'),
                    ),
                    (ModifiersState::SUPER, ChordKeyDisc::Char('r')),
                ],
            ),
            (
                Shortcut::Copy,
                &[
                    (
                        ModifiersState::CONTROL.union(ModifiersState::SHIFT),
                        ChordKeyDisc::Char('c'),
                    ),
                    (ModifiersState::SUPER, ChordKeyDisc::Char('c')),
                ],
            ),
            (
                Shortcut::Paste,
                &[
                    (
                        ModifiersState::CONTROL.union(ModifiersState::SHIFT),
                        ChordKeyDisc::Char('v'),
                    ),
                    (ModifiersState::SUPER, ChordKeyDisc::Char('v')),
                ],
            ),
            (
                Shortcut::RenameTab,
                &[
                    (
                        ModifiersState::CONTROL.union(ModifiersState::SHIFT),
                        ChordKeyDisc::Char('e'),
                    ),
                    (ModifiersState::empty(), ChordKeyDisc::Function(2)),
                ],
            ),
        ];
        let mut by_chord = HashMap::new();
        for (action, chords) in pairs {
            for (mods, key) in *chords {
                by_chord.insert(
                    ChordKey {
                        modifiers_bits: mods.bits(),
                        key: *key,
                    },
                    *action,
                );
            }
        }
        Self { by_chord }
    }

    /// Lookup the action triggered by a winit key + modifier set, or `None`
    /// if no chord matches. Reject any combo with `Alt` set — vibeflow does
    /// not bind any `Alt+...` chord.
    #[must_use]
    pub fn lookup(&self, key: &Key, modifiers: ModifiersState) -> Option<Shortcut> {
        if modifiers.alt_key() {
            return None;
        }
        let disc = match key {
            Key::Character(c) => {
                let s = c.as_str();
                let mut chars = s.chars();
                let first = chars.next()?;
                if chars.next().is_some() || !first.is_ascii() {
                    return None;
                }
                ChordKeyDisc::Char(first.to_ascii_lowercase())
            }
            Key::Named(NamedKey::Tab) => ChordKeyDisc::Tab,
            Key::Named(NamedKey::F1) => ChordKeyDisc::Function(1),
            Key::Named(NamedKey::F2) => ChordKeyDisc::Function(2),
            Key::Named(NamedKey::F3) => ChordKeyDisc::Function(3),
            Key::Named(NamedKey::F4) => ChordKeyDisc::Function(4),
            Key::Named(NamedKey::F5) => ChordKeyDisc::Function(5),
            Key::Named(NamedKey::F6) => ChordKeyDisc::Function(6),
            Key::Named(NamedKey::F7) => ChordKeyDisc::Function(7),
            Key::Named(NamedKey::F8) => ChordKeyDisc::Function(8),
            Key::Named(NamedKey::F9) => ChordKeyDisc::Function(9),
            Key::Named(NamedKey::F10) => ChordKeyDisc::Function(10),
            Key::Named(NamedKey::F11) => ChordKeyDisc::Function(11),
            Key::Named(NamedKey::F12) => ChordKeyDisc::Function(12),
            _ => return None,
        };
        // Strip Alt (already rejected above) but keep Ctrl/Shift/Super.
        let mods_bits = (modifiers
            & (ModifiersState::CONTROL | ModifiersState::SHIFT | ModifiersState::SUPER))
            .bits();
        self.by_chord
            .get(&ChordKey {
                modifiers_bits: mods_bits,
                key: disc,
            })
            .copied()
    }
}

/// Backward-compat free function for callers that haven't been migrated to
/// `ShortcutTable::lookup` yet. Uses the default bindings.
#[must_use]
pub fn match_shortcut(key: &Key, modifiers: ModifiersState) -> Option<Shortcut> {
    static DEFAULT: std::sync::OnceLock<ShortcutTable> = std::sync::OnceLock::new();
    DEFAULT
        .get_or_init(ShortcutTable::with_default_bindings)
        .lookup(key, modifiers)
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

    // ===== Existing 17 tests (preserved verbatim from Stage 8) =====

    #[test]
    fn ctrl_shift_t_is_new_tab() {
        assert_eq!(
            match_shortcut(&ch("T"), mods(true, true, false, false)),
            Some(Shortcut::NewTab)
        );
    }

    #[test]
    fn ctrl_shift_lowercase_t_is_new_tab() {
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

    #[test]
    fn plain_t_is_none() {
        assert_eq!(
            match_shortcut(&ch("T"), mods(false, false, false, false)),
            None
        );
    }

    #[test]
    fn ctrl_t_without_shift_is_none() {
        assert_eq!(
            match_shortcut(&ch("T"), mods(true, false, false, false)),
            None
        );
    }

    #[test]
    fn ctrl_shift_alt_t_is_none() {
        assert_eq!(
            match_shortcut(&ch("T"), mods(true, true, true, false)),
            None
        );
    }

    #[test]
    fn ctrl_shift_x_is_none() {
        assert_eq!(
            match_shortcut(&ch("X"), mods(true, true, false, false)),
            None
        );
    }

    #[test]
    fn super_with_ctrl_is_none() {
        assert_eq!(
            match_shortcut(&ch("T"), mods(true, false, false, true)),
            None
        );
    }

    // ===== New Stage 9 tests =====

    #[test]
    fn ctrl_shift_e_is_rename_tab() {
        assert_eq!(
            match_shortcut(&ch("e"), mods(true, true, false, false)),
            Some(Shortcut::RenameTab)
        );
    }

    #[test]
    fn f2_is_rename_tab() {
        assert_eq!(
            match_shortcut(&Key::Named(NamedKey::F2), mods(false, false, false, false)),
            Some(Shortcut::RenameTab)
        );
    }

    #[test]
    fn f1_is_none_by_default() {
        assert_eq!(
            match_shortcut(&Key::Named(NamedKey::F1), mods(false, false, false, false)),
            None
        );
    }

    #[test]
    fn shortcut_table_default_has_all_actions() {
        let t = ShortcutTable::with_default_bindings();
        // 8 distinct actions × 2 chord aliases = 16 entries.
        assert_eq!(t.by_chord.len(), 16);
    }

    #[test]
    fn shortcut_table_lookup_strips_alt_aware() {
        let t = ShortcutTable::with_default_bindings();
        let alt_only = ModifiersState::ALT;
        // Alt set => None always.
        assert_eq!(
            t.lookup(
                &ch("t"),
                ModifiersState::CONTROL | ModifiersState::SHIFT | alt_only
            ),
            None
        );
    }
}
