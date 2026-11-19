//! Preferences and per-document state, saved by eframe between runs.

use papers_render::{NightStyle, Rotation};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const KEY: &str = "papers-settings";
const MAX_RECENT: usize = 30;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Theme {
    #[default]
    System,
    Light,
    Dark,
}

/// How the page is fitted to the window.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Fit {
    #[default]
    Width,
    Page,
    /// Free zoom.
    None,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub night: bool,
    pub night_style: String,
    /// Follow night mode with a dark interface, and day mode with a light one.
    pub theme: Theme,
    pub sidebar: bool,
    pub sidebar_tab: u8,
    pub dual: bool,
    pub check_updates: bool,
    /// Unix seconds of the last automatic update check.
    pub last_update_check: u64,
    pub skipped_version: String,
    pub recent: Vec<Recent>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            night: false,
            night_style: NightStyle::Smart.id().into(),
            theme: Theme::System,
            sidebar: false,
            sidebar_tab: 0,
            dual: false,
            check_updates: true,
            last_update_check: 0,
            skipped_version: String::new(),
            recent: Vec::new(),
        }
    }
}

/// A recently opened document and where the reader left it.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Recent {
    pub path: PathBuf,
    pub page: u32,
    /// Fraction of the page scrolled past the top of the window.
    #[serde(default)]
    pub offset: f32,
    pub zoom: f32,
    pub fit: Fit,
    #[serde(default)]
    pub rotation: u32,
    #[serde(default)]
    pub title: Option<String>,
}

impl Recent {
    pub fn rotation(&self) -> Rotation {
        Rotation::from_degrees(self.rotation as i64)
    }

    pub fn display_name(&self) -> String {
        self.title.clone().filter(|t| !t.is_empty()).unwrap_or_else(|| file_name(&self.path))
    }
}

pub fn file_name(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.display().to_string())
}

impl Settings {
    pub fn night_style(&self) -> NightStyle {
        NightStyle::from_id(&self.night_style).unwrap_or_default()
    }

    pub fn night(&self) -> Option<NightStyle> {
        self.night.then(|| self.night_style())
    }

    pub fn recent_for(&self, path: &Path) -> Option<&Recent> {
        self.recent.iter().find(|r| same_path(&r.path, path))
    }

    pub fn remember(&mut self, entry: Recent) {
        self.recent.retain(|r| !same_path(&r.path, &entry.path));
        self.recent.insert(0, entry);
        self.recent.truncate(MAX_RECENT);
    }

    pub fn forget(&mut self, path: &Path) {
        self.recent.retain(|r| !same_path(&r.path, path));
    }
}

fn same_path(a: &Path, b: &Path) -> bool {
    if cfg!(windows) {
        a.to_string_lossy().eq_ignore_ascii_case(&b.to_string_lossy())
    } else {
        a == b
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recent_list_is_most_recent_first_and_unique() {
        let mut s = Settings::default();
        let entry = |p: &str, page| Recent {
            path: PathBuf::from(p),
            page,
            offset: 0.0,
            zoom: 1.0,
            fit: Fit::Width,
            rotation: 0,
            title: None,
        };
        s.remember(entry("a.pdf", 1));
        s.remember(entry("b.pdf", 2));
        s.remember(entry("a.pdf", 5));
        assert_eq!(s.recent.len(), 2);
        assert_eq!(s.recent[0].path, PathBuf::from("a.pdf"));
        assert_eq!(s.recent_for(Path::new("a.pdf")).unwrap().page, 5);
        for i in 0..50 {
            s.remember(entry(&format!("{i}.pdf"), 0));
        }
        assert_eq!(s.recent.len(), MAX_RECENT);
    }

    #[test]
    fn settings_survive_missing_fields() {
        let s: Settings = serde_json::from_str(r#"{"night":true}"#).unwrap();
        assert!(s.night);
        assert!(s.check_updates);
        assert_eq!(s.night(), Some(NightStyle::Smart));
    }
}
