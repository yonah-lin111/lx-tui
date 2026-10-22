use super::*;

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "lx-git-snapshot-{name}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

fn git(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .expect("spawn git");
    assert!(status.success(), "git {args:?} failed");
}

#[test]
fn blobs_roundtrip_and_identity_is_per_worktree() {
    let dir = temp_dir("roundtrip");
    git(&dir, &["init", "-q"]);
    let file = dir.join("a.txt");
    std::fs::write(&file, b"x").expect("seed");
    let identity = identify(&file).expect("identity");
    assert_eq!(
        identity.root,
        std::fs::canonicalize(&dir).expect("canonical")
    );
    let oid = write_blob(&identity, b"hello\0world").expect("blob");
    let bytes = read_blob(&identity, &oid).expect("read blob");
    assert_eq!(bytes, b"hello\0world");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn non_git_directory_has_no_identity() {
    let dir = temp_dir("plain");
    assert!(identify(&dir).is_none());
    let _ = std::fs::remove_dir_all(dir);
}
