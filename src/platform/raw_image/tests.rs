use super::*;
use std::path::PathBuf;

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "lx-raw-image-{name}-{}-{}",
        std::process::id(),
        lx_seq()
    ));
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

fn lx_seq() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    SEQ.fetch_add(1, Ordering::Relaxed)
}

#[test]
fn missing_roundtrip_is_explicit() {
    let dir = temp_dir("missing");
    let image = read(&dir.join("nope"), false).expect("read");
    assert_eq!(image.kind, RawKind::Missing);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn binary_bytes_roundtrip_exactly() {
    let dir = temp_dir("binary");
    let path = dir.join("bin");
    let bytes = vec![0u8, 0xff, b'\r', b'\n', 0x00, b'a'];
    std::fs::write(&path, &bytes).expect("seed");
    let image = read(&path, false).expect("read");
    assert_eq!(image.kind, RawKind::Regular);
    assert_eq!(image.bytes, bytes);
    std::fs::remove_file(&path).expect("remove");
    write_regular(&path, &image).expect("write");
    assert_eq!(std::fs::read(&path).expect("read back"), bytes);
    let _ = std::fs::remove_dir_all(dir);
}

#[cfg(unix)]
#[test]
fn symlink_is_saved_and_restored_as_link() {
    let dir = temp_dir("link");
    let target = dir.join("target");
    let link = dir.join("link");
    std::fs::write(&target, b"body").expect("target");
    std::os::unix::fs::symlink(&target, &link).expect("link");
    let image = read(&link, false).expect("read");
    assert_eq!(image.kind, RawKind::Symlink);
    assert_eq!(image.link_target.as_deref(), Some(target.as_path()));
    // 跟随读取得到物理文件内容，但不改变链接本身。
    let followed = read(&link, true).expect("follow");
    assert_eq!(followed.kind, RawKind::Regular);
    assert_eq!(followed.bytes, b"body");
    write_symlink(&link, target.as_path()).expect("restore");
    assert!(
        std::fs::symlink_metadata(&link)
            .expect("stat")
            .file_type()
            .is_symlink()
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn directory_is_rejected() {
    let dir = temp_dir("dir");
    assert!(read(&dir, false).is_err());
    assert!(remove_entry(&dir).is_err());
    let _ = std::fs::remove_dir_all(dir);
}
