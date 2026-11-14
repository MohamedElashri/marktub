//! Literal settings only. No path, network, execution, interpolation or evaluation semantics.
//! The closed presentation catalogue also supplies GUI metadata and effective validation.
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};

const MAX_FILE: usize = 64 * 1024;
const MAX_LINE: usize = 4096;
const MAX_DIAGNOSTICS: usize = 100;
pub const SCHEMA: u32 = 1;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "camelCase")]
pub enum Value {
    Boolean(bool),
    Number(f64),
    Enum(String),
    Text(String),
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Diagnostic {
    pub line: usize,
    pub message: String,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Settings(BTreeMap<String, Value>);

fn key_valid(key: &str) -> bool {
    !key.is_empty()
        && key.split('.').all(|part| {
            let mut bytes = part.bytes();
            bytes.next().is_some_and(|b| b.is_ascii_alphabetic())
                && bytes.all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
        })
}
// -?(0|[1-9][0-9]*)(\.[0-9]+)? ; no exponents, leading +, NaN or infinities.
fn decimal(text: &str) -> bool {
    let unsigned = text.strip_prefix('-').unwrap_or(text);
    let mut parts = unsigned.split('.');
    let integer = parts.next().unwrap_or("");
    if integer.is_empty()
        || !integer.bytes().all(|b| b.is_ascii_digit())
        || (integer.len() > 1 && integer.starts_with('0'))
    {
        return false;
    }
    if let Some(fraction) = parts.next()
        && (fraction.is_empty() || !fraction.bytes().all(|b| b.is_ascii_digit()))
    {
        return false;
    }
    parts.next().is_none()
}
fn literal(input: &str) -> Result<Value, &'static str> {
    if input.starts_with('"') {
        let mut chars = input.chars();
        chars.next();
        let mut text = String::new();
        while let Some(c) = chars.next() {
            match c {
                '"' if chars.next().is_none() => return Ok(Value::Text(text)),
                '"' => {
                    return Err(
                        "Unexpected text after quoted string; inline comments are not supported.",
                    );
                }
                '\\' => text.push(match chars.next() {
                    Some('"') => '"',
                    Some('\\') => '\\',
                    Some('n') => '\n',
                    Some('t') => '\t',
                    _ => return Err("Only quoted escapes \\\", \\\\, \\n and \\t are supported."),
                }),
                c if c.is_control() => return Err("Escape control characters in quoted strings."),
                c => text.push(c),
            }
        }
        return Err("Unterminated quoted string.");
    }
    match input {
        "true" => Ok(Value::Boolean(true)),
        "false" => Ok(Value::Boolean(false)),
        _ if decimal(input) => {
            let number = input
                .parse::<f64>()
                .map_err(|_| "Expected a finite decimal.")?;
            if !number.is_finite() {
                return Err("Expected a finite decimal.");
            }
            Ok(Value::Number(number))
        }
        _ if input
            .bytes()
            .next()
            .is_some_and(|b| b.is_ascii_alphabetic())
            && input
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-')) =>
        {
            Ok(Value::Enum(input.into()))
        }
        _ => Err("Expected a boolean, finite decimal, bare enum or double-quoted string."),
    }
}
fn validate(key: &str, value: &Value) -> Result<(), &'static str> {
    crate::preferences::validate(key, value)
}

impl Settings {
    pub fn parse(bytes: &[u8]) -> Result<Self, Vec<Diagnostic>> {
        let error = |line, message: &str| Diagnostic {
            line,
            message: message.into(),
        };
        if bytes.len() > MAX_FILE {
            return Err(vec![error(1, "Settings exceed the 64 KiB input budget.")]);
        }
        let text = std::str::from_utf8(bytes)
            .map_err(|_| vec![error(1, "Settings must be valid UTF-8.")])?;
        // Exactly one optional file BOM. A second is ordinary invalid key data.
        let text = text.strip_prefix('\u{feff}').unwrap_or(text);
        let mut values = BTreeMap::new();
        let mut seen = HashSet::new();
        let mut diagnostics = vec![];
        // CRLF and LF are accepted; lone CR is deliberately rejected, not ignored.
        for (index, raw) in text.split_inclusive('\n').enumerate() {
            if diagnostics.len() >= MAX_DIAGNOSTICS {
                break;
            }
            let line = index + 1;
            let raw = if let Some(raw) = raw.strip_suffix('\n') {
                raw.strip_suffix('\r').unwrap_or(raw)
            } else {
                raw
            };
            if raw.contains('\r') || raw.len() > MAX_LINE {
                diagnostics.push(error(
                    line,
                    "Invalid line ending or line exceeds 4096 bytes.",
                ));
                continue;
            }
            let input = raw.trim_matches([' ', '\t']);
            if input.is_empty() || input.starts_with('#') {
                continue;
            }
            let Some((key, value)) = input.strip_prefix('$').and_then(|s| s.split_once('=')) else {
                diagnostics.push(error(line, "Expected $key=value."));
                continue;
            };
            let key = key.trim_matches([' ', '\t']);
            if !key_valid(key) {
                diagnostics.push(error(line, "Expected dotted ASCII setting key."));
                continue;
            }
            if !seen.insert(key) {
                diagnostics.push(error(line, "Duplicate setting key."));
                continue;
            }
            let value = match literal(value.trim_matches([' ', '\t'])) {
                Ok(value) => value,
                Err(message) => {
                    diagnostics.push(error(line, message));
                    continue;
                }
            };
            if let Err(message) = validate(key, &value) {
                diagnostics.push(error(line, message));
                continue;
            }
            values.insert(key.to_owned(), value);
        }
        if diagnostics.is_empty() {
            Ok(Self(values))
        } else {
            Err(diagnostics)
        }
    }
    pub fn defaults() -> Self {
        Self::parse(crate::preferences::defaults().as_bytes()).expect("valid defaults")
    }
    pub fn values(&self) -> BTreeMap<String, Value> {
        self.0.clone()
    }
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.0.get(key)
    }
    /// Increasing precedence: global, workspace, file, session. Every layer has
    /// already passed the same closed allowlist; none can add security authority.
    pub fn resolve(global: &Self, workspace: &Self, file: &Self, session: &Self) -> Self {
        let mut values = Self::defaults().0;
        for layer in [global, workspace, file, session] {
            values.extend(layer.0.clone());
        }
        Self(values)
    }
    /// Reject an entire invalid file and keep the previous settings unchanged.
    pub fn replace_from(&mut self, bytes: &[u8]) -> Result<(), Vec<Diagnostic>> {
        let replacement = Self::parse(bytes)?;
        *self = replacement;
        Ok(())
    }
    pub fn serialize(&self) -> String {
        let mut text = String::new();
        for (key, value) in &self.0 {
            text.push('$');
            text.push_str(key);
            text.push('=');
            match value {
                Value::Boolean(b) => text.push_str(if *b { "true" } else { "false" }),
                Value::Number(n) => text.push_str(&n.to_string()),
                Value::Enum(s) => text.push_str(s),
                Value::Text(s) => {
                    text.push('"');
                    for c in s.chars() {
                        match c {
                            '"' => text.push_str("\\\""),
                            '\\' => text.push_str("\\\\"),
                            '\n' => text.push_str("\\n"),
                            '\t' => text.push_str("\\t"),
                            c => text.push(c),
                        }
                    }
                    text.push('"');
                }
            }
            text.push('\n');
        }
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn defaults_and_literal_strings_roundtrip_without_interpretation() {
        let defaults = Settings::defaults();
        assert_eq!(
            Settings::parse(defaults.serialize().as_bytes()).unwrap(),
            defaults
        );
        let s = Settings::parse(b"# literal text\n$reader.font=\"$(touch /tmp/never) ${HOME} \\\"quoted\\\" \\\\ path\"\n$theme=dark\n").unwrap();
        assert_eq!(Settings::parse(s.serialize().as_bytes()).unwrap(), s);
    }
    #[test]
    fn rejects_duplicate_unknown_keys_types_ranges_escapes_comments_and_partial_application() {
        let mut settings = Settings::defaults();
        let original = settings.clone();
        for text in [
            "$reader.width=800\n$theme=broken",
            "$theme=dark\n$theme=light",
            "$security.execution=true",
            "$reader.width=NaN",
            "$reader.width=1e3",
            "$reader.width=0400",
            "$reader.width=760 # comment",
            "$reader.width=760.5",
            "$reader.width=1601",
            "$reader.width=true",
            "$editor.wrap=\"true\"",
            "$theme=\"dark\"",
            "$reader.font=\"bad\\u0020\"",
            "$reader.font=\"bad\\r\"",
            "$reader.font=\"bad\\x\"",
            "$reader.font=\"unterminated",
            "$reader.font=\"ok\" trailing",
            "$bad..key=1",
            "$reader.width=800\r$theme=dark",
            "$theme=dark\r",
            "$schema.version=2",
        ] {
            assert!(settings.replace_from(text.as_bytes()).is_err(), "{text}");
            assert_eq!(settings, original);
        }
    }
    #[test]
    fn reports_all_invalid_lines_and_keeps_last_good_without_leaking_values() {
        let e =
            Settings::parse(b"# first\n$reader.width=1\n$theme=bogus\n$unknown=\"private value\"")
                .unwrap_err();
        assert_eq!(e.iter().map(|e| e.line).collect::<Vec<_>>(), vec![2, 3, 4]);
        assert!(!format!("{e:?}").contains("private value"));
        assert!(Settings::parse(&[0xff]).is_err());
        assert!(Settings::parse(&vec![b'x'; MAX_FILE + 1]).is_err());
    }
    #[test]
    fn validates_utf8_bom_crlf_and_safe_scope_precedence() {
        let global = Settings::parse(b"\xef\xbb\xbf$theme=light\r\n$reader.width=800\r\n").unwrap();
        let workspace = Settings::parse(b"$theme=dark").unwrap();
        let file = Settings::parse(b"$reader.width=900").unwrap();
        let session = Settings::parse(b"$reader.width=1000").unwrap();
        let resolved = Settings::resolve(&global, &workspace, &file, &session);
        assert_eq!(resolved.get("reader.width"), Some(&Value::Number(1000.0)));
        assert_eq!(resolved.get("theme"), Some(&Value::Enum("dark".into())));
        assert_eq!(resolved.get("editor.wrap"), Some(&Value::Boolean(true)));
    }
}
