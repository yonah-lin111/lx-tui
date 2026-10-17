//! Schema 测试：四工具名、严格对象与字段白名单。

use super::*;

#[test]
fn tool_order_and_names_are_fixed() {
    let names: Vec<&str> = available_tools().iter().map(|spec| spec.name).collect();
    assert_eq!(names, vec!["read", "search", "edit", "write"]);
}

#[test]
fn all_schemas_reject_additional_properties() {
    for spec in available_tools() {
        let parameters = spec.parameters();
        assert_eq!(parameters["additionalProperties"], false, "{}", spec.name);
        assert_eq!(parameters["type"], "object", "{}", spec.name);
    }
}

#[test]
fn read_schema_documents_one_based_offset() {
    let read = available_tools()
        .iter()
        .find(|spec| spec.name == "read")
        .expect("read");
    let parameters = read.parameters();
    assert_eq!(parameters["properties"]["offset"]["minimum"], 1);
    let description = parameters["properties"]["offset"]["description"]
        .as_str()
        .expect("description");
    assert!(description.contains("1-based"));
    assert_eq!(parameters["required"], serde_json::json!(["path"]));
}

#[test]
fn edit_schema_has_only_patch() {
    let edit = available_tools()
        .iter()
        .find(|spec| spec.name == "edit")
        .expect("edit");
    let parameters = edit.parameters();
    let properties = parameters["properties"].as_object().expect("properties");
    assert_eq!(properties.len(), 1);
    assert!(properties.contains_key("patch"));
    assert_eq!(parameters["required"], serde_json::json!(["patch"]));
}

#[test]
fn search_schema_optional_fields() {
    let search = available_tools()
        .iter()
        .find(|spec| spec.name == "search")
        .expect("search");
    let parameters = search.parameters();
    assert_eq!(parameters["required"], serde_json::json!(["pattern"]));
    for field in ["path", "include", "literal", "case_sensitive", "limit"] {
        assert!(
            parameters["properties"].get(field).is_some(),
            "search schema missing {field}"
        );
    }
}

#[test]
fn write_schema_requires_path_and_content() {
    let write = available_tools()
        .iter()
        .find(|spec| spec.name == "write")
        .expect("write");
    let parameters = write.parameters();
    assert_eq!(
        parameters["required"],
        serde_json::json!(["path", "content"])
    );
}
