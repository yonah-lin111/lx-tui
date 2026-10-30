use super::*;

#[test]
fn sanitizes_and_prefixes() {
    assert_eq!(
        qualified_name("github", "create_issue", &HashSet::new()),
        "mcp__github__create_issue"
    );
    assert_eq!(
        qualified_name("my-kb", "search.docs v2", &HashSet::new()),
        "mcp__my-kb__search_docs_v2"
    );
}

#[test]
fn long_names_fit_64_chars_and_stay_unique() {
    let taken = HashSet::new();
    let name = qualified_name(
        "company-internal-knowledge-base",
        "search_documents_by_semantic_similarity",
        &taken,
    );
    assert_eq!(name.len(), MAX_TOOL_NAME_LEN);
    assert!(name.starts_with("mcp__company-internal-knowledge-base__"));
    let mut taken = HashSet::new();
    taken.insert(name.clone());
    let other = qualified_name(
        "company-internal-knowledge-base",
        "search_documents_by_semantic_similarity",
        &taken,
    );
    assert_ne!(other, name);
    assert_eq!(other.len(), MAX_TOOL_NAME_LEN);
}

#[test]
fn splits_qualified_names() {
    assert_eq!(
        split_qualified("mcp__github__create_issue"),
        Some(("github", "create_issue"))
    );
    assert_eq!(split_qualified("read"), None);
    assert_eq!(split_qualified("mcp____x"), None);
}
