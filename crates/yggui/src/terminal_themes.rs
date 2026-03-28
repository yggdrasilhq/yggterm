use include_dir::{Dir, include_dir};
use once_cell::sync::Lazy;

static GHOSTTY_THEMES_DIR: Dir<'_> =
    include_dir!("$CARGO_MANIFEST_DIR/../../assets/terminal-themes/ghostty");

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalPaletteSpec {
    pub background: String,
    pub foreground: String,
    pub cursor: String,
    pub selection: String,
    pub black: String,
    pub red: String,
    pub green: String,
    pub yellow: String,
    pub blue: String,
    pub magenta: String,
    pub cyan: String,
    pub white: String,
    pub bright_black: String,
    pub bright_red: String,
    pub bright_green: String,
    pub bright_yellow: String,
    pub bright_blue: String,
    pub bright_magenta: String,
    pub bright_cyan: String,
    pub bright_white: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamedTerminalTheme {
    pub name: String,
    pub palette: TerminalPaletteSpec,
}

static TERMINAL_THEME_CATALOG: Lazy<Vec<NamedTerminalTheme>> = Lazy::new(|| {
    let mut themes = vec![NamedTerminalTheme {
        name: "VS Code Light+".to_string(),
        palette: TerminalPaletteSpec {
            background: "#ffffff".to_string(),
            foreground: "#333333".to_string(),
            cursor: "#007acc".to_string(),
            selection: "rgba(173,214,255,0.45)".to_string(),
            black: "#000000".to_string(),
            red: "#cd3131".to_string(),
            green: "#00bc00".to_string(),
            yellow: "#949800".to_string(),
            blue: "#0451a5".to_string(),
            magenta: "#bc05bc".to_string(),
            cyan: "#0598bc".to_string(),
            white: "#555555".to_string(),
            bright_black: "#666666".to_string(),
            bright_red: "#cd3131".to_string(),
            bright_green: "#14ce14".to_string(),
            bright_yellow: "#b5ba00".to_string(),
            bright_blue: "#0451a5".to_string(),
            bright_magenta: "#bc05bc".to_string(),
            bright_cyan: "#0598bc".to_string(),
            bright_white: "#a5a5a5".to_string(),
        },
    }];

    for file in GHOSTTY_THEMES_DIR.files() {
        let Some(name) = file.path().file_name().and_then(|value| value.to_str()) else {
            continue;
        };
        let Some(contents) = file.contents_utf8() else {
            continue;
        };
        if let Some(palette) = parse_ghostty_theme(contents) {
            themes.push(NamedTerminalTheme {
                name: name.to_string(),
                palette,
            });
        }
    }

    themes.sort_by_cached_key(|theme| theme.name.to_ascii_lowercase());
    themes
});

pub fn terminal_theme_catalog() -> &'static [NamedTerminalTheme] {
    TERMINAL_THEME_CATALOG.as_slice()
}

pub fn terminal_theme_names() -> Vec<String> {
    terminal_theme_catalog()
        .iter()
        .map(|theme| theme.name.clone())
        .collect()
}

pub fn terminal_theme_by_name(name: &str) -> Option<&'static NamedTerminalTheme> {
    terminal_theme_catalog().iter().find(|theme| theme.name == name)
}

fn parse_ghostty_theme(contents: &str) -> Option<TerminalPaletteSpec> {
    let mut palette = vec![String::new(); 16];
    let mut background = None;
    let mut foreground = None;
    let mut cursor = None;
    let mut selection = None;

    for line in contents.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        let value = value.trim();
        if key == "background" {
            background = Some(value.to_string());
            continue;
        }
        if key == "foreground" {
            foreground = Some(value.to_string());
            continue;
        }
        if key == "cursor-color" {
            cursor = Some(value.to_string());
            continue;
        }
        if key == "selection-background" {
            selection = Some(value.to_string());
            continue;
        }
        if key == "palette"
            && let Some((ix, color)) = value.split_once('=')
            && let Ok(ix) = ix.trim().parse::<usize>()
            && ix < 16
        {
            palette[ix] = color.trim().to_string();
        }
    }

    if palette.iter().any(|value| value.is_empty()) {
        return None;
    }

    let background = background?;
    let foreground = foreground?;
    let cursor = cursor.unwrap_or_else(|| foreground.clone());

    Some(TerminalPaletteSpec {
        background,
        foreground,
        cursor,
        selection: selection.unwrap_or_else(|| "rgba(128,128,128,0.28)".to_string()),
        black: palette[0].clone(),
        red: palette[1].clone(),
        green: palette[2].clone(),
        yellow: palette[3].clone(),
        blue: palette[4].clone(),
        magenta: palette[5].clone(),
        cyan: palette[6].clone(),
        white: palette[7].clone(),
        bright_black: palette[8].clone(),
        bright_red: palette[9].clone(),
        bright_green: palette[10].clone(),
        bright_yellow: palette[11].clone(),
        bright_blue: palette[12].clone(),
        bright_magenta: palette[13].clone(),
        bright_cyan: palette[14].clone(),
        bright_white: palette[15].clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ghostty_palette_file() {
        let theme = parse_ghostty_theme(
            "palette = 0=#000000\npalette = 1=#111111\npalette = 2=#222222\npalette = 3=#333333\n\
             palette = 4=#444444\npalette = 5=#555555\npalette = 6=#666666\npalette = 7=#777777\n\
             palette = 8=#888888\npalette = 9=#999999\npalette = 10=#aaaaaa\npalette = 11=#bbbbbb\n\
             palette = 12=#cccccc\npalette = 13=#dddddd\npalette = 14=#eeeeee\npalette = 15=#ffffff\n\
             background = #101010\nforeground = #f0f0f0\ncursor-color = #abcdef\nselection-background = #121212\n",
        )
        .expect("theme should parse");
        assert_eq!(theme.background, "#101010");
        assert_eq!(theme.foreground, "#f0f0f0");
        assert_eq!(theme.bright_white, "#ffffff");
    }
}
