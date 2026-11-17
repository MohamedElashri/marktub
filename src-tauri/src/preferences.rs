//! Closed presentation preferences. Values never represent paths, CSS, or execution authority.
use crate::settings::{Diagnostic, Value};
use serde::Serialize;
use std::collections::{BTreeMap, HashSet};
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Spec {
    pub key: String,
    pub label: String,
    pub category: String,
    pub kind: String,
    pub default: String,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub options: Vec<String>,
}
#[allow(clippy::too_many_arguments)] // One row in the closed preference catalogue.
fn spec(
    key: &str,
    label: &str,
    category: &str,
    kind: &str,
    default: &str,
    min: Option<f64>,
    max: Option<f64>,
    options: &[&str],
) -> Spec {
    Spec {
        key: key.into(),
        label: label.into(),
        category: category.into(),
        kind: kind.into(),
        default: default.into(),
        min,
        max,
        options: options.iter().map(|s| s.to_string()).collect(),
    }
}
pub const COMMANDS: &[(&str, &str, &str)] = &[
    ("power", "Export PDF and local jobs", "none"),
    ("review_records", "Manage stored reviews", "none"),
    ("review", "Toggle document review", "none"),
    ("git", "Git workbench", "none"),
    ("open", "Open file", "Mod+O"),
    ("save", "Save", "Mod+S"),
    ("save_as", "Save As", "Mod+Shift+S"),
    ("updates", "Check for application updates", "none"),
    ("preferences", "Preferences", "Mod+Comma"),
    ("palette", "Command palette", "Mod+Shift+P"),
    ("quick_open", "Quick open", "Mod+P"),
    ("search_workspaces", "Search workspaces", "Mod+Shift+F"),
    ("browser", "Toggle file browser", "Mod+Alt+B"),
    ("read", "Read mode", "Mod+Shift+R"),
    ("source", "Source mode", "Mod+Shift+E"),
    ("split", "Split mode", "none"),
    ("visual", "Visual mode", "none"),
    ("focus", "Document focus", "Mod+Alt+F"),
    ("outline", "Outline and headings", "Mod+Shift+O"),
    ("find", "Find in document", "Mod+F"),
    ("reload", "Reload document", "Mod+Alt+R"),
    ("compare", "Compare disk changes", "Mod+Shift+D"),
    ("example", "Open feature guide", "none"),
    ("move_tab_left", "Move tab left", "Alt+Shift+ArrowLeft"),
    ("move_tab_right", "Move tab right", "Alt+Shift+ArrowRight"),
    ("new_note", "New note", "Mod+N"),
    ("note_template", "New note from template", "Mod+Shift+N"),
    ("new_tab", "New tab", "Mod+T"),
    ("close_tab", "Close tab", "Mod+Shift+X"),
    ("next_tab", "Next tab", "Ctrl+Tab"),
    ("previous_tab", "Previous tab", "Ctrl+Shift+Tab"),
    ("back", "Back in file history", "Alt+ArrowLeft"),
    ("forward", "Forward in file history", "Alt+ArrowRight"),
];
pub const COLORS: &[(&str, &str, &str)] = &[
    ("paper", "Page", "#ffffff"),
    ("ink", "Text", "#24262d"),
    ("quiet", "Secondary text", "#5b6170"),
    ("accent", "Links and focus", "#3d55b4"),
    ("on_accent", "Button text", "#ffffff"),
    ("line", "Rules", "#d9dce5"),
    ("surface", "Panels", "#f3f4f7"),
    ("selection", "Selection", "#dce3fa"),
    ("code", "Code background", "#f3f4f7"),
    ("keyword", "Syntax keyword", "#74409a"),
    ("string", "Syntax string", "#345c4b"),
    ("number", "Syntax number", "#87521e"),
    ("added", "Added text", "#17532e"),
    ("added_bg", "Added background", "#e2f2e7"),
    ("removed", "Removed text", "#922c33"),
    ("removed_bg", "Removed background", "#fce7e9"),
];
pub fn catalogue() -> Vec<Spec> {
    let mut list = vec![
        spec(
            "theme",
            "Theme",
            "Appearance",
            "enum",
            "system",
            None,
            None,
            &[
                "system",
                "light",
                "paper",
                "dark",
                "midnight",
                "riwaq_day",
                "riwaq_night",
                "custom",
            ],
        ),
        spec(
            "theme.custom_mode",
            "Custom appearance",
            "Appearance",
            "enum",
            "light",
            None,
            None,
            &["light", "dark"],
        ),
        spec(
            "ui.font",
            "Interface font",
            "Appearance",
            "font",
            "\"System UI\"",
            None,
            None,
            &[],
        ),
        spec(
            "ui.scale",
            "Interface scale (%)",
            "Appearance",
            "number",
            "100",
            Some(80.),
            Some(150.),
            &[],
        ),
        spec(
            "library.width",
            "Library width (px)",
            "Workspace",
            "number",
            "280",
            Some(220.),
            Some(600.),
            &[],
        ),
        spec(
            "editor.line_numbers",
            "Show line numbers by default",
            "Source",
            "boolean",
            "true",
            None,
            None,
            &[],
        ),
        spec(
            "editor.spellcheck",
            "Spellcheck by default",
            "Source",
            "boolean",
            "false",
            None,
            None,
            &[],
        ),
        spec(
            "reader.font",
            "Reading font",
            "Reading",
            "font",
            "\"Palatino\"",
            None,
            None,
            &[],
        ),
        spec(
            "reader.heading_font",
            "Heading font",
            "Reading",
            "font",
            "\"Same as body\"",
            None,
            None,
            &[],
        ),
        spec(
            "reader.font_size",
            "Reading size (px)",
            "Reading",
            "number",
            "18",
            Some(12.),
            Some(40.),
            &[],
        ),
        spec(
            "reader.width",
            "Reading width (px)",
            "Reading",
            "number",
            "760",
            Some(400.),
            Some(1600.),
            &[],
        ),
        spec(
            "reader.line_height",
            "Reading line height (%)",
            "Reading",
            "number",
            "175",
            Some(120.),
            Some(240.),
            &[],
        ),
        spec(
            "editor.font",
            "Source font",
            "Source",
            "font",
            "\"System Mono\"",
            None,
            None,
            &[],
        ),
        spec(
            "editor.font_size",
            "Source size (px)",
            "Source",
            "number",
            "14",
            Some(11.),
            Some(32.),
            &[],
        ),
        spec(
            "editor.line_height",
            "Source line height (%)",
            "Source",
            "number",
            "165",
            Some(120.),
            Some(220.),
            &[],
        ),
        spec(
            "editor.tab_width",
            "Tab width",
            "Source",
            "number",
            "4",
            Some(1.),
            Some(8.),
            &[],
        ),
        spec(
            "editor.wrap",
            "Wrap source lines",
            "Source",
            "boolean",
            "true",
            None,
            None,
            &[],
        ),
        spec(
            "code.style",
            "Code presentation",
            "Source",
            "enum",
            "classic",
            None,
            None,
            &["classic", "quiet", "plain"],
        ),
        spec(
            "recovery.flush_ms",
            "Recovery batching delay (ms)",
            "Source",
            "number",
            "500",
            Some(100.),
            Some(1000.),
            &[],
        ),
    ];
    for (key, label, default) in COLORS {
        list.push(spec(
            &format!("color.{key}"),
            label,
            "Custom colors",
            "color",
            &format!("\"{default}\""),
            None,
            None,
            &[],
        ));
    }
    for (id, label, default) in COMMANDS {
        list.push(spec(
            &format!("shortcut.{id}"),
            label,
            "Shortcuts",
            "shortcut",
            &format!("\"{default}\""),
            None,
            None,
            &[],
        ));
    }
    list
}
pub fn defaults() -> String {
    let mut text = "$schema.version=1\n".to_owned();
    for s in catalogue() {
        text.push_str(&format!("${}={}\n", s.key, s.default));
    }
    text
}
pub fn validate(key: &str, value: &Value) -> Result<(), &'static str> {
    if key == "schema.version" {
        return match value {
            Value::Number(n) if *n == 1. => Ok(()),
            _ => Err("Expected schema version 1."),
        };
    }
    let s = catalogue()
        .into_iter()
        .find(|s| s.key == key)
        .ok_or("Unknown setting; trust, execution and file grants are not importable settings.")?;
    match (s.kind.as_str(), value) {
        ("number", Value::Number(n))
            if n.fract() == 0. && *n >= s.min.unwrap() && *n <= s.max.unwrap() =>
        {
            Ok(())
        }
        ("boolean", Value::Boolean(_)) => Ok(()),
        ("enum", Value::Enum(v)) if s.options.contains(v) => Ok(()),
        ("font", Value::Text(v))
            if !v.is_empty() && v.len() <= 128 && !v.chars().any(char::is_control) =>
        {
            Ok(())
        }
        ("color", Value::Text(v))
            if v.len() == 7
                && v.starts_with('#')
                && v[1..].bytes().all(|b| b.is_ascii_hexdigit()) =>
        {
            Ok(())
        }
        ("shortcut", Value::Text(v)) => chord(v).map(|_| ()),
        _ => Err("Value has the wrong type, option or range for this setting."),
    }
}
fn chord(value: &str) -> Result<Vec<&str>, &'static str> {
    if value == "none" {
        return Ok(vec![]);
    }
    if value.len() > 64 {
        return Err("Shortcut exceeds 64 bytes.");
    }
    let parts = value.split('+').collect::<Vec<_>>();
    if parts.len() < 2 {
        return Err("Use modifiers plus one key, or none.");
    }
    let key = parts[parts.len() - 1];
    if !(key.len() == 1
        && key
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
        || ["Comma", "Tab", "ArrowLeft", "ArrowRight"].contains(&key))
    {
        return Err("Use A–Z, 0–9, Comma, Tab or ArrowLeft/Right.");
    }
    let modifiers = &parts[..parts.len() - 1];
    let mut seen = HashSet::new();
    for m in modifiers {
        if !["Mod", "Ctrl", "Meta", "Alt", "Shift"].contains(m) || !seen.insert(m) {
            return Err("Unknown or repeated shortcut modifier.");
        }
    }
    if !modifiers
        .iter()
        .any(|m| ["Mod", "Meta", "Ctrl", "Alt"].contains(m))
    {
        return Err("A shortcut needs Mod, Ctrl, Meta or Alt.");
    }
    Ok(parts)
}
fn canonical(value: &str, mac: bool) -> Result<String, &'static str> {
    let parts = chord(value)?;
    if parts.is_empty() {
        return Ok("none".into());
    }
    let mut mods = parts[..parts.len() - 1]
        .iter()
        .map(|m| {
            if *m == "Mod" {
                if mac { "Meta" } else { "Ctrl" }
            } else {
                *m
            }
        })
        .collect::<Vec<_>>();
    let count = mods.len();
    mods.sort();
    mods.dedup();
    if mods.len() != count {
        return Err("Mod duplicates a platform modifier.");
    }
    let key = parts.last().unwrap();
    if mods.contains(&"Alt") && *key == "Tab"
        || mods.contains(&"Meta") && mods.contains(&"Shift") && ["3", "4", "5", "6"].contains(key)
        || !mac && mods.contains(&"Ctrl") && mods.contains(&"Alt") && ["L", "T"].contains(key)
        || !mac && mods.contains(&"Meta") && ["L", "D", "A", "Tab"].contains(key)
        || mods.contains(&"Meta") && ["Q", "H", "M", "W", "Tab"].contains(key)
        || mods.contains(&"Ctrl") && ["Q", "ArrowLeft", "ArrowRight"].contains(key)
        || mods.iter().any(|m| ["Meta", "Ctrl"].contains(m))
            && ["Z", "Y", "X", "C", "V", "A"].contains(key)
            && !mods.contains(&"Alt")
            && (!mods.contains(&"Shift") || ["Z", "C", "V"].contains(key))
    {
        return Err("This OS, window-menu or editor shortcut is reserved.");
    }
    Ok(format!("{}+{}", mods.join("+"), key))
}
pub fn effective(values: &BTreeMap<String, Value>) -> Vec<Diagnostic> {
    let mut diagnostics = vec![];
    for mac in [true, false] {
        let mut used = BTreeMap::new();
        for (id, _, _) in COMMANDS {
            let key = format!("shortcut.{id}");
            if let Some(Value::Text(v)) = values.get(&key) {
                match canonical(v, mac) {
                    Ok(c) if c != "none" => {
                        if used.insert(c, key.clone()).is_some() {
                            diagnostics.push(Diagnostic {
                                line: 1,
                                message: format!(
                                    "Shortcut conflict on {}: {key}.",
                                    if mac { "macOS" } else { "Linux" }
                                ),
                            });
                        }
                    }
                    Err(message) => diagnostics.push(Diagnostic {
                        line: 1,
                        message: format!("{key}: {message}"),
                    }),
                    _ => {}
                }
            }
        }
    }
    if values.get("theme") == Some(&Value::Enum("custom".into())) {
        for (a, b) in [
            ("ink", "paper"),
            ("quiet", "paper"),
            ("accent", "paper"),
            ("on_accent", "accent"),
            ("ink", "surface"),
            ("ink", "selection"),
            ("ink", "code"),
            ("quiet", "code"),
            ("accent", "surface"),
            ("keyword", "paper"),
            ("string", "paper"),
            ("number", "paper"),
            ("keyword", "code"),
            ("string", "code"),
            ("number", "code"),
            ("added", "added_bg"),
            ("removed", "removed_bg"),
        ] {
            let color = |key: &str| match values.get(&format!("color.{key}")) {
                Some(Value::Text(s)) => s.as_str(),
                _ => "#000000",
            };
            if contrast(color(a), color(b)) < 4.5 {
                diagnostics.push(Diagnostic {
                    line: 1,
                    message: format!("Custom {a}/{b} contrast must be at least 4.5:1."),
                });
            }
        }
    }
    diagnostics
}
fn contrast(a: &str, b: &str) -> f64 {
    let light = |s: &str| {
        let rgb =
            [1, 3, 5].map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap_or(0) as f64 / 255.);
        let c = rgb.map(|c| {
            if c <= 0.04045 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        });
        0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
    };
    let (a, b) = (light(a), light(b));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::Settings;
    #[test]
    fn catalogue_defaults_and_shortcuts_are_valid_on_both_platforms() {
        let settings = Settings::defaults();
        assert!(
            effective(&settings.values()).is_empty(),
            "{:?}",
            effective(&settings.values())
        );
        assert!(catalogue().len() > 40);
        for spec in catalogue() {
            assert!(settings.get(&spec.key).is_some());
        }
    }
    #[test]
    fn riwaq_and_heading_typography_use_the_native_typed_catalogue() {
        for theme in ["riwaq_day", "riwaq_night"] {
            let text = format!(
                "$theme={theme}\n$reader.heading_font=\"Amiri\"\n$reader.font=\"Noto Naskh Arabic\""
            );
            let layer = Settings::parse(text.as_bytes()).unwrap();
            let empty = Settings::parse(b"").unwrap();
            let resolved = Settings::resolve(&layer, &empty, &empty, &empty);
            assert!(effective(&resolved.values()).is_empty());
            assert_eq!(
                resolved.get("reader.heading_font"),
                Some(&Value::Text("Amiri".into()))
            );
        }
        assert!(Settings::parse(b"$theme=riwaq_unknown").is_err());
        assert!(Settings::parse(b"$reader.heading_font=42").is_err());
    }
    #[test]
    fn rejects_reserved_duplicate_and_cross_platform_aliases() {
        for text in [
            "$shortcut.open=\"Mod+Q\"",
            "$shortcut.open=\"Mod+Z\"",
            "$shortcut.open=\"Ctrl+P\"",
            "$shortcut.open=\"Mod+Ctrl+O\"",
        ] {
            let layer = Settings::parse(text.as_bytes()).unwrap();
            let resolved = Settings::resolve(
                &layer,
                &Settings::parse(b"").unwrap(),
                &Settings::parse(b"").unwrap(),
                &Settings::parse(b"").unwrap(),
            );
            assert!(!effective(&resolved.values()).is_empty(), "{text}");
        }
        for text in [
            "$shortcut.open=\"eval()\"",
            "$color.ink=\"url(file:///secret)\"",
            "$ui.scale=1000",
            "$editor.font_size=2",
        ] {
            assert!(Settings::parse(text.as_bytes()).is_err());
        }
    }
    #[test]
    fn custom_contrast_is_checked_after_cascade() {
        let empty = Settings::parse(b"").unwrap();
        let good = Settings::parse(b"$theme=custom").unwrap();
        assert!(effective(&Settings::resolve(&good, &empty, &empty, &empty).values()).is_empty());
        let bad = Settings::parse(b"$color.ink=\"#ffffff\"").unwrap();
        let resolved = Settings::resolve(&good, &empty, &bad, &empty);
        assert!(
            effective(&resolved.values())
                .iter()
                .any(|d| d.message.contains("ink/paper"))
        );
    }

    #[test]
    fn layout_and_editor_defaults_have_closed_typed_ranges() {
        for text in [
            "$library.width=219",
            "$library.width=601",
            "$library.width=280.5",
            "$editor.line_numbers=1",
            "$editor.spellcheck=yes",
        ] {
            assert!(Settings::parse(text.as_bytes()).is_err(), "{text}");
        }
        let values = Settings::parse(
            b"$library.width=420\n$editor.line_numbers=false\n$editor.spellcheck=true",
        )
        .unwrap();
        assert!(
            effective(
                &Settings::resolve(
                    &values,
                    &Settings::parse(b"").unwrap(),
                    &Settings::parse(b"").unwrap(),
                    &Settings::parse(b"").unwrap()
                )
                .values()
            )
            .is_empty()
        );
    }
}
