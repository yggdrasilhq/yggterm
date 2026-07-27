//! A minimal JSON reader/writer — enough for one request object per line.
//!
//! **Why not serde.** The crate is deliberately dependency-free so that the FFI
//! floor is its whole cost, and that property is worth more here than it would
//! be in a workspace member: this crate is built only where the WPE dev stack
//! exists, and every dependency is one more thing that has to build there too.
//! The protocol is a flat object of strings, numbers and bools, which is a
//! bounded amount of parser.
//!
//! It is a *reader*, not a validator: unknown keys are kept, and anything the
//! protocol does not ask for is simply never read.

use std::collections::BTreeMap;
use std::fmt::Write as _;

#[derive(Debug, Clone, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Number(f64),
    String(String),
    Array(Vec<Json>),
    Object(BTreeMap<String, Json>),
}

impl Json {
    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Object(map) => map.get(key),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Json::String(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Json::Number(n) => Some(*n),
            _ => None,
        }
    }

    pub fn as_u32(&self) -> Option<u32> {
        self.as_f64()
            .filter(|n| *n >= 0.0 && n.is_finite())
            .map(|n| n as u32)
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Json::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&[Json]> {
        match self {
            Json::Array(items) => Some(items),
            _ => None,
        }
    }

    /// Serialize. Always emits valid JSON: strings are escaped, and a
    /// non-finite number becomes `null` rather than the bare `NaN` token that
    /// would make the whole line unparseable to the other side.
    pub fn to_string(&self) -> String {
        let mut out = String::new();
        self.write(&mut out);
        out
    }

    fn write(&self, out: &mut String) {
        match self {
            Json::Null => out.push_str("null"),
            Json::Bool(true) => out.push_str("true"),
            Json::Bool(false) => out.push_str("false"),
            Json::Number(n) => {
                if n.is_finite() {
                    let _ = write!(out, "{n}");
                } else {
                    out.push_str("null");
                }
            }
            Json::String(s) => write_string(s, out),
            Json::Array(items) => {
                out.push('[');
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    item.write(out);
                }
                out.push(']');
            }
            Json::Object(map) => {
                out.push('{');
                for (i, (key, value)) in map.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    write_string(key, out);
                    out.push(':');
                    value.write(out);
                }
                out.push('}');
            }
        }
    }
}

fn write_string(s: &str, out: &mut String) {
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            // Control characters MUST be escaped or the output is not JSON —
            // and a response carrying page text is exactly where one shows up.
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

/// Build an object from pairs, in one expression.
pub fn obj(pairs: Vec<(&str, Json)>) -> Json {
    Json::Object(
        pairs
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect(),
    )
}

pub fn s(value: impl Into<String>) -> Json {
    Json::String(value.into())
}

pub fn n(value: impl Into<f64>) -> Json {
    Json::Number(value.into())
}

pub fn parse(input: &str) -> Result<Json, String> {
    let bytes: Vec<char> = input.chars().collect();
    let mut at = 0usize;
    skip_ws(&bytes, &mut at);
    let value = parse_value(&bytes, &mut at)?;
    skip_ws(&bytes, &mut at);
    if at != bytes.len() {
        return Err(format!("trailing input at character {at}"));
    }
    Ok(value)
}

fn skip_ws(b: &[char], at: &mut usize) {
    while *at < b.len() && b[*at].is_whitespace() {
        *at += 1;
    }
}

fn parse_value(b: &[char], at: &mut usize) -> Result<Json, String> {
    skip_ws(b, at);
    match b.get(*at) {
        None => Err("unexpected end of input".to_string()),
        Some('{') => parse_object(b, at),
        Some('[') => parse_array(b, at),
        Some('"') => parse_string(b, at).map(Json::String),
        Some('t') => literal(b, at, "true", Json::Bool(true)),
        Some('f') => literal(b, at, "false", Json::Bool(false)),
        Some('n') => literal(b, at, "null", Json::Null),
        Some(_) => parse_number(b, at),
    }
}

fn literal(b: &[char], at: &mut usize, word: &str, value: Json) -> Result<Json, String> {
    if b[*at..].starts_with(&word.chars().collect::<Vec<_>>()[..]) {
        *at += word.len();
        Ok(value)
    } else {
        Err(format!("expected {word} at character {at}"))
    }
}

fn parse_object(b: &[char], at: &mut usize) -> Result<Json, String> {
    *at += 1; // '{'
    let mut map = BTreeMap::new();
    skip_ws(b, at);
    if b.get(*at) == Some(&'}') {
        *at += 1;
        return Ok(Json::Object(map));
    }
    loop {
        skip_ws(b, at);
        let key = parse_string(b, at)?;
        skip_ws(b, at);
        if b.get(*at) != Some(&':') {
            return Err(format!("expected ':' after key {key:?}"));
        }
        *at += 1;
        let value = parse_value(b, at)?;
        map.insert(key, value);
        skip_ws(b, at);
        match b.get(*at) {
            Some(',') => *at += 1,
            Some('}') => {
                *at += 1;
                return Ok(Json::Object(map));
            }
            _ => return Err("expected ',' or '}' in object".to_string()),
        }
    }
}

fn parse_array(b: &[char], at: &mut usize) -> Result<Json, String> {
    *at += 1; // '['
    let mut items = Vec::new();
    skip_ws(b, at);
    if b.get(*at) == Some(&']') {
        *at += 1;
        return Ok(Json::Array(items));
    }
    loop {
        items.push(parse_value(b, at)?);
        skip_ws(b, at);
        match b.get(*at) {
            Some(',') => *at += 1,
            Some(']') => {
                *at += 1;
                return Ok(Json::Array(items));
            }
            _ => return Err("expected ',' or ']' in array".to_string()),
        }
    }
}

fn parse_string(b: &[char], at: &mut usize) -> Result<String, String> {
    if b.get(*at) != Some(&'"') {
        return Err(format!("expected a string at character {at}"));
    }
    *at += 1;
    let mut out = String::new();
    loop {
        match b.get(*at) {
            None => return Err("unterminated string".to_string()),
            Some('"') => {
                *at += 1;
                return Ok(out);
            }
            Some('\\') => {
                *at += 1;
                let escape = b.get(*at).copied().ok_or("unterminated escape")?;
                *at += 1;
                match escape {
                    '"' => out.push('"'),
                    '\\' => out.push('\\'),
                    '/' => out.push('/'),
                    'b' => out.push('\u{8}'),
                    'f' => out.push('\u{c}'),
                    'n' => out.push('\n'),
                    'r' => out.push('\r'),
                    't' => out.push('\t'),
                    'u' => {
                        let hex: String = b.get(*at..*at + 4).ok_or("short \\u escape")?.iter().collect();
                        *at += 4;
                        let code =
                            u32::from_str_radix(&hex, 16).map_err(|_| "bad \\u escape")?;
                        // Lone surrogates cannot be represented; U+FFFD is the
                        // honest substitute and keeps the line parseable.
                        out.push(char::from_u32(code).unwrap_or('\u{FFFD}'));
                    }
                    other => return Err(format!("unknown escape \\{other}")),
                }
            }
            Some(c) => {
                out.push(*c);
                *at += 1;
            }
        }
    }
}

fn parse_number(b: &[char], at: &mut usize) -> Result<Json, String> {
    let start = *at;
    if b.get(*at) == Some(&'-') {
        *at += 1;
    }
    while matches!(b.get(*at), Some(c) if c.is_ascii_digit() || *c == '.' || *c == 'e' || *c == 'E' || *c == '+' || *c == '-')
    {
        *at += 1;
    }
    let text: String = b[start..*at].iter().collect();
    text.parse::<f64>()
        .map(Json::Number)
        .map_err(|_| format!("bad number {text:?}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_request_line_round_trips() {
        let line = r##"{"id":"7","verb":"click","selector":"#go > .btn","width":800}"##;
        let value = parse(line).expect("parses");
        assert_eq!(value.get("id").and_then(Json::as_str), Some("7"));
        assert_eq!(value.get("verb").and_then(Json::as_str), Some("click"));
        assert_eq!(
            value.get("selector").and_then(Json::as_str),
            Some("#go > .btn")
        );
        assert_eq!(value.get("width").and_then(Json::as_u32), Some(800));
        assert_eq!(parse(&value.to_string()).expect("re-parses"), value);
    }

    /// Page text is exactly where a quote, a backslash or a control character
    /// shows up, and an unescaped one makes the whole response line
    /// unparseable to the other side.
    #[test]
    fn strings_that_would_break_the_line_are_escaped() {
        let nasty = "he said \"hi\"\\ \n\t\u{1}end";
        let encoded = s(nasty).to_string();
        assert!(!encoded.contains('\n'), "a raw newline would split the line");
        assert!(encoded.contains("\\u0001"), "control chars must be escaped");
        assert_eq!(parse(&encoded).expect("round trips"), s(nasty));
    }

    #[test]
    fn escapes_decode_back_to_their_characters() {
        let value = parse(r#""a\"b\\c\nd\u0041""#).expect("parses");
        assert_eq!(value.as_str(), Some("a\"b\\c\ndA"));
    }

    #[test]
    fn nested_shapes_parse() {
        let value = parse(r#"{"a":[1,2.5,-3,true,null,{"b":"c"}]}"#).expect("parses");
        let items = value.get("a").and_then(Json::as_array).expect("array");
        assert_eq!(items.len(), 6);
        assert_eq!(items[1].as_f64(), Some(2.5));
        assert_eq!(items[2].as_f64(), Some(-3.0));
        assert_eq!(items[3].as_bool(), Some(true));
        assert_eq!(items[4], Json::Null);
        assert_eq!(items[5].get("b").and_then(Json::as_str), Some("c"));
    }

    #[test]
    fn malformed_input_is_an_error_not_a_panic() {
        for bad in [
            "", "{", "}", "[1,", r#"{"a"}"#, r#"{"a":}"#, r#""unterminated"#,
            "{\"a\":1}{\"b\":2}", "tru", r#"{"a":\}"#,
        ] {
            assert!(parse(bad).is_err(), "{bad:?} should not parse");
        }
    }

    /// A NaN would serialize as a bare token no JSON reader accepts, taking the
    /// whole response down with it.
    #[test]
    fn non_finite_numbers_serialize_as_null() {
        assert_eq!(Json::Number(f64::NAN).to_string(), "null");
        assert_eq!(Json::Number(f64::INFINITY).to_string(), "null");
    }

    #[test]
    fn objects_serialize_deterministically() {
        let a = obj(vec![("z", n(1)), ("a", n(2))]);
        assert_eq!(a.to_string(), r#"{"a":2,"z":1}"#);
    }
}
