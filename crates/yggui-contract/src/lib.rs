use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum UiTheme {
    ZedDark,
    ZedLight,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct YgguiThemeColorStop {
    pub color: String,
    pub x: f32,
    pub y: f32,
    pub alpha: f32,
}

impl Default for YgguiThemeColorStop {
    fn default() -> Self {
        Self {
            color: "#7cc8ff".to_string(),
            x: 0.5,
            y: 0.5,
            alpha: 0.82,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct YgguiThemeSpec {
    pub colors: Vec<YgguiThemeColorStop>,
    pub brightness: f32,
    pub grain: f32,
}

impl Default for YgguiThemeSpec {
    fn default() -> Self {
        Self {
            colors: Vec::new(),
            brightness: 0.56,
            grain: 0.12,
        }
    }
}
