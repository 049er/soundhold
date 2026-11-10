use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub const DEFAULT_WIDTH: f64 = 45.0;
pub const DEFAULT_LEVEL: f64 = 0.9;
const WIDTH_RANGE: (f64, f64) = (10.0, 90.0);
const LEVEL_RANGE: (f64, f64) = (0.5, 1.0);

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub codec: Option<String>,
    pub width: f64,
    pub level: f64,
    pub head: Option<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self { codec: None, width: DEFAULT_WIDTH, level: DEFAULT_LEVEL, head: None }
    }
}

pub fn clamp_width(degrees: f64) -> f64 {
    degrees.clamp(WIDTH_RANGE.0, WIDTH_RANGE.1)
}

pub fn clamp_level(level: f64) -> f64 {
    level.clamp(LEVEL_RANGE.0, LEVEL_RANGE.1)
}

/// The folder named by an XDG `variable`, or `fallback` inside the home folder.
pub fn user_folder(variable: &str, fallback: &str) -> Result<PathBuf> {
    std::env::var_os(variable)
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(fallback)))
        .ok_or_else(|| anyhow!("could not find the home folder"))
}

fn path() -> Result<PathBuf> {
    Ok(user_folder("XDG_CONFIG_HOME", ".config")?.join("soundhold/settings.json"))
}

fn parse(text: &str) -> Settings {
    let settings: Settings = serde_json::from_str(text).unwrap_or_default();
    Settings { width: clamp_width(settings.width), level: clamp_level(settings.level), ..settings }
}

impl Settings {
    pub fn load() -> Self {
        path().ok().and_then(|p| std::fs::read_to_string(p).ok()).map_or_else(Self::default, |text| parse(&text))
    }

    pub fn save(&self) -> Result<()> {
        let path = path()?;
        std::fs::create_dir_all(path.parent().unwrap())?;
        std::fs::write(path, serde_json::to_string_pretty(self)?)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_fills_in_and_limits_values() {
        let defaults = Settings::default();
        let cases = [
            ("not json", defaults.clone()),
            ("{}", defaults.clone()),
            (r#"{"width": 60}"#, Settings { width: 60.0, ..defaults.clone() }),
            (r#"{"width": 500, "level": 0.1}"#, Settings { width: 90.0, level: 0.5, ..defaults.clone() }),
            (r#"{"codec": "sbc", "head": "mine.sofa"}"#, Settings { codec: Some("sbc".into()), head: Some("mine.sofa".into()), ..defaults.clone() }),
        ];
        for (text, expected) in cases {
            assert_eq!(parse(text), expected, "{text}");
        }
    }

    #[test]
    fn saved_text_reads_back_the_same() {
        let settings = Settings { codec: Some("aac".into()), width: 30.0, level: 0.75, head: Some("mine.sofa".into()) };
        assert_eq!(parse(&serde_json::to_string_pretty(&settings).unwrap()), settings);
    }
}
