use super::*;

#[test]
fn parses_frontmatter_and_defaults_name_to_directory() {
    let text = "---\ndescription: Steps to cut a release.\n---\n\n# Release\n";
    let parsed = parse_skill(text, "release-checklist").expect("parse");
    assert_eq!(parsed.name, "release-checklist");
    assert_eq!(parsed.description, "Steps to cut a release.");
}

#[test]
fn name_can_be_explicit_and_values_may_contain_colons() {
    let text = "---\nname: pdf\ndescription: Build for AWS: ECS in one line.\n---\nbody";
    let parsed = parse_skill(text, "whatever").expect("parse");
    assert_eq!(parsed.name, "pdf");
    assert_eq!(parsed.description, "Build for AWS: ECS in one line.");
}

#[test]
fn folded_and_literal_blocks_are_supported() {
    let folded = "---\ndescription: >\n  Read, fill and merge\n  PDF files.\n---\n";
    assert_eq!(
        parse_skill(folded, "pdf").expect("folded").description,
        "Read, fill and merge PDF files."
    );
    let literal = "---\ndescription: |\n  Line one.\n  Line two.\n---\n";
    assert_eq!(
        parse_skill(literal, "pdf").expect("literal").description,
        "Line one. Line two."
    );
}

#[test]
fn missing_frontmatter_or_description_is_an_error() {
    assert_eq!(
        parse_skill("# no frontmatter", "x").unwrap_err(),
        "missing YAML frontmatter delimited by ---"
    );
    assert_eq!(
        parse_skill("---\nname: x\n---\n", "x").unwrap_err(),
        "missing field `description`"
    );
}

#[test]
fn invalid_names_are_rejected() {
    assert!(validate_name("pdf").is_ok());
    assert!(validate_name("my-skill_1.2").is_ok());
    assert!(validate_name("").is_err());
    assert!(validate_name("-bad").is_err());
    assert!(validate_name("bad name").is_err());
    assert!(validate_name(&"x".repeat(65)).is_err());
}

#[test]
fn oversized_skill_is_rejected() {
    let text = format!(
        "---\ndescription: x\n---\n{}",
        "a".repeat(MAX_SKILL_FILE_BYTES)
    );
    assert_eq!(
        parse_skill(&text, "big").unwrap_err(),
        "SKILL.md exceeds 256 KiB"
    );
}
