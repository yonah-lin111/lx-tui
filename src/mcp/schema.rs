//! JSON Schema 清洗与压缩：让第三方 MCP 工具 schema 能被主流供应商接受。

use serde_json::{Map, Value, json};

/// 参数 schema 的字节上限（超过后逐级压缩）。
pub const MAX_SCHEMA_BYTES: usize = 5000;
/// 描述末尾的省略提示。
pub const SCHEMA_OMITTED_NOTE: &str =
    " (Input schema omitted because it was too large; pass a JSON object.)";

/// 清洗 schema 并保证不超过 5000 字节。
///
/// 返回 `(schema, omitted)`；`omitted` 为 true 时调用方需在描述末尾追加
/// [`SCHEMA_OMITTED_NOTE`]。
pub fn sanitize_json_schema(schema: &Value) -> (Value, bool) {
    let mut cleaned = clean(schema.clone());
    if !cleaned.is_object() {
        cleaned = json!({"type": "object", "properties": {}});
    }
    let object = cleaned.as_object_mut();
    if let Some(object) = object {
        object.entry("type").or_insert_with(|| json!("object"));
        if object.get("type") == Some(&json!("object")) {
            object.entry("properties").or_insert_with(|| json!({}));
        }
    }
    if serialized_len(&cleaned) <= MAX_SCHEMA_BYTES {
        return (cleaned, false);
    }
    strip_descriptions(&mut cleaned);
    if serialized_len(&cleaned) <= MAX_SCHEMA_BYTES {
        return (cleaned, false);
    }
    strip_defs(&mut cleaned);
    if serialized_len(&cleaned) <= MAX_SCHEMA_BYTES {
        return (cleaned, false);
    }
    truncate_depth(&mut cleaned, 3);
    if serialized_len(&cleaned) <= MAX_SCHEMA_BYTES {
        return (cleaned, false);
    }
    (json!({"type": "object", "properties": {}}), true)
}

/// Gemini 只接受 schema 子集：只保留 type/description/properties/required/items/enum/format/nullable。
pub fn trim_for_gemini(schema: &Value) -> Value {
    match schema {
        Value::Object(map) => {
            let mut out = Map::new();
            for key in [
                "type",
                "description",
                "properties",
                "required",
                "items",
                "enum",
                "format",
                "nullable",
            ] {
                if let Some(value) = map.get(key) {
                    out.insert(
                        key.to_string(),
                        match (key, value) {
                            ("properties", Value::Object(properties)) => Value::Object(
                                properties
                                    .iter()
                                    .map(|(name, child)| (name.clone(), trim_for_gemini(child)))
                                    .collect(),
                            ),
                            ("items", _) => trim_for_gemini(value),
                            _ => value.clone(),
                        },
                    );
                }
            }
            Value::Object(out)
        }
        Value::Bool(true) => json!({"type": "string"}),
        Value::Bool(false) => json!({"type": "string"}),
        other => other.clone(),
    }
}

fn clean(value: Value) -> Value {
    match value {
        Value::Bool(true) | Value::Bool(false) => json!({"type": "string"}),
        Value::Array(items) => Value::Array(items.into_iter().map(clean).collect()),
        Value::Object(mut map) => {
            // const → enum
            if let Some(constant) = map.remove("const") {
                map.insert("enum".into(), Value::Array(vec![constant]));
            }
            for key in [
                "properties",
                "items",
                "additionalProperties",
                "prefixItems",
                "anyOf",
                "oneOf",
                "allOf",
                "$defs",
                "definitions",
            ] {
                if let Some(child) = map.get_mut(key).map(Value::take) {
                    let cleaned = match key {
                        "properties" | "$defs" | "definitions" => match child {
                            Value::Object(entries) => Value::Object(
                                entries
                                    .into_iter()
                                    .map(|(name, child)| (name, clean(child)))
                                    .collect(),
                            ),
                            other => clean(other),
                        },
                        _ => clean(child),
                    };
                    map.insert(key.into(), cleaned);
                }
            }
            if !map.contains_key("type") {
                let inferred = if map.contains_key("properties")
                    || map.contains_key("required")
                    || map.contains_key("additionalProperties")
                {
                    Some("object")
                } else if map.contains_key("items") {
                    Some("array")
                } else if map.contains_key("enum") || map.contains_key("format") {
                    Some("string")
                } else if map.contains_key("minimum")
                    || map.contains_key("maximum")
                    || map.contains_key("multipleOf")
                {
                    Some("number")
                } else {
                    None
                };
                if let Some(inferred) = inferred {
                    map.insert("type".into(), json!(inferred));
                }
            }
            match map.get("type").and_then(Value::as_str) {
                Some("object") if !map.contains_key("properties") => {
                    map.insert("properties".into(), json!({}));
                }
                Some("array") if !map.contains_key("items") => {
                    map.insert("items".into(), json!({"type": "string"}));
                }
                _ => {}
            }
            Value::Object(map)
        }
        other => other,
    }
}

fn serialized_len(value: &Value) -> usize {
    serde_json::to_string(value).map_or(usize::MAX, |text| text.len())
}

fn walk_objects(value: &mut Value, apply: &mut impl FnMut(&mut Map<String, Value>)) {
    match value {
        Value::Object(map) => {
            apply(map);
            for child in map.values_mut() {
                walk_objects(child, apply);
            }
        }
        Value::Array(items) => {
            for item in items {
                walk_objects(item, apply);
            }
        }
        _ => {}
    }
}

fn strip_descriptions(value: &mut Value) {
    walk_objects(value, &mut |map| {
        map.remove("description");
        map.remove("title");
    });
}

fn strip_defs(value: &mut Value) {
    walk_objects(value, &mut |map| {
        map.remove("$defs");
        map.remove("definitions");
    });
    walk_objects(value, &mut |map| {
        if map.contains_key("$ref") {
            *map = Map::from_iter([("type".to_string(), json!("object"))]);
        }
    });
}

fn truncate_depth(value: &mut Value, max_depth: usize) {
    fn inner(value: &mut Value, depth: usize, max_depth: usize) {
        match value {
            Value::Object(map) => {
                if depth >= max_depth {
                    if map.get("type").and_then(Value::as_str) == Some("object") {
                        map.remove("properties");
                    }
                    return;
                }
                for child in map.values_mut() {
                    inner(child, depth + 1, max_depth);
                }
            }
            Value::Array(items) => {
                for item in items {
                    inner(item, depth + 1, max_depth);
                }
            }
            _ => {}
        }
    }
    inner(value, 0, max_depth);
}

#[cfg(test)]
mod tests;
