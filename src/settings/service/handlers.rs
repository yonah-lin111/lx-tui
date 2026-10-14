//! 设置服务路由与处理器：请求解析、token/同源校验、配置与命令 CRUD。

use super::{MAX_BODY_BYTES, MAX_HEADER_BYTES, ServiceState, SettingsEvent, page};
use std::collections::HashMap;

use crate::config::Config;
use crate::settings::commands::{
    self, BlockType, CommandKind, CommandRecord, CommandScope, MdScope,
};

/// `GET /api/state` 与 `PUT /api/commands` 使用的命令视图。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandView {
    pub name: String,
    /// `user` 或 `project`。
    pub scope: String,
    /// `agentMD` 或 `agentBlock`。
    pub kind: String,
    pub description: String,
    /// `agentMD` 的 `global` / `template`；`agentBlock` 恒为 `template`。
    pub md_scope: String,
    /// `agentBlock` 的标题；其余为空。
    pub title: String,
    /// `agentBlock` 的块类型（`template` / `supple`）；其余恒为 `template`。
    pub block_type: String,
    pub body: String,
}

impl CommandView {
    /// 由存储记录转换；`scope` 由调用方给定（合并列表保留来源作用域）。
    pub fn of(record: &CommandRecord, scope: CommandScope) -> Self {
        Self {
            name: record.name.clone(),
            scope: scope.as_str().to_string(),
            kind: record.kind.as_str().to_string(),
            description: record.description.clone(),
            md_scope: record.scope.as_str().to_string(),
            title: record.title.clone().unwrap_or_default(),
            block_type: record.block.as_str().to_string(),
            body: record.body.clone(),
        }
    }
}

/// 已解析的 HTTP 请求。
#[derive(Debug, Clone)]
pub struct Request {
    pub method: String,
    /// 不含 query 的路径。
    pub path: String,
    pub query: HashMap<String, String>,
    /// header 名小写。
    pub headers: HashMap<String, String>,
    pub body: String,
}

/// 处理结果。
#[derive(Debug)]
pub struct Response {
    pub status: u16,
    pub content_type: &'static str,
    pub body: String,
}

impl Response {
    /// JSON 响应。
    pub fn json(status: u16, body: impl Into<String>) -> Self {
        Self {
            status,
            content_type: "application/json; charset=utf-8",
            body: body.into(),
        }
    }

    /// 错误 JSON 响应。
    pub fn error(status: u16, message: &str) -> Self {
        Self::json(status, format!("{{\"error\":{}}}", json_string(message)))
    }
}

/// 尝试从已读缓冲解析完整请求；数据不完整返回 None，格式非法返回 500。
pub fn parse_request(buffer: &[u8]) -> std::io::Result<Option<Request>> {
    let Some(header_end) = find_header_end(buffer) else {
        if buffer.len() > MAX_HEADER_BYTES {
            return Ok(Some(malformed()));
        }
        return Ok(None);
    };
    if header_end > MAX_HEADER_BYTES {
        return Ok(Some(malformed()));
    }
    let header_text = String::from_utf8_lossy(&buffer[..header_end]).into_owned();
    let mut lines = header_text.split("\r\n");
    let Some(request_line) = lines.next() else {
        return Ok(Some(malformed()));
    };
    let mut parts = request_line.split_whitespace();
    let (Some(method), Some(target)) = (parts.next(), parts.next()) else {
        return Ok(Some(malformed()));
    };
    let (path, query) = split_target(target);
    let mut headers = HashMap::new();
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_string());
        }
    }
    let content_length = headers
        .get("content-length")
        .and_then(|value| value.trim().parse::<usize>().ok())
        .unwrap_or(0);
    if content_length > MAX_BODY_BYTES {
        let mut request = malformed();
        request.body = String::new();
        return Ok(Some(request_with_status(request, 413)));
    }
    let body_start = header_end + 4;
    if buffer.len() < body_start + content_length {
        return Ok(None);
    }
    let body =
        String::from_utf8_lossy(&buffer[body_start..body_start + content_length]).into_owned();
    let _ = MAX_HEADER_BYTES;
    Ok(Some(Request {
        method: method.to_string(),
        path,
        query,
        headers,
        body,
    }))
}

/// 无法解析的请求占位：路由层统一回 400。
fn malformed() -> Request {
    Request {
        method: String::new(),
        path: String::new(),
        query: HashMap::new(),
        headers: HashMap::new(),
        body: String::new(),
    }
}

/// 把错误状态写进占位请求（利用空 method 无法匹配路由的特性，由 status 字段承载）。
fn request_with_status(mut request: Request, status: u16) -> Request {
    request.method = format!("#{status}");
    request
}

/// 拆分 `target` 为路径与 query。
fn split_target(target: &str) -> (String, HashMap<String, String>) {
    let (path, raw_query) = target.split_once('?').unwrap_or((target, ""));
    let mut query = HashMap::new();
    for pair in raw_query.split('&') {
        if pair.is_empty() {
            continue;
        }
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        query.insert(percent_decode(key), percent_decode(value));
    }
    (percent_decode(path), query)
}

/// `%XX` 解码 + `+` 视为空格；非法序列原样保留。
fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'%' if index + 2 < bytes.len() => {
                let hex = |byte: u8| (byte as char).to_digit(16);
                if let (Some(high), Some(low)) = (hex(bytes[index + 1]), hex(bytes[index + 2])) {
                    out.push((high * 16 + low) as u8);
                    index += 3;
                    continue;
                }
                out.push(bytes[index]);
                index += 1;
            }
            b'+' => {
                out.push(b' ');
                index += 1;
            }
            byte => {
                out.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// header 段结束偏移（`\r\n\r\n` 起点）。
fn find_header_end(buffer: &[u8]) -> Option<usize> {
    buffer.windows(4).position(|window| window == b"\r\n\r\n")
}

/// 路由分派；所有写操作要求 token + 同源。
pub fn route(request: &Request, state: &ServiceState) -> Response {
    if request.method.is_empty() {
        return Response::error(400, "malformed request");
    }
    if let Some(status) = request.method.strip_prefix('#') {
        let status = status.parse().unwrap_or(400);
        return Response::error(status, "request too large");
    }
    // 静态资源（CSS/JS）不含敏感内容，且浏览器以无 token 的裸路径请求，
    // 故在 token 校验之前放行；页面与 JSON API 仍逐请求校验。
    if request.method == "GET"
        && let Some((content_type, body)) = page::asset(&request.path)
    {
        return Response {
            status: 200,
            content_type,
            body: body.to_string(),
        };
    }
    if !token_ok(request, state) {
        return Response::error(403, "invalid token");
    }
    let write = !matches!(request.method.as_str(), "GET" | "HEAD");
    if write && !origin_ok(request, state) {
        return Response::error(403, "cross-origin write rejected");
    }
    match (request.method.as_str(), request.path.as_str()) {
        ("GET", "/") => Response {
            status: 200,
            content_type: "text/html; charset=utf-8",
            body: page::html().to_string(),
        },
        ("GET", "/api/state") => state_json(state),
        ("PUT", "/api/config") => put_config(request, state),
        ("PUT", path) | ("DELETE", path) | ("POST", path) if path.starts_with("/api/commands/") => {
            command_request(request, path, state)
        }
        ("GET", "/api/commands") => commands_json(state),
        _ => Response::error(404, "not found"),
    }
}

/// query token 校验（常量时间比较，避免计时旁路）。
fn token_ok(request: &Request, state: &ServiceState) -> bool {
    let Some(token) = request.query.get("token") else {
        return false;
    };
    let expected = state.token().as_bytes();
    let provided = token.as_bytes();
    if expected.len() != provided.len() {
        return false;
    }
    expected
        .iter()
        .zip(provided)
        .fold(0_u8, |acc, (left, right)| acc | (left ^ right))
        == 0
}

/// 同源校验：Origin 缺失（curl 场景）放行；存在时必须落在回环主机上。
///
/// 服务只监听回环地址，任意请求本身都来自本机；此检查用于阻止浏览器里
/// 其它网页发起的跨站写入（其 Origin 是外部域名）。
fn origin_ok(request: &Request, _state: &ServiceState) -> bool {
    let Some(origin) = request.headers.get("origin") else {
        return true;
    };
    if origin == "null" {
        return true;
    }
    let host = origin
        .split_once("://")
        .map(|(_, rest)| rest)
        .unwrap_or(origin);
    let host = host.split(['/', ':']).next().unwrap_or("");
    matches!(host, "127.0.0.1" | "localhost" | "[::1]" | "::1")
}

/// `GET /api/state`：当前配置 + 命令快照。
fn state_json(state: &ServiceState) -> Response {
    let config = crate::config::load_or_default(state.config_path()).unwrap_or_default();
    let commands = state.commands();
    Response::json(
        200,
        format!(
            "{{\"config\":{},\"commands\":[{}],\"project_available\":false}}",
            config_json(&config),
            commands
                .iter()
                .map(command_view_json)
                .collect::<Vec<_>>()
                .join(",")
        ),
    )
}

/// `GET /api/commands`：仅命令快照。
fn commands_json(state: &ServiceState) -> Response {
    let commands = state.commands();
    Response::json(
        200,
        format!(
            "[{}]",
            commands
                .iter()
                .map(command_view_json)
                .collect::<Vec<_>>()
                .join(",")
        ),
    )
}

/// `PUT /api/config`：校验并原子保存；成功投递事件。
fn put_config(request: &Request, state: &ServiceState) -> Response {
    let Some(config_entries) = json_object_field(&request.body, "config") else {
        return Response::error(422, "body must contain a config object");
    };
    let config = match config_from_json(config_entries) {
        Ok(config) => config,
        Err(message) => return Response::error(422, &message),
    };
    if let Err(error) = crate::config::save(state.config_path(), &config) {
        return Response::error(422, &error.to_string());
    }
    let _ = state.sender().send(SettingsEvent::ConfigChanged(config));
    Response::json(200, "{\"ok\":true}")
}

/// 解析 8 个已知键；未知键忽略，非法值报错。
fn config_from_json(entries: Vec<(String, JsonValue)>) -> Result<Config, String> {
    let mut config = Config::default();
    for (key, value) in entries {
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
        let number = value
            .as_number()
            .ok_or_else(|| format!("{key} must be an integer"))?;
        *slot = u16::try_from(number).map_err(|_| format!("{key} is out of range"))?;
    }
    config.validate().map_err(|error| error.to_string())?;
    Ok(config)
}

/// 命令路径请求：`/api/commands/<scope>/<kind>/<name>`（重命名追加 `/rename`）。
fn command_request(request: &Request, path: &str, state: &ServiceState) -> Response {
    let rest = path.trim_start_matches("/api/commands/");
    let mut segments: Vec<&str> = rest.split('/').collect();
    let rename = match segments.last() {
        Some(last) if *last == "rename" => {
            segments.pop();
            true
        }
        _ => false,
    };
    if segments.len() != 3 {
        return Response::error(404, "invalid command path");
    }
    let (scope, kind, name) = (segments[0], segments[1], segments[2]);
    let Some(scope) = CommandScope::parse(scope) else {
        return Response::error(400, "invalid scope");
    };
    let Some(kind) = CommandKind::parse(kind) else {
        return Response::error(400, "invalid kind");
    };
    if scope == CommandScope::Project {
        return Response::error(
            403,
            "project commands are edited from the workspace scope only",
        );
    }
    if commands::validate_name(name).is_err() {
        return Response::error(400, "invalid command name");
    }
    let root = match state.user_root() {
        Some(root) => root,
        None => {
            let Ok(root) = crate::config::user_data_root() else {
                return Response::error(500, "home directory unavailable");
            };
            root
        }
    };
    let path_on_disk = commands::commands_dir(scope, &root, kind).join(format!("{name}.md"));
    match (request.method.as_str(), rename) {
        ("PUT", false) => {
            let response = put_command(request, scope, &root, kind, &path_on_disk);
            if response.status == 200 {
                after_command_change(state);
            }
            response
        }
        ("DELETE", false) => {
            if commands::delete(&path_on_disk).is_err() {
                return Response::error(404, "command not found");
            }
            after_command_change(state);
            Response::json(200, "{\"ok\":true}")
        }
        ("POST", true) => {
            let response = rename_command(request, &path_on_disk);
            if response.status == 200 {
                after_command_change(state);
            }
            response
        }
        _ => Response::error(405, "method not allowed"),
    }
}

/// 命令文件变更后：立刻按磁盘重建用户级快照（`/api/state` 无需等待 TUI 推送），
/// 并投递事件让 TUI 重载自定义命令。
fn after_command_change(state: &ServiceState) {
    refresh_user_snapshot(state);
    let _ = state.sender().send(SettingsEvent::CommandsChanged);
}

/// 从用户数据根重读 User 作用域命令并替换快照中的 User 部分；Project 部分由 TUI 提供。
fn refresh_user_snapshot(state: &ServiceState) {
    let root = match state.user_root() {
        Some(root) => root,
        None => match crate::config::user_data_root() {
            Ok(root) => root,
            Err(_) => return,
        },
    };
    let (records, _errors) = commands::list(CommandScope::User, &root);
    let user: Vec<CommandView> = records
        .iter()
        .map(|record| CommandView::of(record, CommandScope::User))
        .collect();
    state.replace_user_commands(user);
}

/// 保存（覆盖）命令文件。
fn put_command(
    request: &Request,
    scope: CommandScope,
    root: &std::path::Path,
    kind: CommandKind,
    path_on_disk: &std::path::Path,
) -> Response {
    let Some(blocks) = json_object_pairs(&request.body) else {
        return Response::error(422, "invalid command json");
    };
    let mut description = String::new();
    let mut md_scope = MdScope::Global;
    let mut title = None;
    let mut argument_hint = None;
    let mut body = None;
    let mut block_type = BlockType::Template;
    for (key, value) in blocks {
        match key.as_str() {
            "description" => description = value.as_string().unwrap_or_default(),
            "block_type" => {
                let Some(parsed) = value.as_string().and_then(|value| BlockType::parse(&value))
                else {
                    return Response::error(422, "invalid block_type");
                };
                block_type = parsed;
            }
            "md_scope" => {
                let Some(parsed) = value.as_string().and_then(|value| MdScope::parse(&value))
                else {
                    return Response::error(422, "invalid md_scope");
                };
                md_scope = parsed;
            }
            "title" => title = value.as_string().filter(|value| !value.is_empty()),
            "argument_hint" => {
                argument_hint = value.as_string().filter(|value| !value.is_empty());
            }
            "body" => body = value.as_string(),
            _ => {}
        }
    }
    let Some(body) = body.filter(|body| !body.trim().is_empty()) else {
        return Response::error(422, "body must not be empty");
    };
    if kind == CommandKind::AgentBlock && md_scope == MdScope::Global {
        return Response::error(422, "agentBlock requires template scope");
    }
    let name = path_on_disk
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_default();
    let record = CommandRecord {
        name,
        scope: if kind == CommandKind::AgentBlock {
            MdScope::Template
        } else {
            md_scope
        },
        description,
        argument_hint,
        title,
        body,
        kind,
        block: block_type,
        path: path_on_disk.to_path_buf(),
    };
    if let Err(error) = commands::save(scope, root, &record, true) {
        return Response::error(422, &error.to_string());
    }
    Response::json(200, "{\"ok\":true}")
}

/// 重命名命令文件。
fn rename_command(request: &Request, path_on_disk: &std::path::Path) -> Response {
    let Some(blocks) = json_object_pairs(&request.body) else {
        return Response::error(422, "invalid json");
    };
    let Some(new_name) = blocks
        .iter()
        .find(|(key, _)| key == "to")
        .and_then(|(_, value)| value.as_string())
    else {
        return Response::error(422, "missing `to`");
    };
    match commands::rename(path_on_disk, &new_name) {
        Ok(_) => Response::json(200, "{\"ok\":true}"),
        Err(error) => Response::error(422, &error.to_string()),
    }
}

/// 命令视图 → JSON。
fn command_view_json(view: &CommandView) -> String {
    format!(
        "{{\"name\":{},\"scope\":{},\"kind\":{},\"description\":{},\"md_scope\":{},\"title\":{},\"block_type\":{},\"body\":{}}}",
        json_string(&view.name),
        json_string(&view.scope),
        json_string(&view.kind),
        json_string(&view.description),
        json_string(&view.md_scope),
        json_string(&view.title),
        json_string(&view.block_type),
        json_string(&view.body),
    )
}

/// 配置 → JSON（键序固定）。
fn config_json(config: &Config) -> String {
    format!(
        "{{\"sidebar_width\":{},\"sidebar_min_width\":{},\"sidebar_max_width\":{},\"narrow_width\":{},\"min_width\":{},\"min_height\":{},\"min_pane_width\":{},\"min_pane_height\":{}}}",
        config.sidebar_width,
        config.sidebar_min_width,
        config.sidebar_max_width,
        config.narrow_width,
        config.min_width,
        config.min_height,
        config.min_pane_width,
        config.min_pane_height,
    )
}

/// JSON 字符串转义（含控制字符）。
pub fn json_string(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for ch in text.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            ch if (ch as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", ch as u32)),
            ch => out.push(ch),
        }
    }
    out.push('"');
    out
}

/// 极简 JSON 值：数字、字符串、对象与其它（数组/布尔/空）。
#[derive(Debug, Clone, PartialEq)]
pub enum JsonValue {
    Number(u64),
    String(String),
    Object(Vec<(String, JsonValue)>),
    Other,
}

impl JsonValue {
    fn as_number(&self) -> Option<u64> {
        match self {
            Self::Number(number) => Some(*number),
            _ => None,
        }
    }

    fn as_string(&self) -> Option<String> {
        match self {
            Self::String(text) => Some(text.clone()),
            _ => None,
        }
    }
}

/// 解析顶层对象为键值对；格式非法返回 None。
pub fn json_object_pairs(text: &str) -> Option<Vec<(String, JsonValue)>> {
    let mut parser = JsonParser::new(text);
    let entries = parser.object().ok()?;
    parser.finish().ok()?;
    Some(entries)
}

/// 取顶层对象的某个嵌套对象字段；供 `config` 字段提取。
fn json_object_field(text: &str, field: &str) -> Option<Vec<(String, JsonValue)>> {
    let mut parser = JsonParser::new(text);
    let entries = parser.object().ok()?;
    for (key, value) in entries {
        if key == field {
            return match value {
                JsonValue::Object(entries) => Some(entries),
                _ => None,
            };
        }
    }
    None
}

/// 极简 JSON 解析器：支持对象、数组、字符串（含转义）、整数、布尔与 null。
pub struct JsonParser<'a> {
    bytes: &'a [u8],
    index: usize,
}

impl<'a> JsonParser<'a> {
    pub fn new(text: &'a str) -> Self {
        Self {
            bytes: text.as_bytes(),
            index: 0,
        }
    }

    /// 解析顶层对象。
    pub fn object(&mut self) -> Result<Vec<(String, JsonValue)>, ()> {
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
    pub fn value(&mut self) -> Result<JsonValue, ()> {
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
                while self.peek().is_some_and(|byte| byte.is_ascii_alphanumeric()) {
                    self.index += 1;
                }
                Ok(JsonValue::Other)
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
    pub fn finish(&mut self) -> Result<(), ()> {
        self.skip_whitespace();
        if self.index == self.bytes.len() {
            Ok(())
        } else {
            Err(())
        }
    }
}
