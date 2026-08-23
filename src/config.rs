//! The configuration file.
//!
//! The format is a small subset of TOML: flat `key = value` lines, `#` comments, and
//! string, integer, decimal and boolean values. That is all this utility needs, and
//! writing the twenty lines below keeps the dependency tree at the two crates a global
//! input hook can be audited with. See the dependency rule in `CLAUDE.md`.

use std::fs;
use std::path::{Path, PathBuf};

use crate::gesture::{Button, GestureConfig};

/// Everything the configuration file can set.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Config {
    pub gesture: GestureConfig,
    /// Whether gesture recognition starts switched on.
    pub enabled: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            gesture: GestureConfig::default(),
            enabled: true,
        }
    }
}

/// The file that would be written for a default configuration, comments included.
pub const TEMPLATE: &str = "\
# mouse-desktop configuration.
#
# Edit this file, then choose \"Reload configuration\" in the tray menu.
# Delete the file to go back to these defaults.

# Button that has to be held down for a horizontal drag to change desktop.
# One of: middle, right, x1, x2
# The wheel button is the default because almost every mouse has one.
trigger = \"middle\"

# Horizontal distance, in pixels, that moves one desktop. Lower is more sensitive.
threshold_px = 60

# Shortest delay between two switches, in milliseconds. The desktop animation needs
# roughly this long, so lowering it much makes fast drags drop switches.
repeat_cooldown_ms = 250

# How far from horizontal a drag may stray and still count, as a ratio of the horizontal
# distance. 1.0 accepts anything up to 45 degrees; 0.5 demands a flatter drag.
max_vertical_ratio = 1.0

# Swap the direction: with this on, dragging right moves to the previous desktop.
invert = false

# Send a real click when the trigger button is pressed and released without dragging, so
# ordinary clicking with that button keeps working.
replay_click_when_no_gesture = true

# Start with gesture recognition switched on.
enabled = true
";

/// Where the configuration lives, and whether it travels with the executable.
pub fn path() -> PathBuf {
    // Portable mode: a file next to the executable wins, so the utility can be carried on
    // a memory stick and leave nothing behind on the machine.
    if let Some(beside_exe) = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join("config.toml")))
    {
        if beside_exe.is_file() {
            return beside_exe;
        }
    }

    let roaming = std::env::var("APPDATA").unwrap_or_default();
    Path::new(&roaming)
        .join("mouse-desktop")
        .join("config.toml")
}

/// Read the configuration, writing the annotated template first if the file is missing.
///
/// Never fails: anything unreadable falls back to the defaults and is reported through
/// the returned warnings, because a utility that sits in the tray all day should not
/// refuse to start over a typo.
pub fn load_or_create(path: &Path) -> (Config, Vec<String>) {
    let mut warnings = Vec::new();

    if !path.exists() {
        if let Some(parent) = path.parent() {
            if let Err(error) = fs::create_dir_all(parent) {
                warnings.push(format!("could not create {}: {error}", parent.display()));
            }
        }
        if let Err(error) = fs::write(path, TEMPLATE) {
            warnings.push(format!("could not write {}: {error}", path.display()));
        }
    }

    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) => {
            warnings.push(format!("could not read {}: {error}", path.display()));
            return (Config::default(), warnings);
        }
    };

    let (config, parse_warnings) = parse(&text);
    warnings.extend(parse_warnings);
    (config, warnings)
}

/// Parse the configuration text. Unknown keys and unusable values are reported and then
/// ignored, leaving that setting at its default.
pub fn parse(text: &str) -> (Config, Vec<String>) {
    let mut config = Config::default();
    let mut warnings = Vec::new();

    for (number, raw_line) in text.lines().enumerate() {
        let line = strip_comment(raw_line).trim();
        if line.is_empty() {
            continue;
        }

        let Some((key, value)) = line.split_once('=') else {
            warnings.push(format!("line {}: expected key = value", number + 1));
            continue;
        };
        let (key, value) = (key.trim(), value.trim());

        let outcome = match key {
            "trigger" => parse_button(unquote(value)).map(|it| config.gesture.trigger = it),
            "threshold_px" => value
                .parse()
                .map(|it| config.gesture.threshold_px = it)
                .ok(),
            "repeat_cooldown_ms" => value
                .parse()
                .map(|it| config.gesture.repeat_cooldown_ms = it)
                .ok(),
            "max_vertical_ratio" => value
                .parse()
                .map(|it| config.gesture.max_vertical_ratio = it)
                .ok(),
            "invert" => value.parse().map(|it| config.gesture.invert = it).ok(),
            "replay_click_when_no_gesture" => value
                .parse()
                .map(|it| config.gesture.replay_click_when_no_gesture = it)
                .ok(),
            "enabled" => value.parse().map(|it| config.enabled = it).ok(),
            _ => {
                warnings.push(format!("line {}: unknown setting \"{key}\"", number + 1));
                continue;
            }
        };

        if outcome.is_none() {
            warnings.push(format!(
                "line {}: \"{value}\" is not a valid value for {key}, keeping the default",
                number + 1
            ));
        }
    }

    if config.gesture.threshold_px < 1 {
        warnings.push("threshold_px must be at least 1, keeping the default".to_owned());
        config.gesture.threshold_px = GestureConfig::default().threshold_px;
    }
    // Written out rather than as `<= 0.0` so that NaN, which compares false against
    // everything, is caught too.
    let ratio = config.gesture.max_vertical_ratio;
    if ratio.is_nan() || ratio <= 0.0 {
        warnings.push("max_vertical_ratio must be greater than 0, keeping the default".to_owned());
        config.gesture.max_vertical_ratio = GestureConfig::default().max_vertical_ratio;
    }

    (config, warnings)
}

/// Remove a trailing `#` comment, leaving anything inside double quotes alone.
fn strip_comment(line: &str) -> &str {
    let mut inside_quotes = false;
    for (index, character) in line.char_indices() {
        match character {
            '"' => inside_quotes = !inside_quotes,
            '#' if !inside_quotes => return &line[..index],
            _ => {}
        }
    }
    line
}

fn unquote(value: &str) -> &str {
    value.trim_matches('"')
}

fn parse_button(name: &str) -> Option<Button> {
    match name.to_ascii_lowercase().as_str() {
        "middle" | "wheel" => Some(Button::Middle),
        "right" => Some(Button::Right),
        "x1" => Some(Button::X1),
        "x2" => Some(Button::X2),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shipped_template_parses_to_the_defaults() {
        let (config, warnings) = parse(TEMPLATE);
        assert_eq!(config, Config::default());
        assert!(warnings.is_empty(), "{warnings:?}");
    }

    #[test]
    fn every_setting_can_be_changed_from_the_file() {
        let text = "trigger = \"x2\"
threshold_px = 35
repeat_cooldown_ms = 400
max_vertical_ratio = 0.5
invert = true
replay_click_when_no_gesture = false
enabled = false
";
        let (config, warnings) = parse(text);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(
            config,
            Config {
                gesture: GestureConfig {
                    trigger: Button::X2,
                    threshold_px: 35,
                    repeat_cooldown_ms: 400,
                    max_vertical_ratio: 0.5,
                    invert: true,
                    replay_click_when_no_gesture: false,
                },
                enabled: false,
            }
        );
    }

    #[test]
    fn every_trigger_name_is_understood() {
        for (text, expected) in [
            ("middle", Button::Middle),
            ("wheel", Button::Middle),
            ("MIDDLE", Button::Middle),
            ("right", Button::Right),
            ("x1", Button::X1),
            ("x2", Button::X2),
        ] {
            let (config, _) = parse(&format!("trigger = \"{text}\""));
            assert_eq!(config.gesture.trigger, expected, "for {text}");
        }
    }

    #[test]
    fn comments_and_blank_lines_are_ignored() {
        let (config, warnings) = parse("# a comment\n\n   \nthreshold_px = 20 # trailing\n");
        assert_eq!(config.gesture.threshold_px, 20);
        assert!(warnings.is_empty(), "{warnings:?}");
    }

    #[test]
    fn a_hash_inside_a_string_is_not_a_comment() {
        assert_eq!(
            strip_comment("trigger = \"mid#dle\" # real"),
            "trigger = \"mid#dle\" "
        );
    }

    #[test]
    fn an_unusable_value_keeps_the_default_and_is_reported() {
        let (config, warnings) = parse("threshold_px = wide");
        assert_eq!(
            config.gesture.threshold_px,
            GestureConfig::default().threshold_px
        );
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("threshold_px"), "{warnings:?}");
    }

    #[test]
    fn an_unknown_setting_is_reported_rather_than_fatal() {
        let (config, warnings) = parse("colour = \"blue\"\nthreshold_px = 10");
        assert_eq!(config.gesture.threshold_px, 10, "parsing carried on");
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("colour"), "{warnings:?}");
    }

    #[test]
    fn a_line_with_no_equals_sign_is_reported() {
        let (_, warnings) = parse("trigger middle");
        assert_eq!(warnings.len(), 1);
    }

    #[test]
    fn nonsensical_numbers_are_replaced_by_the_defaults() {
        let (config, warnings) = parse("threshold_px = 0\nmax_vertical_ratio = 0");
        assert_eq!(
            config.gesture.threshold_px,
            GestureConfig::default().threshold_px
        );
        assert_eq!(
            config.gesture.max_vertical_ratio,
            GestureConfig::default().max_vertical_ratio
        );
        assert_eq!(warnings.len(), 2, "{warnings:?}");
    }

    #[test]
    fn an_empty_file_gives_the_defaults() {
        let (config, warnings) = parse("");
        assert_eq!(config, Config::default());
        assert!(warnings.is_empty());
    }
}
