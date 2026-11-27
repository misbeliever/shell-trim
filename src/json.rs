use std::fmt::Write as _;

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum JsonValue {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Array(Vec<JsonValue>),
    Object(Vec<(String, JsonValue)>),
}

impl JsonValue {
    pub(crate) fn get(&self, key: &str) -> Option<&JsonValue> {
        match self {
            JsonValue::Object(entries) => entries.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub(crate) fn as_str(&self) -> Option<&str> {
        match self {
            JsonValue::Str(s) => Some(s.as_str()),
            _ => None,
        }
    }

    pub(crate) fn as_bool(&self) -> Option<bool> {
        match self {
            JsonValue::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub(crate) fn as_f64(&self) -> Option<f64> {
        match self {
            JsonValue::Num(n) => Some(*n),
            _ => None,
        }
    }

    pub(crate) fn as_u64(&self) -> Option<u64> {
        self.as_f64()
            .map(|n| if n <= 0.0 { 0 } else { n.round() as u64 })
    }

    pub(crate) fn as_i64(&self) -> Option<i64> {
        self.as_f64().map(|n| n.round() as i64)
    }

    pub(crate) fn as_array(&self) -> Option<&[JsonValue]> {
        match self {
            JsonValue::Array(arr) => Some(arr.as_slice()),
            _ => None,
        }
    }

    pub(crate) fn as_object(&self) -> Option<&[(String, JsonValue)]> {
        match self {
            JsonValue::Object(entries) => Some(entries.as_slice()),
            _ => None,
        }
    }

    pub(crate) fn as_object_mut(&mut self) -> Option<&mut Vec<(String, JsonValue)>> {
        match self {
            JsonValue::Object(entries) => Some(entries),
            _ => None,
        }
    }

    pub(crate) fn to_compact_string(&self) -> String {
        let mut out = String::new();
        self.write_compact(&mut out);
        out
    }

    pub(crate) fn to_pretty_string(&self, indent: usize) -> String {
        let mut out = String::new();
        self.write_pretty(&mut out, indent, 0);
        out.push('\n');
        out
    }

    fn write_compact(&self, out: &mut String) {
        match self {
            JsonValue::Null => out.push_str("null"),
            JsonValue::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            JsonValue::Num(n) => write_json_num(*n, out),
            JsonValue::Str(s) => out.push_str(&json_string(s)),
            JsonValue::Array(items) => {
                out.push('[');
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    item.write_compact(out);
                }
                out.push(']');
            }
            JsonValue::Object(entries) => {
                out.push('{');
                for (i, (k, v)) in entries.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    out.push_str(&json_string(k));
                    out.push(':');
                    v.write_compact(out);
                }
                out.push('}');
            }
        }
    }

    fn write_pretty(&self, out: &mut String, step: usize, depth: usize) {
        match self {
            JsonValue::Null | JsonValue::Bool(_) | JsonValue::Num(_) | JsonValue::Str(_) => {
                self.write_compact(out);
            }
            JsonValue::Array(items) => {
                if items.is_empty() {
                    out.push_str("[]");
                    return;
                }
                out.push_str("[\n");
                let child_pad = " ".repeat((depth + 1) * step);
                for (i, item) in items.iter().enumerate() {
                    out.push_str(&child_pad);
                    item.write_pretty(out, step, depth + 1);
                    if i + 1 < items.len() {
                        out.push(',');
                    }
                    out.push('\n');
                }
                out.push_str(&" ".repeat(depth * step));
                out.push(']');
            }
            JsonValue::Object(entries) => {
                if entries.is_empty() {
                    out.push_str("{}");
                    return;
                }
                out.push_str("{\n");
                let child_pad = " ".repeat((depth + 1) * step);
                for (i, (k, v)) in entries.iter().enumerate() {
                    out.push_str(&child_pad);
                    out.push_str(&json_string(k));
                    out.push_str(": ");
                    v.write_pretty(out, step, depth + 1);
                    if i + 1 < entries.len() {
                        out.push(',');
                    }
                    out.push('\n');
                }
                out.push_str(&" ".repeat(depth * step));
                out.push('}');
            }
        }
    }
}

fn write_json_num(n: f64, out: &mut String) {
    if !n.is_finite() {
        out.push_str("null");
    } else if n.fract() == 0.0 && (-9_007_199_254_740_991.0..=9_007_199_254_740_991.0).contains(&n)
    {
        let _ = write!(out, "{}", n as i64);
    } else {
        let _ = write!(out, "{n}");
    }
}

pub(crate) fn json_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

pub(crate) struct JsonParser<'a> {
    src: &'a str,
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> JsonParser<'a> {
    pub(crate) fn parse(src: &'a str) -> Result<JsonValue, String> {
        let mut p = Self {
            src,
            bytes: src.as_bytes(),
            pos: 0,
        };
        let val = p.parse_value()?;
        p.skip_ws();
        if p.pos < p.bytes.len() {
            return Err(format!("unexpected trailing content at byte {}", p.pos));
        }
        Ok(val)
    }

    pub(crate) fn parse_string_at(src: &'a str, start: usize) -> Option<(String, usize)> {
        let mut p = Self {
            src,
            bytes: src.as_bytes(),
            pos: start,
        };
        let val = p.parse_string().ok()?;
        Some((val, p.pos))
    }

    fn skip_ws(&mut self) {
        while self.pos < self.bytes.len() && self.bytes[self.pos].is_ascii_whitespace() {
            self.pos += 1;
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn expect(&mut self, byte: u8) -> Result<(), String> {
        match self.peek() {
            Some(b) if b == byte => {
                self.pos += 1;
                Ok(())
            }
            Some(b) => Err(format!(
                "expected '{}' but found '{}' at byte {}",
                byte as char, b as char, self.pos
            )),
            None => Err(format!("expected '{}' at EOF", byte as char)),
        }
    }

    fn parse_value(&mut self) -> Result<JsonValue, String> {
        self.skip_ws();
        match self.peek() {
            Some(b'{') => self.parse_object(),
            Some(b'[') => self.parse_array(),
            Some(b'"') => self.parse_string().map(JsonValue::Str),
            Some(b't') => self.parse_literal("true", JsonValue::Bool(true)),
            Some(b'f') => self.parse_literal("false", JsonValue::Bool(false)),
            Some(b'n') => self.parse_literal("null", JsonValue::Null),
            Some(b'-' | b'0'..=b'9') => self.parse_number(),
            Some(b) => Err(format!("unexpected '{}' at byte {}", b as char, self.pos)),
            None => Err("unexpected EOF while parsing JSON".into()),
        }
    }

    fn parse_literal(&mut self, lit: &str, val: JsonValue) -> Result<JsonValue, String> {
        if self.src[self.pos..].starts_with(lit) {
            self.pos += lit.len();
            Ok(val)
        } else {
            Err(format!("invalid literal at byte {}", self.pos))
        }
    }

    fn parse_number(&mut self) -> Result<JsonValue, String> {
        let start = self.pos;
        if self.peek() == Some(b'-') {
            self.pos += 1;
        }
        let mut has_digits = false;
        while let Some(b) = self.peek() {
            if b.is_ascii_digit() {
                has_digits = true;
                self.pos += 1;
            } else {
                break;
            }
        }
        if !has_digits {
            return Err(format!("invalid number at byte {start}"));
        }
        if self.peek() == Some(b'.') {
            self.pos += 1;
            let mut frac_digits = false;
            while let Some(b) = self.peek() {
                if b.is_ascii_digit() {
                    frac_digits = true;
                    self.pos += 1;
                } else {
                    break;
                }
            }
            if !frac_digits {
                return Err(format!("invalid decimal fraction at byte {start}"));
            }
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.pos += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.pos += 1;
            }
            let mut exp_digits = false;
            while let Some(b) = self.peek() {
                if b.is_ascii_digit() {
                    exp_digits = true;
                    self.pos += 1;
                } else {
                    break;
                }
            }
            if !exp_digits {
                return Err(format!("invalid exponent at byte {start}"));
            }
        }
        let num: f64 = self.src[start..self.pos].parse().map_err(|_| {
            format!(
                "invalid number '{}' at byte {start}",
                &self.src[start..self.pos]
            )
        })?;
        Ok(JsonValue::Num(num))
    }

    fn parse_string(&mut self) -> Result<String, String> {
        self.expect(b'"')?;
        let mut out = String::new();
        while self.pos < self.bytes.len() {
            let b = self.bytes[self.pos];
            if b == b'"' {
                self.pos += 1;
                return Ok(out);
            }
            if b == b'\\' {
                self.pos += 1;
                if self.pos >= self.bytes.len() {
                    return Err("unterminated escape sequence in string".into());
                }
                match self.bytes[self.pos] {
                    b'"' => out.push('"'),
                    b'\\' => out.push('\\'),
                    b'/' => out.push('/'),
                    b'b' => out.push('\u{0008}'),
                    b'f' => out.push('\u{000c}'),
                    b'n' => out.push('\n'),
                    b'r' => out.push('\r'),
                    b't' => out.push('\t'),
                    b'u' => {
                        let unit1 = self.parse_hex4()?;
                        if (0xD800..=0xDBFF).contains(&unit1)
                            && self.pos + 2 < self.bytes.len()
                            && self.bytes[self.pos + 1] == b'\\'
                            && self.bytes[self.pos + 2] == b'u'
                        {
                            self.pos += 2;
                            let unit2 = self.parse_hex4()?;
                            if (0xDC00..=0xDFFF).contains(&unit2) {
                                let cp = 0x10000
                                    + (((unit1 as u32 - 0xD800) << 10) | (unit2 as u32 - 0xDC00));
                                if let Some(ch) = char::from_u32(cp) {
                                    out.push(ch);
                                }
                            } else if let Some(ch) = char::from_u32(unit1 as u32) {
                                out.push(ch);
                            }
                        } else if let Some(ch) = char::from_u32(unit1 as u32) {
                            out.push(ch);
                        }
                    }
                    other => {
                        return Err(format!("invalid string escape '\\{}'", other as char));
                    }
                }
                self.pos += 1;
                continue;
            }
            let ch = self.src[self.pos..]
                .chars()
                .next()
                .ok_or_else(|| "invalid UTF-8 in string".to_string())?;
            out.push(ch);
            self.pos += ch.len_utf8();
        }
        Err("unterminated JSON string".into())
    }

    fn parse_hex4(&mut self) -> Result<u16, String> {
        if self.pos + 4 >= self.bytes.len() {
            return Err("incomplete \\uXXXX escape".into());
        }
        let hex = &self.src[self.pos + 1..self.pos + 5];
        let val = u16::from_str_radix(hex, 16)
            .map_err(|_| format!("invalid \\uXXXX escape '\\u{hex}'"))?;
        self.pos += 4;
        Ok(val)
    }

    fn parse_array(&mut self) -> Result<JsonValue, String> {
        self.expect(b'[')?;
        let mut items = Vec::new();
        self.skip_ws();
        if self.peek() == Some(b']') {
            self.pos += 1;
            return Ok(JsonValue::Array(items));
        }
        loop {
            items.push(self.parse_value()?);
            self.skip_ws();
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b']') => {
                    self.pos += 1;
                    return Ok(JsonValue::Array(items));
                }
                _ => return Err(format!("expected ',' or ']' in array at byte {}", self.pos)),
            }
        }
    }

    fn parse_object(&mut self) -> Result<JsonValue, String> {
        self.expect(b'{')?;
        let mut entries = Vec::new();
        self.skip_ws();
        if self.peek() == Some(b'}') {
            self.pos += 1;
            return Ok(JsonValue::Object(entries));
        }
        loop {
            self.skip_ws();
            if self.peek() != Some(b'"') {
                return Err(format!("expected object key string at byte {}", self.pos));
            }
            let key = self.parse_string()?;
            self.skip_ws();
            self.expect(b':')?;
            let val = self.parse_value()?;
            entries.push((key, val));
            self.skip_ws();
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b'}') => {
                    self.pos += 1;
                    return Ok(JsonValue::Object(entries));
                }
                _ => {
                    return Err(format!(
                        "expected ',' or '}}' in object at byte {}",
                        self.pos
                    ));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_json_escapes_arrays_and_roundtrips_pretty() {
        let src = r#"{
            "s": "line1\n\"quoted\"\t\\slash \u0041",
            "arr": [1, -2.5, true, null, {"nested": 3e2}]
        }"#;
        let val = JsonParser::parse(src).expect("valid json");
        assert_eq!(
            val.get("s").and_then(|v| v.as_str()),
            Some("line1\n\"quoted\"\t\\slash A")
        );
        let arr = val.get("arr").and_then(|v| v.as_array()).unwrap();
        assert_eq!(arr[0].as_u64(), Some(1));
        assert_eq!(arr[1].as_f64(), Some(-2.5));
        assert_eq!(arr[2].as_bool(), Some(true));
        assert_eq!(arr[3], JsonValue::Null);
        assert_eq!(arr[4].get("nested").and_then(|v| v.as_f64()), Some(300.0));

        let reparsed = JsonParser::parse(&val.to_pretty_string(2)).unwrap();
        assert_eq!(reparsed, val);
    }
}
