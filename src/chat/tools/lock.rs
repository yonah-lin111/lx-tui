//! 按规范化路径的短时写锁：同一文件被多个 agent loop 同时修改时串行化。
//!
//! 只锁**单个写入目标**，不把整个工具执行或模型采样串行化；锁只在一次写操作
//! 的闭包内持有，结束立即释放。锁表用 `OnceLock<Mutex<HashMap<..>>>` 惰性建立，
//! 表本身仅在取锁/归还的瞬间持有。
//!
//! 条目按**引用计数**回收：取引用与归还（含减到 0 时移除）都在表锁内完成，因此
//! 表的大小 = 当前在途的写入路径数，不随历史写入过的路径数单调增长。

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

/// 一条路径的锁条目：互斥量 + 使用者引用计数。
struct PathLock {
    mutex: Mutex<()>,
    /// 当前引用该条目的调用者数。
    ///
    /// **只在 [`table`] 的表锁内读写**：所有访问都被表锁串行化，因此 `Relaxed` 足够
    /// （不需要原子本身再提供同步）；反过来，绝不允许在表锁外读它来做判断。
    refs: AtomicUsize,
}

impl PathLock {
    fn new() -> Self {
        Self {
            mutex: Mutex::new(()),
            refs: AtomicUsize::new(0),
        }
    }
}

/// 全局锁表：规范化路径 → 该路径的锁条目。
fn table() -> &'static Mutex<HashMap<String, Arc<PathLock>>> {
    static TABLE: OnceLock<Mutex<HashMap<String, Arc<PathLock>>>> = OnceLock::new();
    TABLE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 引用守卫：正常返回与 panic 展开都会归还引用（panic 不会把条目永久留在表里）。
struct RefGuard {
    key: String,
}

impl Drop for RefGuard {
    fn drop(&mut self) {
        release(&self.key);
    }
}

/// 在 path 的写锁内执行 `body`。
///
/// 锁表或条目互斥量中毒时继续使用内部值：这里没有需要保护的不变量，
/// 单次写入失败会通过返回值体现，不需要把互斥量中毒升级成 panic。
///
/// # 引用计数与条目回收的无竞态推理
///
/// 「取引用」与「归还/移除」**都在表锁内完成**，因此两者在时间上互斥：
///
/// - 取锁方：在表锁内 `entry(key).or_insert_with(..)` 并立刻 `refs += 1`；
/// - 释放方：在表锁内 `refs -= 1`，减到 0 时**在同一临界区**里 `remove(key)`。
///
/// 由此得到不变式「表里存在的条目必然 `refs >= 1`」：条目只在 `refs` 从 0 变 1 的
/// 临界区里插入、只在从 1 变 0 的临界区里移除。于是不会出现「A 判定 refs == 0 准备
/// 移除、B 同时从表里拿到同一个 `Arc`」这种丢锁窗口——B 若能拿到条目，说明它先在表锁
/// 内加了引用，A 随后（同一把表锁的后续临界区）看到的一定是 `refs >= 1`，不会移除；
/// 反过来 A 移除条目时 `refs` 已经是 0，没有任何在途持有者，此时新建的条目没有需要
/// 与之互斥的对象（等价于首次取锁）。持有 `Arc` 的调用者也不会因为条目被移除而丢互斥
/// ——移除的前提就是没有别人还持有它。
///
/// 另外，取到的 `key` 在归还时**原样复用**（不重新计算）：写入过程中目标文件可能从
/// 「不存在」变成「存在」，重算 `lock_key` 会得到另一个 key，从而找不到自己加的引用。
pub(crate) fn with_path_lock<T>(path: &Path, body: impl FnOnce() -> T) -> T {
    let key = lock_key(path);
    let _refs = RefGuard { key: key.clone() };
    let entry = {
        let mut guard = match table().lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        let entry = guard
            .entry(key)
            .or_insert_with(|| Arc::new(PathLock::new()));
        entry.refs.fetch_add(1, Ordering::Relaxed);
        Arc::clone(entry)
    };
    let held = match entry.mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    let value = body();
    drop(held);
    value
}

/// 归还一次引用；减到 0 时把条目从表里移除（见 [`with_path_lock`] 的推理）。
fn release(key: &str) {
    let mut guard = match table().lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    // 表里的条目 `refs` 必然 >= 1（插入即 +1，只在减到 0 的同一临界区里移除），
    // 因此这里的 `fetch_sub` 不会下溢；找不到条目说明本次取锁没有真的加过引用。
    let last = match guard.get(key) {
        Some(entry) => entry.refs.fetch_sub(1, Ordering::Relaxed) == 1,
        None => false,
    };
    if last {
        guard.remove(key);
    }
}

/// 规范化路径 key：优先 `canonicalize`，失败时回退父目录规范化结果拼文件名。
///
/// 新建文件时目标还不存在，`canonicalize` 必然失败；用父目录规范化结果拼接
/// 文件名，可以让 `./a.txt` 与 `a.txt` 落到同一把锁。
pub(crate) fn lock_key(path: &Path) -> String {
    if let Ok(canonical) = std::fs::canonicalize(path) {
        return crate::platform::path_key(&canonical);
    }
    if let (Some(parent), Some(name)) = (path.parent(), path.file_name())
        && let Ok(canonical) = std::fs::canonicalize(parent)
    {
        return crate::platform::path_key(&canonical.join(name));
    }
    crate::platform::path_key(path)
}

/// 便于测试：当前锁表条目数。
#[cfg(test)]
pub(crate) fn tracked_paths() -> usize {
    match table().lock() {
        Ok(guard) => guard.len(),
        Err(poisoned) => poisoned.into_inner().len(),
    }
}

/// 便于测试：等锁表清空，返回等待后仍留在表里的条目数。
///
/// 锁表是全局的，同一个测试二进制里并发的写文件测试会**短暂**占用条目；这里给
/// 一小段等待窗口，把「别人的在途写入」和「本测试的泄漏」区分开。真泄漏是永久的，
/// 等待不会掩盖它——超时后返回非零，断言照样失败。
#[cfg(test)]
pub(crate) fn drained_paths() -> usize {
    for _ in 0..200 {
        if tracked_paths() == 0 {
            return 0;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    tracked_paths()
}

#[cfg(test)]
mod tests;
