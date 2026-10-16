//! Lightweight JSON parser in pure Rust without external dependencies.
//! Satisfies `--locked` constraint by not requiring `serde_json`.

use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq)]
pub enum JsonValue {
    Null,
    Bool(bool),
    Number(f64),
    String(String),
    Array(Vec<JsonValue>),
    Object(BTreeMap<String, JsonValue>),
}

impl JsonValue {
    pub fn get(&self, key: &str) -> Option<&JsonValue> {
        match self {
            JsonValue::Object(map) => map.get(key),
            _ => None,
        }
    }

    pub fn get_str(&self, key: &str) -> Option<&str> {
        match self.get(key) {
            Some(JsonValue::String(s)) => Some(s.as_str()),
            _ => None,
        }
    }

    pub fn get_bool(&self, key: &str) -> Option<bool> {
        match self.get(key) {
            Some(JsonValue::Bool(b)) => Some(*b),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&Vec<JsonValue>> {
        match self {
            JsonValue::Array(arr) => Some(arr),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            JsonValue::String(s) => Some(s.as_str()),
            _ => None,
        }
    }
}

pub struct JsonParser<'a> {
    chars: Vec<char>,
    pos: usize,
    _marker: std::marker::PhantomData<&'a str>,
}

impl<'a> JsonParser<'a> {
    pub fn parse(input: &str) -> Result<JsonValue, String> {
        let mut parser = JsonParser {
            chars: input.chars().collect(),
            pos: 0,
            _marker: std::marker::PhantomData,
        };
        parser.skip_whitespace();
        let val = parser.parse_value()?;
        parser.skip_whitespace();
        Ok(val)
    }

    fn skip_whitespace(&mut self) {
        while self.pos < self.chars.len() && self.chars[self.pos].is_whitespace() {
            self.pos += 1;
        }
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn parse_value(&mut self) -> Result<JsonValue, String> {
        self.skip_whitespace();
        match self.peek() {
            Some('{') => self.parse_object(),
            Some('[') => self.parse_array(),
            Some('"') => self.parse_string().map(JsonValue::String),
            Some('t') | Some('f') => self.parse_bool().map(JsonValue::Bool),
            Some('n') => self.parse_null(),
            Some(c) if c == '-' || c.is_ascii_digit() => self.parse_number().map(JsonValue::Number),
            Some(c) => Err(format!("unexpected character '{c}' at position {}", self.pos)),
            None => Err("unexpected end of input".to_string()),
        }
    }

    fn parse_object(&mut self) -> Result<JsonValue, String> {
        self.pos += 1; // '{'
        let mut map = BTreeMap::new();
        self.skip_whitespace();
        if self.peek() == Some('}') {
            self.pos += 1;
            return Ok(JsonValue::Object(map));
        }

        loop {
            self.skip_whitespace();
            let key = self.parse_string()?;
            self.skip_whitespace();
            if self.peek() != Some(':') {
                return Err(format!("expected ':' after key at position {}", self.pos));
            }
            self.pos += 1; // ':'
            let val = self.parse_value()?;
            map.insert(key, val);

            self.skip_whitespace();
            match self.peek() {
                Some(',') => {
                    self.pos += 1;
                }
                Some('}') => {
                    self.pos += 1;
                    break;
                }
                Some(c) => return Err(format!("expected ',' or '}}', found '{c}'")),
                None => return Err("unclosed object".to_string()),
            }
        }

        Ok(JsonValue::Object(map))
    }

    fn parse_array(&mut self) -> Result<JsonValue, String> {
        self.pos += 1; // '['
        let mut list = Vec::new();
        self.skip_whitespace();
        if self.peek() == Some(']') {
            self.pos += 1;
            return Ok(JsonValue::Array(list));
        }

        loop {
            let val = self.parse_value()?;
            list.push(val);
            self.skip_whitespace();
            match self.peek() {
                Some(',') => {
                    self.pos += 1;
                }
                Some(']') => {
                    self.pos += 1;
                    break;
                }
                Some(c) => return Err(format!("expected ',' or ']', found '{c}'")),
                None => return Err("unclosed array".to_string()),
            }
        }

        Ok(JsonValue::Array(list))
    }

    fn parse_string(&mut self) -> Result<String, String> {
        self.skip_whitespace();
        if self.peek() != Some('"') {
            return Err(format!("expected '\"' at position {}", self.pos));
        }
        self.pos += 1;
        let mut s = String::new();
        while self.pos < self.chars.len() {
            let c = self.chars[self.pos];
            self.pos += 1;
            match c {
                '"' => return Ok(s),
                '\\' => {
                    if self.pos >= self.chars.len() {
                        return Err("escape at end of string".to_string());
                    }
                    let esc = self.chars[self.pos];
                    self.pos += 1;
                    match esc {
                        '"' => s.push('"'),
                        '\\' => s.push('\\'),
                        '/' => s.push('/'),
                        'b' => s.push('\x08'),
                        'f' => s.push('\x0c'),
                        'n' => s.push('\n'),
                        'r' => s.push('\r'),
                        't' => s.push('\t'),
                        'u' => {
                            if self.pos + 4 > self.chars.len() {
                                return Err("short unicode escape".to_string());
                            }
                            let hex_str: String = self.chars[self.pos..self.pos + 4].iter().collect();
                            self.pos += 4;
                            let cp = u32::from_str_radix(&hex_str, 16)
                                .map_err(|e| format!("bad unicode escape: {e}"))?;
                            if let Some(ch) = char::from_u32(cp) {
                                s.push(ch);
                            }
                        }
                        other => s.push(other),
                    }
                }
                other => s.push(other),
            }
        }
        Err("unterminated string".to_string())
    }

    fn parse_bool(&mut self) -> Result<bool, String> {
        if self.chars[self.pos..].starts_with(&['t', 'r', 'u', 'e']) {
            self.pos += 4;
            Ok(true)
        } else if self.chars[self.pos..].starts_with(&['f', 'a', 'l', 's', 'e']) {
            self.pos += 5;
            Ok(false)
        } else {
            Err(format!("invalid boolean at position {}", self.pos))
        }
    }

    fn parse_null(&mut self) -> Result<JsonValue, String> {
        if self.chars[self.pos..].starts_with(&['n', 'u', 'l', 'l']) {
            self.pos += 4;
            Ok(JsonValue::Null)
        } else {
            Err(format!("invalid null at position {}", self.pos))
        }
    }

    fn parse_number(&mut self) -> Result<f64, String> {
        let start = self.pos;
        if self.peek() == Some('-') {
            self.pos += 1;
        }
        while self.pos < self.chars.len()
            && (self.chars[self.pos].is_ascii_digit()
                || self.chars[self.pos] == '.'
                || self.chars[self.pos] == 'e'
                || self.chars[self.pos] == 'E'
                || self.chars[self.pos] == '+'
                || self.chars[self.pos] == '-')
        {
            self.pos += 1;
        }
        let s: String = self.chars[start..self.pos].iter().collect();
        s.parse::<f64>().map_err(|e| format!("bad number '{s}': {e}"))
    }
}

pub fn parse_json(s: &str) -> Result<JsonValue, String> {
    JsonParser::parse(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_nested_objects() {
        let raw = r#"{"result":{"agent":{"agent_status":"working","pane_id":"%2"}},"nums":[1, 2.5, -3]}"#;
        let v = parse_json(raw).unwrap();
        assert_eq!(
            v.get("result")
                .and_then(|r| r.get("agent"))
                .and_then(|a| a.get_str("agent_status")),
            Some("working")
        );
        assert_eq!(
            v.get("result")
                .and_then(|r| r.get("agent"))
                .and_then(|a| a.get_str("pane_id")),
            Some("%2")
        );
    }
}
