use super::*;
use std::ffi::OsString;
use std::os::unix::ffi::OsStringExt;

#[test]
fn utf8_paths_roundtrip_unchanged() {
    let path = Path::new("/tmp/目录/a.txt");
    let encoded = encode_path(path);
    assert_eq!(encoded, "/tmp/目录/a.txt");
    assert_eq!(decode_path(&encoded), path);
}

#[test]
fn non_utf8_paths_roundtrip_without_loss() {
    let path = PathBuf::from(OsString::from_vec(b"/tmp/bad-\xff\xfe.txt".to_vec()));
    let encoded = encode_path(&path);
    assert!(encoded.starts_with(TAG), "必须带标记：{encoded}");
    assert_eq!(decode_path(&encoded), path);
}

fn temp(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "lx-path-chain-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

#[test]
fn chain_accepts_real_directories() {
    let root = temp("ok");
    std::fs::create_dir_all(root.join("sub")).expect("dirs");
    let target = root.join("sub/a.txt");
    let (anchor, canonical) = path_anchor(&target).expect("anchor");
    assert!(verify_path_chain(&anchor, &canonical, &target).is_ok());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn chain_rejects_symlinked_component() {
    let root = temp("link");
    let elsewhere = temp("elsewhere");
    std::fs::create_dir_all(root.join("sub")).expect("dirs");
    let target = root.join("sub/a.txt");
    let (anchor, canonical) = path_anchor(&target).expect("anchor");
    std::fs::remove_dir_all(root.join("sub")).expect("remove");
    std::os::unix::fs::symlink(&elsewhere, root.join("sub")).expect("symlink");
    assert!(
        verify_path_chain(&anchor, &canonical, &target).is_err(),
        "中间目录被换成链接必须拒绝"
    );
    let _ = std::fs::remove_dir_all(root);
    let _ = std::fs::remove_dir_all(elsewhere);
}
