//! 运行时配置模型、默认值、校验与 `settings.json` 持久化。
//!
//! 数据根与数据库一致：生产 `~/.lx-t/`，开发 `~/.lx-t-dev/`；
//! 缺失字段取默认值，未知键忽略，非法已知值由 `validate` 拒绝并返回错误。

pub mod ai;

use std::fmt;
use std::path::{Path, PathBuf};

/// 数据库只读借用的数据根推导与开发环境判定；避免复制两份约定。
use crate::db::{database_dir, is_dev_environment};

/// 运行时配置。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// 侧栏展开宽度（列）。
    pub sidebar_width: u16,
    /// 侧栏展开最小宽度（列，对齐 herdr 默认下限）。
    pub sidebar_min_width: u16,
    /// 侧栏展开最大宽度（列，herdr 默认上限再放大）。
    pub sidebar_max_width: u16,
    /// 低于该宽度隐藏侧栏。
    pub narrow_width: u16,
    /// 最小可用宽度。
    pub min_width: u16,
    /// 最小可用高度。
    pub min_height: u16,
    /// 窗格最小宽度（含边框列）。
    pub min_pane_width: u16,
    /// 窗格最小高度（含边框行）。
    pub min_pane_height: u16,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            sidebar_width: 24,
            sidebar_min_width: 18,
            sidebar_max_width: 40,
            narrow_width: 80,
            min_width: 40,
            min_height: 10,
            min_pane_width: 10,
            min_pane_height: 3,
        }
    }
}

impl Config {
    /// 校验宽度关系；非法组合返回 `ConfigError::Invalid`。
    ///
    /// 侧栏最小值不得大于最大值，展开宽度必须落在闭区间内，各下限不小于 1。
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.sidebar_min_width < 1 || self.sidebar_min_width > self.sidebar_max_width {
            return Err(ConfigError::Invalid(
                "sidebar_min_width must be >= 1 and <= sidebar_max_width".into(),
            ));
        }
        if self.sidebar_width < self.sidebar_min_width
            || self.sidebar_width > self.sidebar_max_width
        {
            return Err(ConfigError::Invalid(
                "sidebar_width must be within sidebar_min_width..=sidebar_max_width".into(),
            ));
        }
        if self.min_width < 1 || self.min_height < 1 {
            return Err(ConfigError::Invalid(
                "min_width and min_height must be >= 1".into(),
            ));
        }
        if self.min_pane_width < 1 || self.min_pane_height < 1 {
            return Err(ConfigError::Invalid(
                "min_pane_width and min_pane_height must be >= 1".into(),
            ));
        }
        if self.narrow_width < 1 {
            return Err(ConfigError::Invalid("narrow_width must be >= 1".into()));
        }
        Ok(())
    }

    /// 序列化为 `settings.json` 文本（缩进两空格，键序固定）。
    pub fn to_json(&self) -> String {
        format!(
            concat!(
                "{{\n",
                "  \"sidebar_width\": {},\n",
                "  \"sidebar_min_width\": {},\n",
                "  \"sidebar_max_width\": {},\n",
                "  \"narrow_width\": {},\n",
                "  \"min_width\": {},\n",
                "  \"min_height\": {},\n",
                "  \"min_pane_width\": {},\n",
                "  \"min_pane_height\": {}\n",
                "}}\n"
            ),
            self.sidebar_width,
            self.sidebar_min_width,
            self.sidebar_max_width,
            self.narrow_width,
            self.min_width,
            self.min_height,
            self.min_pane_width,
            self.min_pane_height,
        )
    }

    /// 从 `settings.json` 文本解析；缺失字段取默认，未知键忽略，非法值报错。
    pub fn from_json(text: &str) -> Result<Self, ConfigError> {
        let value = parse_json_object(text)?;
        let mut config = Self::default();
        for (key, value) in &value {
            let slot = match key.as_str() {
                "sidebar_width" => &mut config.sidebar_width,
                "sidebar_min_width" => &mut config.sidebar_min_width,
                "sidebar_max_width" => &mut config.sidebar_max_width,
                "narrow_width" => &mut config.narrow_width,
                "min_width" => &mut config.min_width,
                "min_height" => &mut config.min_height,
                "min_pane_width" => &mut config.min_pane_width,
                "min_pane_height" => &mut config.min_pane_height,
                _ => continue,
            };
            *slot = json_u16(value).ok_or_else(|| {
                ConfigError::Invalid(format!("{key} must be an integer 0..65535"))
            })?;
        }
        config.validate()?;
        Ok(config)
    }
}

/// 配置错误。
#[derive(Debug)]
pub enum ConfigError {
    /// HOME 缺失，无法定位 settings.json。
    HomeMissing,
    /// IO 错误。
    Io(std::io::Error),
    /// JSON 结构或取值非法。
    Invalid(String),
}

impl fmt::Display for ConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::HomeMissing => write!(formatter, "home directory unavailable"),
            Self::Io(error) => write!(formatter, "io error: {error}"),
            Self::Invalid(message) => write!(formatter, "invalid settings: {message}"),
        }
    }
}

impl std::error::Error for ConfigError {}

impl From<std::io::Error> for ConfigError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

/// 用户数据根：生产 `~/.lx-t`，开发 `~/.lx-t-dev`；与数据库同根。
pub fn user_data_root() -> Result<PathBuf, ConfigError> {
    let db_dir = database_dir(is_dev_environment()).map_err(|_| ConfigError::HomeMissing)?;
    db_dir
        .parent()
        .map(Path::to_path_buf)
        .ok_or(ConfigError::HomeMissing)
}

/// 用户级 settings.json 路径。
pub fn settings_path() -> Result<PathBuf, ConfigError> {
    Ok(user_data_root()?.join("settings.json"))
}

/// 载入用户 settings.json；文件缺失或 HOME 不可用时返回默认配置。
pub fn load_settings() -> Result<Config, ConfigError> {
    let path = match settings_path() {
        Ok(path) => path,
        Err(_) => return Ok(Config::default()),
    };
    load_or_default(&path)
}

/// 保存用户 settings.json；先校验再原子写入。
pub fn save_settings(config: &Config) -> Result<(), ConfigError> {
    save(&settings_path()?, config)
}

/// 载入用户 settings.json；文件缺失或 HOME 不可用时返回默认配置。
///
/// 文件存在但内容非法时保留错误：调用方据此显示可见的加载告警并回退默认值。
pub fn load_or_default(path: &Path) -> Result<Config, ConfigError> {
    match std::fs::read_to_string(path) {
        Ok(text) => Config::from_json(&text),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
        Err(error) => Err(ConfigError::Io(error)),
    }
}

/// 原子保存 settings.json：同目录临时文件写完重命名，失败清理临时文件。
pub fn save(path: &Path, config: &Config) -> Result<(), ConfigError> {
    config.validate()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let temp = temp_path(path);
    std::fs::write(&temp, config.to_json())?;
    if let Err(error) = std::fs::rename(&temp, path) {
        let _ = std::fs::remove_file(&temp);
        return Err(ConfigError::Io(error));
    }
    Ok(())
}

/// 同目录临时文件名：`settings.json.tmp.<pid>`（写入后立即重命名，不跨目录）。
fn temp_path(path: &Path) -> PathBuf {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "settings.json".into());
    path.with_file_name(format!("{name}.tmp.{}", std::process::id()))
}

/// 极简 JSON 对象解析：只接受扁平 `string: u64|string` 键值对，键序无关。
///
/// 仅服务于本项目的 settings.json，不追求通用 JSON 兼容（嵌套值直接跳过）。
fn parse_json_object(text: &str) -> Result<Vec<(String, JsonValue)>, ConfigError> {
    let mut parser = JsonParser::new(text);
    let entries = parser.object()?;
    parser.finish()?;
    Ok(entries)
}

/// 极简 JSON 值：数字（u64）或字符串（其它类型报错由调用方处理）。
enum JsonValue {
    Number(u64),
    Other,
}

/// `JsonValue` 到 u16 的严格转换；非数字或越界返回 None。
fn json_u16(value: &JsonValue) -> Option<u16> {
    match value {
        JsonValue::Number(number) => u16::try_from(*number).ok(),
        JsonValue::Other => None,
    }
}

/// 极简 JSON 解析器：扁平对象、数字与字符串，负数与浮点按非法处理。
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

    /// 解析顶层对象为键值列表。
    fn object(&mut self) -> Result<Vec<(String, JsonValue)>, ConfigError> {
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
                _ => return Err(invalid("expected ',' or '}'")),
            }
        }
    }

    /// 解析一个值；数组与嵌套对象跳过内容后记为 `Other`，字符串与数字如实记录。
    fn value(&mut self) -> Result<JsonValue, ConfigError> {
        match self.peek() {
            Some(b'"') => {
                self.string()?;
                Ok(JsonValue::Other)
            }
            Some(b'{') | Some(b'[') => {
                self.skip_compound()?;
                Ok(JsonValue::Other)
            }
            Some(b'-') => Err(invalid("negative values are not supported")),
            Some(byte) if byte.is_ascii_digit() => Ok(JsonValue::Number(self.number()?)),
            Some(byte) if byte.is_ascii_alphabetic() => {
                // `true` / `false` / `null` 等裸词：跳到分隔符，记为其它类型。
                while self.peek().is_some_and(|byte| byte.is_ascii_alphanumeric()) {
                    self.index += 1;
                }
                Ok(JsonValue::Other)
            }
            _ => Err(invalid("expected a value")),
        }
    }

    /// 跳过嵌套对象/数组（含内部字符串与转义）。
    fn skip_compound(&mut self) -> Result<(), ConfigError> {
        let open = self.peek().ok_or_else(|| invalid("unexpected end"))?;
        let close = match open {
            b'{' => b'}',
            b'[' => b']',
            _ => return Err(invalid("expected compound value")),
        };
        let mut depth = 0_usize;
        loop {
            let byte = self.peek().ok_or_else(|| invalid("unexpected end"))?;
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

    /// 解析字符串字面量并还原转义；不支持 `\u` 转义（键名限定 ASCII）。
    fn string(&mut self) -> Result<String, ConfigError> {
        self.expect(b'"')?;
        let mut out = String::new();
        loop {
            let byte = self.peek().ok_or_else(|| invalid("unterminated string"))?;
            match byte {
                b'"' => {
                    self.index += 1;
                    return Ok(out);
                }
                b'\\' => {
                    self.index += 1;
                    let escaped = self.peek().ok_or_else(|| invalid("dangling escape"))?;
                    let ch = match escaped {
                        b'"' => '"',
                        b'\\' => '\\',
                        b'/' => '/',
                        b'n' => '\n',
                        b't' => '\t',
                        b'r' => '\r',
                        _ => return Err(invalid("unsupported escape")),
                    };
                    out.push(ch);
                    self.index += 1;
                }
                _ if byte.is_ascii() => {
                    out.push(byte as char);
                    self.index += 1;
                }
                _ => {
                    let rest = std::str::from_utf8(&self.bytes[self.index..])
                        .map_err(|_| invalid("invalid utf-8"))?;
                    let ch = rest
                        .chars()
                        .next()
                        .ok_or_else(|| invalid("invalid utf-8"))?;
                    out.push(ch);
                    self.index += ch.len_utf8();
                }
            }
        }
    }

    /// 解析非负整数；负数与小数点按非法拒绝。
    fn number(&mut self) -> Result<u64, ConfigError> {
        let start = self.index;
        while self.peek().is_some_and(|byte| byte.is_ascii_digit()) {
            self.index += 1;
        }
        let digits = &self.bytes[start..self.index];
        if digits.is_empty() {
            return Err(invalid("expected a number"));
        }
        let text = std::str::from_utf8(digits).map_err(|_| invalid("invalid number"))?;
        text.parse::<u64>()
            .map_err(|_| invalid("number out of range"))
    }

    /// 跳过空白字符。
    fn skip_whitespace(&mut self) {
        while self.peek().is_some_and(|byte| byte.is_ascii_whitespace()) {
            self.index += 1;
        }
    }

    /// 断言当前字节并消费。
    fn expect(&mut self, expected: u8) -> Result<(), ConfigError> {
        if self.peek() == Some(expected) {
            self.index += 1;
            return Ok(());
        }
        Err(invalid("unexpected token"))
    }

    /// 当前字节。
    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.index).copied()
    }

    /// 顶层解析结束：剩余内容只能是空白。
    fn finish(&mut self) -> Result<(), ConfigError> {
        self.skip_whitespace();
        if self.index == self.bytes.len() {
            Ok(())
        } else {
            Err(invalid("trailing data"))
        }
    }
}

/// 构造非法 JSON 错误。
fn invalid(message: &str) -> ConfigError {
    ConfigError::Invalid(format!("malformed json: {message}"))
}

#[cfg(test)]
mod tests;
