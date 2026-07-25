//! `bind-storage`: persistent, human-inspectable configuration and annotations.
//!
//! Deliberately a plain JSON file under a per-user config directory — no
//! database until one solves a concrete need. Holds user [`Preferences`],
//! [`Keybindings`], and [`Annotation`]s (notes/bookmarks attached to addresses
//! or symbols). Loading is total: a missing or partially-unknown file yields
//! defaults rather than an error.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use bind_core::{BindError, BindResult};
use serde::{Deserialize, Serialize};

/// Visual theme. Bind favors a restrained, monochrome-friendly aesthetic and
/// must degrade when Unicode glyphs are unavailable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Theme {
    #[default]
    Dark,
    Light,
    Monochrome,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Preferences {
    #[serde(default)]
    pub theme: Theme,
    /// When false, the TUI restricts itself to ASCII box-drawing/markers.
    #[serde(default = "default_true")]
    pub unicode: bool,
    /// Show a decimal interpretation alongside hex register values.
    #[serde(default)]
    pub show_decimal: bool,
    /// Interactive trace ring-buffer capacity.
    #[serde(default = "default_ring")]
    pub ring_capacity: usize,
}

fn default_true() -> bool {
    true
}
fn default_ring() -> usize {
    50_000
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            theme: Theme::default(),
            unicode: true,
            show_decimal: false,
            ring_capacity: 50_000,
        }
    }
}

/// Action → key-chord bindings. Values are human strings like `"F5"`, `"s"`,
/// `"ctrl+c"`; the TUI maps them to key events. Missing actions fall back to
/// built-in defaults.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Keybindings {
    #[serde(default)]
    pub bindings: BTreeMap<String, String>,
}

impl Keybindings {
    pub fn defaults() -> Self {
        let mut bindings = BTreeMap::new();
        for (action, key) in [
            ("continue", "c"),
            ("pause", "p"),
            ("step-over", "n"),
            ("step-into", "s"),
            ("step-out", "o"),
            ("step-instruction", "i"),
            ("toggle-breakpoint", "b"),
            ("command-palette", ":"),
            ("search", "/"),
            ("focus-next", "Tab"),
            ("help", "?"),
            ("quit", "q"),
        ] {
            bindings.insert(action.to_string(), key.to_string());
        }
        Self { bindings }
    }

    pub fn get(&self, action: &str) -> Option<&str> {
        self.bindings.get(action).map(String::as_str)
    }
}

/// What an annotation is attached to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AnnotationTarget {
    Address(u64),
    Symbol(String),
    Event(u64),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Annotation {
    pub target: AnnotationTarget,
    pub note: String,
    #[serde(default)]
    pub bookmark: bool,
}

/// The whole persisted configuration document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct BindConfig {
    #[serde(default)]
    pub preferences: Preferences,
    #[serde(default)]
    pub keybindings: Keybindings,
    #[serde(default)]
    pub annotations: Vec<Annotation>,
}

impl BindConfig {
    pub fn with_default_keys() -> Self {
        Self {
            preferences: Preferences::default(),
            keybindings: Keybindings::defaults(),
            annotations: Vec::new(),
        }
    }

    pub fn add_annotation(&mut self, a: Annotation) {
        self.annotations.push(a);
    }

    pub fn bookmarks(&self) -> impl Iterator<Item = &Annotation> {
        self.annotations.iter().filter(|a| a.bookmark)
    }
}

/// Per-user config directory: `$BIND_CONFIG_DIR` or `~/.config/bind`.
pub fn config_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("BIND_CONFIG_DIR") {
        return PathBuf::from(dir);
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join(".config").join("bind")
}

pub fn config_path() -> PathBuf {
    config_dir().join("config.json")
}

/// Loads config from `path`, returning defaults (with default keybindings) if
/// the file is absent. Unknown fields are ignored so a newer config still
/// loads.
pub fn load_from(path: impl AsRef<Path>) -> BindResult<BindConfig> {
    let path = path.as_ref();
    match std::fs::read(path) {
        Ok(bytes) => {
            let mut cfg: BindConfig = serde_json::from_slice(&bytes)
                .map_err(|e| BindError::Internal(format!("parse config: {e}")))?;
            if cfg.keybindings.bindings.is_empty() {
                cfg.keybindings = Keybindings::defaults();
            }
            Ok(cfg)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(BindConfig::with_default_keys()),
        Err(e) => Err(BindError::Internal(format!(
            "read config {}: {e}",
            path.display()
        ))),
    }
}

/// Loads from the default [`config_path`].
pub fn load() -> BindResult<BindConfig> {
    load_from(config_path())
}

/// Saves `cfg` to `path`, creating parent directories.
pub fn save_to(cfg: &BindConfig, path: impl AsRef<Path>) -> BindResult<()> {
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| BindError::Internal(format!("create {}: {e}", parent.display())))?;
    }
    let json = serde_json::to_vec_pretty(cfg)
        .map_err(|e| BindError::Internal(format!("serialize config: {e}")))?;
    std::fs::write(path, json)
        .map_err(|e| BindError::Internal(format!("write {}: {e}", path.display())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_yields_defaults_with_keys() {
        let cfg = load_from("/nonexistent/bind/config.json").unwrap();
        assert!(cfg.keybindings.get("quit").is_some());
    }

    #[test]
    fn roundtrip() {
        let dir = std::env::temp_dir().join(format!("bindcfg-{}", std::process::id()));
        let path = dir.join("config.json");
        let mut cfg = BindConfig::with_default_keys();
        cfg.preferences.theme = Theme::Monochrome;
        cfg.add_annotation(Annotation {
            target: AnnotationTarget::Symbol("main".into()),
            note: "entry".into(),
            bookmark: true,
        });
        save_to(&cfg, &path).unwrap();
        let back = load_from(&path).unwrap();
        assert_eq!(back.preferences.theme, Theme::Monochrome);
        assert_eq!(back.bookmarks().count(), 1);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn unknown_fields_ignored() {
        let json = r#"{"preferences":{"theme":"Light","future":true},"extra":1}"#;
        let dir = std::env::temp_dir().join(format!("bindcfg2-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("c.json");
        std::fs::write(&path, json).unwrap();
        let cfg = load_from(&path).unwrap();
        assert_eq!(cfg.preferences.theme, Theme::Light);
        std::fs::remove_dir_all(&dir).ok();
    }
}
