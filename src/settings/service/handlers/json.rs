//! Settings HTTP 处理器的极简 JSON 解析：对象、数组、字符串、整数、布尔与 null。

/// 极简 JSON 值：数字、字符串、对象、null 与其它（数组/布尔）。
#[derive(Debug, Clone, PartialEq)]
pub(super) enum JsonValue {
    Number(u64),
    String(String),
    Bool(bool),
    Object(Vec<(String, JsonValue)>),
    Null,
    Other,
}

impl JsonValue {
    pub(super) fn as_number(&self) -> Option<u64> {
        match self {
            Self::Number(number) => Some(*number),
            _ => None,
        }
    }

    pub(super) fn as_string(&self) -> Option<String> {
        match self {
            Self::String(text) => Some(text.clone()),
            _ => None,
        }
    }
}

/// 解析顶层对象为键值对；格式非法返回 None。
pub(super) fn json_object_pairs(text: &str) -> Option<Vec<(String, JsonValue)>> {
    let mut parser = JsonParser::new(text);
    let entries = parser.object().ok()?;
    parser.finish().ok()?;
    Some(entries)
}

/// 取顶层对象的某个嵌套对象字段。
///
/// 字段缺失 → `Ok(None)`；是对象 → `Ok(Some(entries))`；其它类型 → `Err`。
pub(super) fn json_object_field(
    text: &str,
    field: &str,
) -> Result<Option<Vec<(String, JsonValue)>>, String> {
    let mut parser = JsonParser::new(text);
    let entries = parser
        .object()
        .map_err(|_| "invalid JSON body".to_string())?;
    parser
        .finish()
        .map_err(|_| "invalid JSON body".to_string())?;
    for (key, value) in entries {
        if key == field {
            return match value {
                JsonValue::Object(entries) => Ok(Some(entries)),
                _ => Err(format!("invalid JSON: expected object for field {field}")),
            };
        }
    }
    Ok(None)
}

/// 极简 JSON 解析器：支持对象、数组、字符串（含转义）、整数、布尔与 null。
struct JsonParser<'a> {
    bytes: &'a [u8],
    index: usize,
}

impl<'a> JsonParser<'a> {
    fn new(text: &'a str) -> Self {
        Self {
            bytes: text.as_bytes(),
            index: 0,
        }
    }

    /// 解析顶层对象。
    fn object(&mut self) -> Result<Vec<(String, JsonValue)>, ()> {
        self.skip_whitespace();
        self.expect(b'{')?;
        let mut entries = Vec::new();
        self.skip_whitespace();
        if self.peek() == Some(b'}') {
            self.index += 1;
            return Ok(entries);
        }
        loop {
            self.skip_whitespace();
            let key = self.string()?;
            self.skip_whitespace();
            self.expect(b':')?;
            self.skip_whitespace();
            let value = self.value()?;
            entries.push((key, value));
            self.skip_whitespace();
            match self.peek() {
                Some(b',') => self.index += 1,
                Some(b'}') => {
                    self.index += 1;
                    return Ok(entries);
                }
                _ => return Err(()),
            }
        }
    }

    /// 解析任意值。
    fn value(&mut self) -> Result<JsonValue, ()> {
        match self.peek() {
            Some(b'"') => self.string().map(JsonValue::String),
            Some(b'{') => self.object().map(JsonValue::Object),
            Some(b'[') => {
                self.skip_compound(b'[', b']')?;
                Ok(JsonValue::Other)
            }
            Some(byte) if byte.is_ascii_digit() => self.number().map(JsonValue::Number),
            Some(b'-') => Err(()),
            Some(byte) if byte.is_ascii_alphabetic() => {
                let start = self.index;
                while self.peek().is_some_and(|byte| byte.is_ascii_alphanumeric()) {
                    self.index += 1;
                }
                let word = std::str::from_utf8(&self.bytes[start..self.index]).map_err(|_| ())?;
                Ok(match word {
                    "true" => JsonValue::Bool(true),
                    "false" => JsonValue::Bool(false),
                    "null" => JsonValue::Null,
                    _ => JsonValue::Other,
                })
            }
            _ => Err(()),
        }
    }

    fn skip_compound(&mut self, open: u8, close: u8) -> Result<(), ()> {
        let mut depth = 0_usize;
        loop {
            let byte = self.peek().ok_or(())?;
            match byte {
                b'"' => {
                    self.string()?;
                    continue;
                }
                _ if byte == open => depth += 1,
                _ if byte == close => {
                    depth -= 1;
                    self.index += 1;
                    if depth == 0 {
                        return Ok(());
                    }
                    continue;
                }
                _ => {}
            }
            self.index += 1;
        }
    }

    fn string(&mut self) -> Result<String, ()> {
        self.expect(b'"')?;
        let mut out = String::new();
        loop {
            let byte = self.peek().ok_or(())?;
            match byte {
                b'"' => {
                    self.index += 1;
                    return Ok(out);
                }
                b'\\' => {
                    self.index += 1;
                    let escaped = self.peek().ok_or(())?;
                    let ch = match escaped {
                        b'"' => '"',
                        b'\\' => '\\',
                        b'/' => '/',
                        b'n' => '\n',
                        b't' => '\t',
                        b'r' => '\r',
                        b'b' => '\u{8}',
                        b'f' => '\u{c}',
                        b'u' => {
                            let hex = self.bytes.get(self.index + 1..self.index + 5).ok_or(())?;
                            let text = std::str::from_utf8(hex).map_err(|_| ())?;
                            let code = u32::from_str_radix(text, 16).map_err(|_| ())?;
                            self.index += 4;
                            char::from_u32(code).unwrap_or('\u{fffd}')
                        }
                        _ => return Err(()),
                    };
                    out.push(ch);
                    self.index += 1;
                }
                _ if byte.is_ascii() => {
                    out.push(byte as char);
                    self.index += 1;
                }
                _ => {
                    let rest = std::str::from_utf8(&self.bytes[self.index..]).map_err(|_| ())?;
                    let ch = rest.chars().next().ok_or(())?;
                    out.push(ch);
                    self.index += ch.len_utf8();
                }
            }
        }
    }

    fn number(&mut self) -> Result<u64, ()> {
        let start = self.index;
        while self.peek().is_some_and(|byte| byte.is_ascii_digit()) {
            self.index += 1;
        }
        let text = std::str::from_utf8(&self.bytes[start..self.index]).map_err(|_| ())?;
        text.parse::<u64>().map_err(|_| ())
    }

    fn skip_whitespace(&mut self) {
        while self.peek().is_some_and(|byte| byte.is_ascii_whitespace()) {
            self.index += 1;
        }
    }

    fn expect(&mut self, expected: u8) -> Result<(), ()> {
        if self.peek() == Some(expected) {
            self.index += 1;
            return Ok(());
        }
        Err(())
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.index).copied()
    }

    /// 解析结束：剩余只能是空白。
    fn finish(&mut self) -> Result<(), ()> {
        self.skip_whitespace();
        if self.index == self.bytes.len() {
            Ok(())
        } else {
            Err(())
        }
    }
}
