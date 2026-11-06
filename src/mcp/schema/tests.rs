use super::*;

#[test]
fn infers_missing_types_and_fills_properties() {
    let (schema, omitted) = sanitize_json_schema(&json!({
        "required": ["q"],
        "properties": {"q": {"format": "uri"}}
    }));
    assert!(!omitted);
    assert_eq!(schema["type"], "object");
    assert_eq!(schema["properties"]["q"]["type"], "string");
}

#[test]
fn converts_const_and_boolean_schemas() {
    let (schema, _) = sanitize_json_schema(&json!({
        "type": "object",
        "properties": {
            "kind": {"const": "issue"},
            "anything": true,
            "list": {"type": "array"}
        }
    }));
    assert_eq!(schema["properties"]["kind"]["enum"][0], "issue");
    assert_eq!(schema["properties"]["anything"]["type"], "string");
    assert_eq!(schema["properties"]["list"]["items"]["type"], "string");
}

#[test]
fn oversized_schemas_are_compressed() {
    let mut properties = Map::new();
    for index in 0..300 {
        properties.insert(
            format!("field_{index}"),
            json!({"type": "string", "description": "d".repeat(80)}),
        );
    }
    let schema = json!({"type": "object", "properties": properties});
    let (cleaned, omitted) = sanitize_json_schema(&schema);
    assert!(omitted, "超限 schema 必须降级");
    assert_eq!(cleaned, json!({"type": "object", "properties": {}}));
}

#[test]
fn gemini_trims_unknown_keywords() {
    let trimmed = trim_for_gemini(&json!({
        "type": "object",
        "additionalProperties": false,
        "$ref": "#/x",
        "properties": {
            "q": {"type": "string", "minLength": 1},
            "opt": {"anyOf": [{"type": "integer"}, {"type": "null"}]},
            "free_map": {"type": "object", "additionalProperties": {"type": "string"}}
        }
    }));
    assert!(trimmed.get("additionalProperties").is_none());
    assert!(trimmed.get("$ref").is_none());
    assert_eq!(trimmed["properties"]["q"]["type"], "string");
    assert_eq!(trimmed["properties"]["opt"]["type"], "integer");
    assert_eq!(trimmed["properties"]["opt"]["nullable"], true);
    assert_eq!(trimmed["properties"]["free_map"]["properties"], json!({}));
}
