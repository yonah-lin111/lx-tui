//! 日志脱敏：URL、请求头、错误正文与内联图片。
//!
//! 原则：结构信息可以保留，密钥、查询参数、prompt/文件内容不能落盘。

/// 请求头白名单：只有这些键保留值，其余只保留键名并标记 `[redacted]`。
const HEADER_WHITELIST: [&str; 8] = [
    "accept",
    "anthropic-beta",
    "anthropic-version",
    "content-type",
    "openai-beta",
    "user-agent",
    "x-goog-api-client",
    "x-stainless-lang",
];

/// URL 只保留 `scheme://host[:port]`。
pub fn url_origin(url: &str) -> String {
    split_url(url)
        .map(|(origin, _path)| origin)
        .unwrap_or_else(|| "<invalid-url>".to_string())
}

/// URL 只保留 `scheme://host[:port]/path`，丢弃 userinfo、query 与 fragment。
pub fn url_origin_and_path(url: &str) -> String {
    split_url(url)
        .map(|(origin, path)| {
            if path.is_empty() {
                origin
            } else {
                format!("{origin}{path}")
            }
        })
        .unwrap_or_else(|| "<invalid-url>".to_string())
}

fn split_url(url: &str) -> Option<(String, String)> {
    let (scheme, rest) = url.split_once("://")?;
    if scheme.is_empty() {
        return None;
    }
    let rest = rest.split(['?', '#']).next().unwrap_or("");
    let (authority, path) = match rest.find('/') {
        Some(index) => (&rest[..index], &rest[index..]),
        None => (rest, ""),
    };
    if authority.is_empty() {
        return None;
    }
    // 去掉 userinfo：`user:pass@host` → `host`。
    let authority = authority
        .rsplit_once('@')
        .map_or(authority, |(_, host)| host);
    if authority.is_empty() {
        return None;
    }
    Some((format!("{scheme}://{authority}"), path.to_string()))
}

/// 请求头脱敏：白名单保留值，其余值替换为 `[redacted]`。
pub fn redacted_headers(headers: &[(String, String)]) -> Vec<(String, String)> {
    headers
        .iter()
        .map(|(name, value)| {
            let lower = name.to_ascii_lowercase();
            if HEADER_WHITELIST.contains(&lower.as_str()) {
                (lower, value.clone())
            } else {
                (lower, "[redacted]".to_string())
            }
        })
        .collect()
}

/// 单行化、去控制字符并按 UTF-8 边界截断。
pub fn sanitize_text(text: &str, max_bytes: usize) -> String {
    let mut cleaned = String::with_capacity(text.len().min(max_bytes.saturating_add(16)));
    let mut last_space = true;
    let mut truncated = false;
    for ch in text.chars() {
        if ch.is_control() && ch != '\n' && ch != '\t' {
            continue;
        }
        let ch = if ch == '\n' || ch == '\t' { ' ' } else { ch };
        if ch == ' ' {
            if last_space {
                continue;
            }
            last_space = true;
        } else {
            last_space = false;
        }
        if cleaned.len() + ch.len_utf8() > max_bytes {
            truncated = true;
            break;
        }
        cleaned.push(ch);
    }
    if truncated {
        return format!("{}…[truncated]", cleaned.trim_end());
    }
    cleaned.trim().to_string()
}

/// 截断到不超过 `max_bytes` 的 UTF-8 边界。
pub fn truncate_utf8(text: &str, max_bytes: usize) -> &str {
    if text.len() <= max_bytes {
        return text;
    }
    let mut end = max_bytes;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

/// 把请求体 JSON 中的图片 base64 替换为体积占位，同时保留 JSON 原始缩进与多空格。
///
/// 解析失败时回退为按 UTF-8 边界截断，不做猜测性替换。
pub fn omit_inline_images(body_json: &str, max_bytes: usize) -> String {
    if !may_contain_inline_image(body_json) {
        return truncate_json_text(body_json, max_bytes);
    }
    let Ok(mut value) = serde_json::from_str::<serde_json::Value>(body_json) else {
        return sanitize_text(body_json, max_bytes);
    };
    omit_images_in_value(&mut value);
    let text = serde_json::to_string(&value).unwrap_or_else(|_| "[body omitted]".to_string());
    truncate_json_text(&text, max_bytes)
}

fn may_contain_inline_image(body_json: &str) -> bool {
    body_json.contains("data:image/")
        || body_json.contains("\"base64\"")
        || body_json.contains("\"inlineData\"")
        || body_json.contains("\"inline_data\"")
        || body_json.contains("\"image\"")
        || body_json.contains("\"input_image\"")
}

fn truncate_json_text(text: &str, max_bytes: usize) -> String {
    if text.len() <= max_bytes {
        text.to_string()
    } else {
        format!("{}…[truncated]", truncate_utf8(text, max_bytes))
    }
}

fn omit_images_in_value(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::String(text)
            if text.starts_with("data:image/") && text.contains(";base64,") =>
        {
            *text = format!("[image omitted: {} bytes]", text.len());
        }
        serde_json::Value::Array(items) => {
            for item in items {
                omit_images_in_value(item);
            }
        }
        serde_json::Value::Object(object) => {
            let is_typed_image = object
                .get("type")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|kind| matches!(kind, "base64" | "image" | "input_image"));
            let has_mime_type = ["mimeType", "mime_type", "media_type"].iter().any(|key| {
                object
                    .get(*key)
                    .and_then(serde_json::Value::as_str)
                    .is_some()
            });
            if (is_typed_image || has_mime_type)
                && let Some(serde_json::Value::String(data)) = object.get_mut("data")
            {
                *data = format!("[image omitted: {} bytes]", data.len());
            }
            for value in object.values_mut() {
                omit_images_in_value(value);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests;
