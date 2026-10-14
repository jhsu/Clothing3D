//! Application settings: preferences that apply to every pattern, stored
//! apart from the working session in `$XDG_CONFIG_HOME/clothes3d/settings.json`
//! (default `~/.config/clothes3d/settings.json`).

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::units::Units;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub units: Units,
    /// Label every segment with its length.
    pub show_lengths: bool,
    pub snap_grid: bool,
    /// Grid snap step, mm.
    pub snap_step: f64,
    pub snap_points: bool,
    /// Snap the end of the second sewing edge to where its length matches the first.
    pub snap_match: bool,
    /// Seam length difference (mm) above which a seam is flagged.
    pub seam_tolerance: f64,
    /// Segment length edits move the segment's start point instead of its end.
    pub length_moves_start: bool,
    /// Serve the MCP server so AI agents can edit the open pattern.
    pub mcp_enabled: bool,
    /// Local port for the MCP server (`http://127.0.0.1:<port>/mcp`).
    pub mcp_port: u16,
}

pub const DEFAULT_MCP_PORT: u16 = 7333;

impl Default for Settings {
    fn default() -> Self {
        Self {
            units: Units::Millimeters,
            show_lengths: false,
            snap_grid: true,
            snap_step: 10.0,
            snap_points: true,
            snap_match: true,
            seam_tolerance: 1.0,
            length_moves_start: false,
            mcp_enabled: true,
            mcp_port: DEFAULT_MCP_PORT,
        }
    }
}

impl Settings {
    pub fn path() -> Option<PathBuf> {
        let config = std::env::var_os("XDG_CONFIG_HOME")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
        Some(config.join("clothes3d").join("settings.json"))
    }

    /// Load settings; a missing file gives the defaults, a broken one gives
    /// the defaults plus an error message to show.
    pub fn load() -> (Self, Option<String>) {
        let Some(path) = Self::path() else { return (Self::default(), None) };
        match std::fs::read_to_string(&path) {
            Ok(text) => match serde_json::from_str::<serde_json::Value>(&text)
                .and_then(|v| Ok((serde_json::from_value::<Self>(v.clone())?, v)))
            {
                Ok((mut s, raw)) => {
                    // A file that picks inches but no snap step gets the inch default.
                    if raw.get("snap_step").is_none() {
                        s.snap_step = s.units.default_snap_step();
                    }
                    (s, None)
                }
                Err(e) => (Self::default(), Some(format!("Ignoring {}: {e}", path.display()))),
            },
            Err(_) => (Self::default(), None),
        }
    }

    pub fn save(&self) -> Result<(), String> {
        let path = Self::path().ok_or("no config directory (HOME is not set)")?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let text = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(&path, text).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// Switch display units, moving the snap step to the new unit's default
    /// if it was still at the old unit's default.
    pub fn toggle_units(&mut self) {
        let next = self.units.toggled();
        if (self.snap_step - self.units.default_snap_step()).abs() < 1e-9 {
            self.snap_step = next.default_snap_step();
        }
        self.units = next;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partial_file_fills_defaults() {
        let s: Settings = serde_json::from_str(r#"{"units":"Inches"}"#).unwrap();
        assert_eq!(s.units, Units::Inches);
        assert_eq!(s.seam_tolerance, 1.0);
    }

    #[test]
    fn toggling_units_moves_default_snap_step() {
        let mut s = Settings::default();
        s.toggle_units();
        assert_eq!(s.units, Units::Inches);
        assert_eq!(s.snap_step, 25.4 / 8.0);
        s.snap_step = 5.0; // customized: left alone
        s.toggle_units();
        assert_eq!(s.snap_step, 5.0);
    }
}
