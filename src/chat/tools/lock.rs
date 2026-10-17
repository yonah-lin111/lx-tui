//! 按规范化路径的短时写锁：同一文件被多个 agent loop 同时修改时串行化。
//!
//! 只锁**单个写入目标**，不把整个工具执行或模型采样串行化；锁只在一次写操作
//! 的闭包内持有，结束立即释放。锁表用 `OnceLock<Mutex<HashMap<..>>>` 惰性建立，
//! 表本身仅在取锁瞬间持有。

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};

/// 全局锁表：规范化路径 → 该路径的互斥量。
fn table() -> &'static Mutex<HashMap<String, Arc<Mutex<()>>>> {
    static TABLE: OnceLock<Mutex<HashMap<String, Arc<Mutex<()>>>>> = OnceLock::new();
    TABLE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 在 path 的写锁内执行 `body`。
///
/// 锁表或条目互斥量中毒时继续使用内部值：这里没有需要保护的不变量，
/// 单次写入失败会通过返回值体现，不需要把互斥量中毒升级成 panic。
pub(crate) fn with_path_lock<T>(path: &Path, body: impl FnOnce() -> T) -> T {
    let key = lock_key(path);
    let inner = {
        let mut guard = match table().lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        guard
            .entry(key)
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    };
    let held = match inner.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    let value = body();
    drop(held);
    value
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

#[cfg(test)]
mod tests;
