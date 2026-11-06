//! Per-machine appearance and listening port.
use crate::notes::{self, Notes};
use serde_json::Value;

/// The machine's look (theme and palette), beside the sync record in the state folder.
pub(super) const LOOK_FILE: &str = "home.json";

/// Light or dark: the system's choice, or one fixed for a viewer that can't ask (the
/// Claude app's file pane always renders light).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Theme {
    #[default]
    Auto,
    Light,
    Dark,
}

impl Theme {
    pub fn parse(word: &str) -> Option<Theme> {
        match word {
            "auto" | "system" => Some(Theme::Auto),
            "light" => Some(Theme::Light),
            "dark" => Some(Theme::Dark),
            _ => None,
        }
    }

    pub(super) fn name(self) -> &'static str {
        match self {
            Theme::Auto => "auto",
            Theme::Light => "light",
            Theme::Dark => "dark",
        }
    }
}

/// The colour family the page is drawn in, each with a light and a dark face.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Palette {
    /// Warm paper and terracotta, the board's first look.
    Ember,
    /// Cool paper and purple, the default look.
    #[default]
    Iris,
    /// Grey-blue paper and steel.
    Slate,
    /// Soft green and forest.
    Moss,
    /// Neutral paper and charcoal.
    Graphite,
}

impl Palette {
    pub fn parse(word: &str) -> Option<Palette> {
        match word {
            "ember" => Some(Palette::Ember),
            "iris" => Some(Palette::Iris),
            "slate" => Some(Palette::Slate),
            "moss" => Some(Palette::Moss),
            "graphite" => Some(Palette::Graphite),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Palette::Ember => "ember",
            Palette::Iris => "iris",
            Palette::Slate => "slate",
            Palette::Moss => "moss",
            Palette::Graphite => "graphite",
        }
    }
}

/// How the page is drawn on this machine. A viewer may still override it on the page,
/// which that browser remembers.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Look {
    pub theme: Theme,
    pub palette: Palette,
}

/// The saved look; the defaults for anything unset or unreadable.
pub fn look(notes: &Notes) -> Look {
    let saved = std::fs::read_to_string(notes.state.join(LOOK_FILE))
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .unwrap_or(Value::Null);
    Look {
        theme: saved["theme"]
            .as_str()
            .and_then(Theme::parse)
            .unwrap_or_default(),
        palette: saved["palette"]
            .as_str()
            .and_then(Palette::parse)
            .unwrap_or_default(),
    }
}

/// Sets what's given and keeps the rest.
pub fn set_look(
    notes: &Notes,
    theme: Option<Theme>,
    palette: Option<Palette>,
) -> Result<(), String> {
    let mut have = look(notes);
    if let Some(t) = theme {
        have.theme = t;
    }
    if let Some(p) = palette {
        have.palette = p;
    }
    // The port and anything else kept there stay as they are.
    let mut saved = std::fs::read_to_string(notes.state.join(LOOK_FILE))
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .unwrap_or_else(|| serde_json::json!({}));
    saved["theme"] = have.theme.name().into();
    saved["palette"] = have.palette.name().into();
    notes::write_text(&notes.state.join(LOOK_FILE), &saved.to_string())
}

/// The port as saved, 0 and all; None when none was.
pub(super) fn saved_port(notes: &Notes) -> Option<u16> {
    std::fs::read_to_string(notes.state.join(LOOK_FILE))
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|v| v["port"].as_u64())
        .and_then(|p| u16::try_from(p).ok())
}

/// The port this machine serves on, beside the look; `SERVE_PORT` when unset.
pub fn port(notes: &Notes) -> u16 {
    std::fs::read_to_string(notes.state.join(LOOK_FILE))
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|v| v["port"].as_u64())
        .and_then(|p| u16::try_from(p).ok())
        .filter(|p| *p != 0)
        .unwrap_or(SERVE_PORT)
}

pub fn set_port(notes: &Notes, port: u16) -> Result<(), String> {
    let mut saved = std::fs::read_to_string(notes.state.join(LOOK_FILE))
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .unwrap_or_else(|| serde_json::json!({}));
    saved["port"] = port.into();
    notes::write_text(&notes.state.join(LOOK_FILE), &saved.to_string())
}

/// The address the home is served at on this machine.
pub fn url(notes: &Notes) -> String {
    format!("http://localhost:{}/", port(notes))
}

/// The port `astrolabe home --serve` takes unless told otherwise.
pub const SERVE_PORT: u16 = 4747;
