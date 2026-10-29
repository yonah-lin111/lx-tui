use super::*;

fn temp_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "lx-skills-store-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default()
    ));
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

#[test]
fn write_read_delete_round_trip() {
    let root = temp_dir("roundtrip");
    let path = write_skill(&root, "pdf", "Read PDF files.", "# PDF\nbody\n").expect("write");
    assert!(path.ends_with("pdf/SKILL.md"));
    let (name, description, body) = read_skill(&root, "pdf").expect("read");
    assert_eq!(name, "pdf");
    assert_eq!(description, "Read PDF files.");
    assert!(body.contains("# PDF"));
    delete_skill(&root, "pdf").expect("delete");
    assert!(!root.join("pdf").exists());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn delete_refuses_a_directory_with_other_files() {
    let root = temp_dir("other");
    write_skill(&root, "pdf", "Read PDF files.", "body").expect("write");
    std::fs::write(root.join("pdf/scripts.sh"), "echo hi").expect("extra file");
    assert!(delete_skill(&root, "pdf").is_err());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn serialize_rejects_empty_description_and_invalid_name() {
    assert!(serialize_skill("pdf", "", "body").is_err());
    assert!(serialize_skill("bad name", "d", "body").is_err());
}
