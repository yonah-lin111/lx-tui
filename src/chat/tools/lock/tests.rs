//! 按路径写锁测试：同路径不同写法共用一把锁、锁表可复用。

use super::super::test_support::Fixture;
use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

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
    assert_eq!(drained_paths(), 0, "释放后条目不留在表里");
}

/// 写锁表按引用计数回收：写完 100 个不同路径后必须回到空，不随写入历史单调增长。
#[test]
fn table_does_not_grow_after_writes_complete() {
    let fixture = Fixture::new("lock-table");
    for index in 0..100 {
        let path = fixture.join(&format!("f{index}.txt"));
        with_path_lock(&path, || ());
    }
    assert_eq!(drained_paths(), 0, "100 个路径写完后锁表必须回到空");
}

/// 同一路径的并发进入必须串行化：闭包内的并发计数峰值只能是 1。
#[test]
fn same_path_locks_are_serialized() {
    let fixture = Fixture::new("lock-serialize");
    let path = fixture.join("a.txt");
    // 两个线程先在锁外对齐同一时刻再抢，避免「后一个线程还没起来」把重叠窗口抹掉
    // （那样即使没有互斥量也会得到峰值 1，测试就成了空洞的）。
    let start = std::sync::Barrier::new(2);
    let inside = AtomicUsize::new(0);
    let peak = AtomicUsize::new(0);
    std::thread::scope(|scope| {
        for _ in 0..2 {
            scope.spawn(|| {
                start.wait();
                with_path_lock(&path, || {
                    let now = inside.fetch_add(1, Ordering::SeqCst) + 1;
                    peak.fetch_max(now, Ordering::SeqCst);
                    // 停在临界区里：没有互斥量时另一个线程必然在这段时间内进入。
                    std::thread::sleep(std::time::Duration::from_millis(20));
                    inside.fetch_sub(1, Ordering::SeqCst);
                });
            });
        }
    });
    assert_eq!(peak.load(Ordering::SeqCst), 1, "同一路径的写入必须串行化");
}

/// 互斥量中毒不升级成 panic：body 里的 panic 正常传播，后续调用仍能拿到锁。
#[test]
fn poisoned_lock_does_not_panic() {
    let fixture = Fixture::new("lock-poison");
    let path = fixture.join("a.txt");
    let panicked = std::thread::spawn({
        let path = path.clone();
        move || with_path_lock(&path, || panic!("写入失败"))
    });
    assert!(panicked.join().is_err(), "body 里的 panic 必须原样传播");
    assert_eq!(with_path_lock(&path, || 7), 7);

    // 引用计数回收让 panic 不会留下中毒条目（上面那次取锁用的是新条目），但「表里
    // 存在中毒条目」这个时序仍必须容忍：手工种一个中毒条目，取锁不得再 panic 一次。
    let key = lock_key(&path);
    let poisoned_entry = Arc::new(PathLock::new());
    let probe = Arc::clone(&poisoned_entry);
    let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
        let _guard = probe.mutex.lock().expect("首次加锁");
        panic!("制造中毒");
    }));
    assert!(caught.is_err(), "制造中毒的 panic 必须发生");
    assert!(
        poisoned_entry.mutex.lock().is_err(),
        "互斥量应已中毒（后续 lock 返回 Err）"
    );
    {
        let mut guard = match table().lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        poisoned_entry.refs.store(1, Ordering::Relaxed);
        guard.insert(key.clone(), Arc::clone(&poisoned_entry));
    }
    assert_eq!(with_path_lock(&path, || 7), 7, "中毒条目不得让取锁 panic");
    // 清掉手工种下的条目，别污染全局锁表。
    match table().lock() {
        Ok(mut guard) => guard.remove(&key),
        Err(poisoned) => poisoned.into_inner().remove(&key),
    };
    assert_eq!(drained_paths(), 0, "panic 展开也要归还引用");
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
