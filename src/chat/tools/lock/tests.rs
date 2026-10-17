//! 按路径写锁测试：同路径不同写法共用一把锁、锁表可复用。

use super::super::test_support::Fixture;
use super::*;

#[test]
fn same_file_variants_share_a_key() {
    let fixture = Fixture::new("lock-key");
    let file = fixture.write("a.txt", "x");
    let dotted = fixture.join(".").join("a.txt");
    assert_eq!(lock_key(&file), lock_key(&dotted));

    // 尚不存在的目标：用父目录规范化结果拼接文件名。
    let missing = fixture.join("b.txt");
    assert_eq!(
        lock_key(&missing),
        lock_key(&fixture.join(".").join("b.txt"))
    );
}

#[test]
fn lock_is_reusable_after_release() {
    let fixture = Fixture::new("lock-reuse");
    let path = fixture.join("a.txt");
    let first = with_path_lock(&path, || 7);
    assert_eq!(first, 7);
    let second = with_path_lock(&path, || 8);
    assert_eq!(second, 8);
    assert!(tracked_paths() >= 1);
}

#[test]
fn different_paths_do_not_block_each_other() {
    let fixture = Fixture::new("lock-distinct");
    let a = fixture.join("a.txt");
    let b = fixture.join("b.txt");
    let counter = std::sync::atomic::AtomicUsize::new(0);
    with_path_lock(&a, || {
        with_path_lock(&b, || {
            counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        });
    });
    assert_eq!(counter.load(std::sync::atomic::Ordering::Relaxed), 1);
}
