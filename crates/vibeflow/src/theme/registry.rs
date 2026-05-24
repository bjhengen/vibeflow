//! Stage 13: theme registry — scans `~/.config/vibeflow/themes/`.

use crate::theme::ThemeData;
use std::collections::BTreeMap;
use std::path::PathBuf;

pub struct ThemeRegistry {
    themes: BTreeMap<String, ThemeData>,
    themes_dir: PathBuf,
}

impl ThemeRegistry {
    /// A registry with no themes and no backing directory. Intended as a
    /// placeholder for tests and default-init contexts. Calling [`Self::reload`]
    /// on it safely no-ops (an empty `PathBuf` makes `read_dir` fail and yields
    /// an empty registry — no panic).
    pub fn new_empty() -> Self {
        Self {
            themes: BTreeMap::new(),
            themes_dir: PathBuf::new(),
        }
    }

    /// Scan `themes_dir` for `*.toml` files. Invalid entries logged at warn.
    pub fn load(themes_dir: PathBuf) -> Self {
        let mut themes = BTreeMap::new();
        let Ok(entries) = std::fs::read_dir(&themes_dir) else {
            tracing::debug!(
                "themes dir not found at {}; registry will be empty",
                themes_dir.display()
            );
            return Self { themes, themes_dir };
        };
        const MAX_THEMES_AT_STARTUP: usize = 50;
        let mut entry_paths: Vec<std::path::PathBuf> = entries
            .filter_map(Result::ok)
            .filter_map(|e| {
                let p = e.path();
                if p.extension().and_then(|s| s.to_str()) == Some("toml") {
                    Some(p)
                } else {
                    None
                }
            })
            .collect();
        entry_paths.sort();
        if entry_paths.len() > MAX_THEMES_AT_STARTUP {
            let dropped: Vec<String> = entry_paths[MAX_THEMES_AT_STARTUP..]
                .iter()
                .filter_map(|p| p.file_name().and_then(|s| s.to_str()).map(str::to_owned))
                .collect();
            tracing::warn!(
                cap = MAX_THEMES_AT_STARTUP,
                dropped_count = dropped.len(),
                dropped_names = ?dropped,
                "config: theme directory has more themes than cap; dropping excess (alphabetical)"
            );
            entry_paths.truncate(MAX_THEMES_AT_STARTUP);
        }
        for path in entry_paths {
            let contents = match std::fs::read_to_string(&path) {
                Ok(s) => s,
                Err(e) => {
                    tracing::warn!("cannot read theme {}: {e}", path.display());
                    continue;
                }
            };
            let theme = match ThemeData::from_toml(&contents) {
                Ok(t) => t,
                Err(e) => {
                    tracing::warn!("cannot parse theme {}: {e}", path.display());
                    continue;
                }
            };
            // INVARIANT: themes are keyed by the `name` field *inside* the parsed
            // TOML, not by filename. T13's `--import-colors` writes `<basename>.toml`
            // with `name = <basename>`, so for CLI-imported themes the two always
            // match. A hand-edited file whose internal `name` differs from its
            // filename registers under the internal name; duplicate internal names
            // collide (last parsed wins — BTreeMap iteration order is deterministic).
            themes.insert(theme.name.clone(), theme);
        }
        Self { themes, themes_dir }
    }

    pub fn get(&self, name: &str) -> Option<&ThemeData> {
        self.themes.get(name)
    }

    /// Theme names in sorted order (BTreeMap iteration).
    pub fn names(&self) -> Vec<String> {
        self.themes.keys().cloned().collect()
    }

    pub fn reload(&mut self) {
        *self = Self::load(self.themes_dir.clone());
    }

    pub fn themes_dir(&self) -> &std::path::Path {
        &self.themes_dir
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_valid_theme(dir: &std::path::Path, name: &str) {
        let data = ThemeData {
            name: name.into(),
            ansi: [[0.5; 4]; 16],
            foreground: [1.0; 4],
            background: [0.0; 4],
            cursor: [0.5, 0.5, 0.5, 1.0],
            cursor_text: [0.0; 4],
            bold: None,
            link: None,
            selection: None,
        };
        std::fs::write(dir.join(format!("{name}.toml")), data.to_toml()).expect("write");
    }

    #[test]
    fn load_empty_dir_returns_empty_registry() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let reg = ThemeRegistry::load(tmp.path().to_path_buf());
        assert!(reg.names().is_empty());
    }

    #[test]
    fn load_picks_up_valid_themes() {
        let tmp = tempfile::tempdir().expect("tempdir");
        write_valid_theme(tmp.path(), "alpha");
        write_valid_theme(tmp.path(), "beta");
        let reg = ThemeRegistry::load(tmp.path().to_path_buf());
        assert_eq!(reg.names(), vec!["alpha".to_string(), "beta".to_string()]);
        assert!(reg.get("alpha").is_some());
        assert!(reg.get("missing").is_none());
    }

    #[test]
    fn load_skips_malformed_themes() {
        let tmp = tempfile::tempdir().expect("tempdir");
        write_valid_theme(tmp.path(), "good");
        std::fs::write(tmp.path().join("bad.toml"), "this is not valid toml }}}}").unwrap();
        let reg = ThemeRegistry::load(tmp.path().to_path_buf());
        assert_eq!(reg.names(), vec!["good".to_string()]);
    }

    #[test]
    fn load_ignores_non_toml_files() {
        let tmp = tempfile::tempdir().expect("tempdir");
        write_valid_theme(tmp.path(), "good");
        std::fs::write(tmp.path().join("README.md"), "not a theme").unwrap();
        std::fs::write(tmp.path().join("random.txt"), "also not a theme").unwrap();
        let reg = ThemeRegistry::load(tmp.path().to_path_buf());
        assert_eq!(reg.names(), vec!["good".to_string()]);
    }

    #[test]
    fn registry_caps_themes_at_50_at_startup() {
        let temp = tempfile::tempdir().expect("tempdir");
        let dir = temp.path();
        // Create 60 VALID stub themes via the existing `write_valid_theme` helper.
        for i in 1..=60 {
            let name = format!("theme_{i:03}");
            write_valid_theme(dir, &name);
        }
        let registry = ThemeRegistry::load(dir.to_path_buf());
        assert!(
            registry.names().len() <= 50,
            "registry must NEVER exceed 50 themes regardless of dir contents (got {})",
            registry.names().len(),
        );
    }
}
