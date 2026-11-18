//! Stringified NBT (SNBT) formatter and parser.

use std::fmt::Write as _;

use crate::{NbtCompound, NbtError, NbtTag};

fn is_bare_key_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '+')
}

fn write_escaped_string(out: &mut String, s: &str) {
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            other => out.push(other),
        }
    }
    out.push('"');
}

fn format_snbt_into(out: &mut String, tag: &NbtTag) {
    match tag {
        NbtTag::End => {}
        NbtTag::Byte(v) => {
            let _ = write!(out, "{v}b");
        }
        NbtTag::Short(v) => {
            let _ = write!(out, "{v}s");
        }
        NbtTag::Int(v) => {
            let _ = write!(out, "{v}");
        }
        NbtTag::Long(v) => {
            let _ = write!(out, "{v}L");
        }
        NbtTag::Float(v) => {
            let _ = write!(out, "{v:?}f");
        }
        NbtTag::Double(v) => {
            let _ = write!(out, "{v:?}d");
        }
        NbtTag::ByteArray(arr) => {
            out.push_str("[B;");
            for (idx, b) in arr.iter().enumerate() {
                if idx > 0 {
                    out.push(',');
                }
                let _ = write!(out, "{b}b");
            }
            out.push(']');
        }
        NbtTag::String(s) => write_escaped_string(out, s),
        NbtTag::List(items) => {
            out.push('[');
            for (idx, item) in items.iter().enumerate() {
                if idx > 0 {
                    out.push(',');
                }
                format_snbt_into(out, item);
            }
            out.push(']');
        }
        NbtTag::Compound(map) => {
            out.push('{');
            for (idx, (k, v)) in map.iter().enumerate() {
                if idx > 0 {
                    out.push(',');
                }
                if !k.is_empty() && k.chars().all(is_bare_key_char) {
                    out.push_str(k);
                } else {
                    write_escaped_string(out, k);
                }
                out.push(':');
                format_snbt_into(out, v);
            }
            out.push('}');
        }
        NbtTag::IntArray(arr) => {
            out.push_str("[I;");
            for (idx, n) in arr.iter().enumerate() {
                if idx > 0 {
                    out.push(',');
                }
                let _ = write!(out, "{n}");
            }
            out.push(']');
        }
        NbtTag::LongArray(arr) => {
            out.push_str("[L;");
            for (idx, n) in arr.iter().enumerate() {
                if idx > 0 {
                    out.push(',');
                }
                let _ = write!(out, "{n}L");
            }
            out.push(']');
        }
    }
}

/// Formats an [`NbtTag`] as canonical Stringified NBT (SNBT).
#[must_use]
pub fn to_snbt(tag: &NbtTag) -> String {
    let mut out = String::new();
    format_snbt_into(&mut out, tag);
    out
}

struct SnbtParser<'a> {
    src: &'a str,
    pos: usize,
}

impl<'a> SnbtParser<'a> {
    const fn new(src: &'a str) -> Self {
        Self { src, pos: 0 }
    }

    fn skip_ws(&mut self) {
        while let Some(ch) = self.peek() {
            if ch.is_ascii_whitespace() {
                self.pos += ch.len_utf8();
            } else {
                break;
            }
        }
    }

    fn peek(&self) -> Option<char> {
        self.src[self.pos..].chars().next()
    }

    fn next_char(&mut self) -> Option<char> {
        let ch = self.peek()?;
        self.pos += ch.len_utf8();
        Some(ch)
    }

    fn expect_char(&mut self, expected: char) -> Result<(), NbtError> {
        self.skip_ws();
        match self.next_char() {
            Some(c) if c == expected => Ok(()),
            other => Err(NbtError::SnbtSyntax(format!(
                "expected `{expected}` at byte {}, got {other:?}",
                self.pos
            ))),
        }
    }

    fn parse_quoted(&mut self, quote_char: char) -> Result<String, NbtError> {
        let mut out = String::new();
        while let Some(ch) = self.next_char() {
            if ch == quote_char {
                return Ok(out);
            }
            if ch == '\\' {
                let esc = self.next_char().ok_or_else(|| {
                    NbtError::SnbtSyntax("unterminated escape in SNBT string".to_owned())
                })?;
                match esc {
                    '"' => out.push('"'),
                    '\'' => out.push('\''),
                    '\\' => out.push('\\'),
                    'n' => out.push('\n'),
                    'r' => out.push('\r'),
                    't' => out.push('\t'),
                    other => out.push(other),
                }
            } else {
                out.push(ch);
            }
        }
        Err(NbtError::SnbtSyntax("unterminated SNBT string".to_owned()))
    }

    fn parse_bare_token(&mut self) -> &'a str {
        let start = self.pos;
        while let Some(ch) = self.peek() {
            if is_bare_key_char(ch) {
                self.pos += ch.len_utf8();
            } else {
                break;
            }
        }
        &self.src[start..self.pos]
    }

    fn parse_key(&mut self) -> Result<String, NbtError> {
        self.skip_ws();
        if let Some('"' | '\'') = self.peek() {
            let q = self.next_char().unwrap_or('"');
            self.parse_quoted(q)
        } else {
            let key = self.parse_bare_token();
            if key.is_empty() {
                Err(NbtError::SnbtSyntax(format!(
                    "expected compound key at byte {}",
                    self.pos
                )))
            } else {
                Ok(key.to_owned())
            }
        }
    }

    fn parse_value(&mut self, depth: usize) -> Result<NbtTag, NbtError> {
        if depth > crate::DEFAULT_MAX_NBT_DEPTH {
            return Err(NbtError::DepthLimitExceeded(crate::DEFAULT_MAX_NBT_DEPTH));
        }
        self.skip_ws();
        match self.peek() {
            Some('{') => self.parse_compound(depth + 1),
            Some('[') => self.parse_bracketed(depth + 1),
            Some('"' | '\'') => {
                let q = self.next_char().unwrap_or('"');
                Ok(NbtTag::String(self.parse_quoted(q)?))
            }
            Some(_) => self.parse_scalar(),
            None => Err(NbtError::SnbtSyntax("unexpected end of SNBT".to_owned())),
        }
    }

    fn parse_compound(&mut self, depth: usize) -> Result<NbtTag, NbtError> {
        self.expect_char('{')?;
        let mut map = NbtCompound::new();
        self.skip_ws();
        if self.peek() == Some('}') {
            self.next_char();
            return Ok(NbtTag::Compound(map));
        }
        loop {
            let key = self.parse_key()?;
            self.expect_char(':')?;
            let val = self.parse_value(depth)?;
            map.insert(key, val);
            self.skip_ws();
            match self.peek() {
                Some(',') => {
                    self.next_char();
                    self.skip_ws();
                    if self.peek() == Some('}') {
                        self.next_char();
                        break;
                    }
                }
                Some('}') => {
                    self.next_char();
                    break;
                }
                other => {
                    return Err(NbtError::SnbtSyntax(format!(
                        "expected `,` or `}}` at byte {}, got {other:?}",
                        self.pos
                    )));
                }
            }
        }
        Ok(NbtTag::Compound(map))
    }

    fn parse_bracketed(&mut self, depth: usize) -> Result<NbtTag, NbtError> {
        self.expect_char('[')?;
        self.skip_ws();
        let rem = &self.src[self.pos..];
        if rem.starts_with("B;") || rem.starts_with("b;") {
            self.pos += 2;
            return self.parse_typed_array(
                |t| match t {
                    NbtTag::Byte(v) => Some(v),
                    NbtTag::Int(v) => i8::try_from(v).ok(),
                    _ => None,
                },
                NbtTag::ByteArray,
                depth,
            );
        }
        if rem.starts_with("I;") || rem.starts_with("i;") {
            self.pos += 2;
            return self.parse_typed_array(|t| t.as_int(), NbtTag::IntArray, depth);
        }
        if rem.starts_with("L;") || rem.starts_with("l;") {
            self.pos += 2;
            return self.parse_typed_array(
                |t| match t {
                    NbtTag::Long(v) => Some(v),
                    NbtTag::Int(v) => Some(i64::from(v)),
                    _ => None,
                },
                NbtTag::LongArray,
                depth,
            );
        }

        let mut items = Vec::new();
        if self.peek() == Some(']') {
            self.next_char();
            return Ok(NbtTag::List(items));
        }
        loop {
            items.push(self.parse_value(depth)?);
            self.skip_ws();
            match self.peek() {
                Some(',') => {
                    self.next_char();
                    self.skip_ws();
                    if self.peek() == Some(']') {
                        self.next_char();
                        break;
                    }
                }
                Some(']') => {
                    self.next_char();
                    break;
                }
                other => {
                    return Err(NbtError::SnbtSyntax(format!(
                        "expected `,` or `]` at byte {}, got {other:?}",
                        self.pos
                    )));
                }
            }
        }
        Ok(NbtTag::List(items))
    }

    fn parse_typed_array<T>(
        &mut self,
        extract: impl Fn(NbtTag) -> Option<T>,
        wrap: impl Fn(Vec<T>) -> NbtTag,
        depth: usize,
    ) -> Result<NbtTag, NbtError> {
        let mut out = Vec::new();
        self.skip_ws();
        if self.peek() == Some(']') {
            self.next_char();
            return Ok(wrap(out));
        }
        loop {
            let tag = self.parse_value(depth)?;
            let elem = extract(tag)
                .ok_or_else(|| NbtError::SnbtSyntax("typed array element mismatch".to_owned()))?;
            out.push(elem);
            self.skip_ws();
            match self.peek() {
                Some(',') => {
                    self.next_char();
                }
                Some(']') => {
                    self.next_char();
                    break;
                }
                other => {
                    return Err(NbtError::SnbtSyntax(format!(
                        "expected `,` or `]` in typed array, got {other:?}"
                    )));
                }
            }
        }
        Ok(wrap(out))
    }

    fn parse_scalar(&mut self) -> Result<NbtTag, NbtError> {
        let token = self.parse_bare_token();
        if token.is_empty() {
            return Err(NbtError::SnbtSyntax(format!(
                "unexpected character at byte {}",
                self.pos
            )));
        }
        if token.eq_ignore_ascii_case("true") {
            return Ok(NbtTag::Byte(1));
        }
        if token.eq_ignore_ascii_case("false") {
            return Ok(NbtTag::Byte(0));
        }
        if let Some(num) = token.strip_suffix(['b', 'B'])
            && let Ok(v) = num.parse::<i8>()
        {
            return Ok(NbtTag::Byte(v));
        }
        if let Some(num) = token.strip_suffix(['s', 'S'])
            && let Ok(v) = num.parse::<i16>()
        {
            return Ok(NbtTag::Short(v));
        }
        if let Some(num) = token.strip_suffix(['l', 'L'])
            && let Ok(v) = num.parse::<i64>()
        {
            return Ok(NbtTag::Long(v));
        }
        if let Some(num) = token.strip_suffix(['f', 'F'])
            && let Ok(v) = num.parse::<f32>()
        {
            return Ok(NbtTag::Float(v));
        }
        if let Some(num) = token.strip_suffix(['d', 'D'])
            && let Ok(v) = num.parse::<f64>()
        {
            return Ok(NbtTag::Double(v));
        }
        if let Ok(v) = token.parse::<i32>() {
            return Ok(NbtTag::Int(v));
        }
        if (token.contains('.') || token.contains('e') || token.contains('E'))
            && let Ok(v) = token.parse::<f64>()
        {
            return Ok(NbtTag::Double(v));
        }
        Ok(NbtTag::String(token.to_owned()))
    }
}

/// Parses a Stringified NBT (SNBT) literal into an [`NbtTag`].
pub fn from_snbt(input: &str) -> Result<NbtTag, NbtError> {
    let mut parser = SnbtParser::new(input);
    let val = parser.parse_value(1)?;
    parser.skip_ws();
    if parser.pos != input.len() {
        return Err(NbtError::SnbtSyntax(format!(
            "trailing characters at byte {}",
            parser.pos
        )));
    }
    Ok(val)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snbt_roundtrip_complex_compound() {
        let compound = NbtCompound::new()
            .with("byte", NbtTag::Byte(-5))
            .with("short", NbtTag::Short(320))
            .with("int", NbtTag::Int(42_000))
            .with("long", NbtTag::Long(9_000_000_000))
            .with("name", NbtTag::String("Hello \"Minestom\"".to_owned()))
            .with("bytes", NbtTag::ByteArray(vec![1, -2, 3]))
            .with("ints", NbtTag::IntArray(vec![10, 20, 30]))
            .with("longs", NbtTag::LongArray(vec![100, 200]));
        let root = NbtTag::Compound(compound);
        let text = to_snbt(&root);
        let reparsed = from_snbt(&text).expect("parse SNBT");
        assert_eq!(reparsed, root);
    }
}
