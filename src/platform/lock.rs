//! 跨进程 advisory 锁：路径写锁与 session 执行权。
//!
//! 锁文件按稳定算法命名（路径/会话字符串的 FNV-1a 哈希），不使用带随机种子的
//! `HashMap` 哈希；锁的是**独立 sidecar 文件**，不是会被原子替换的目标 inode。
//! 碰撞只会造成额外串行，绝不会把两个目标当成同一个恢复对象。

use std::fs::{File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// 路径锁等待上限：超过即放弃（调用方按冲突处理，不无限挂住事件循环）。
const PATH_LOCK_TIMEOUT: Duration = Duration::from_secs(5);
/// 轮询间隔。
const POLL: Duration = Duration::from_millis(5);

/// 一把 held 的跨进程锁；关闭文件即释放（Drop）。
#[derive(Debug)]
pub struct FileGuard {
    file: File,
}

impl Drop for FileGuard {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

/// 按稳定顺序取得一组路径锁；`Ok(None)` 表示超时。
#[derive(Debug)]
pub struct PathGuards {
    _guards: Vec<FileGuard>,
}

/// 取得路径锁组（按 canonical key 排序去重后逐个阻塞至超时）。
pub fn lock_paths(paths: &[PathBuf]) -> io::Result<Option<PathGuards>> {
    let mut keys: Vec<String> = paths.iter().map(|path| path_key(path)).collect();
    keys.sort();
    keys.dedup();
    let root = lock_root()?.join("paths");
    std::fs::create_dir_all(&root)?;
    let mut guards = Vec::with_capacity(keys.len());
    for key in keys {
        match lock_file(&root, &key, PATH_LOCK_TIMEOUT)? {
            Some(guard) => guards.push(guard),
            None => return Ok(None),
        }
    }
    Ok(Some(PathGuards { _guards: guards }))
}

/// 非阻塞取得某会话的跨进程执行权；拿不到返回 None。
///
/// 新轮次、撤销与删除共用同一把锁；锁文件放在 `locks/sessions/`。
pub fn try_lock_session(session_id: &str) -> io::Result<Option<FileGuard>> {
    try_lock_named("sessions", session_id)
}

/// 尝试取得普通命名锁（测试与内部复用）。
pub fn try_lock_named(group: &str, name: &str) -> io::Result<Option<FileGuard>> {
    let root = lock_root()?.join(group);
    std::fs::create_dir_all(&root)?;
    lock_file(&root, name, Duration::ZERO)
}

/// 锁根目录：与数据库同环境的 `~/.lx-t[-dev]/locks`。
pub fn lock_root() -> io::Result<PathBuf> {
    let home = std::env::var_os("HOME")
        .filter(|home| !home.is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "home directory unavailable"))?;
    // 显式覆盖数据库路径时，锁目录也必须跟着同一份 DB 走：否则两个实例用不同
    // `LX_TUI_ENV` 打开同一个 DB，却落在 dev/prod 两个互不相干的锁目录。
    if let Some(db) = std::env::var_os("LX_TUI_DB_PATH").filter(|value| !value.is_empty()) {
        let key = super::path_key(Path::new(&db));
        return Ok(home
            .join(".lx-t")
            .join("locks")
            .join(format!("db-{:016x}", hash(key.as_bytes()))));
    }
    let root = if crate::platform::dev_environment() {
        ".lx-t-dev"
    } else {
        ".lx-t"
    };
    Ok(home.join(root).join("locks"))
}

/// 稳定路径键：优先 canonicalize；失败时用父目录规范化结果拼文件名。
pub fn path_key(path: &Path) -> String {
    if let Ok(canonical) = std::fs::canonicalize(path) {
        return super::path_key(&canonical);
    }
    if let (Some(parent), Some(name)) = (path.parent(), path.file_name())
        && let Ok(canonical) = std::fs::canonicalize(parent)
    {
        return super::path_key(&canonical.join(name));
    }
    super::path_key(path)
}

/// 打开并尝试锁定一个 sidecar；`timeout` 内重试。
fn lock_file(root: &Path, name: &str, timeout: Duration) -> io::Result<Option<FileGuard>> {
    let path = root.join(format!("{:016x}.lock", hash(name.as_bytes())));
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path)?;
    let deadline = Instant::now() + timeout;
    loop {
        match file.try_lock() {
            Ok(()) => return Ok(Some(FileGuard { file })),
            Err(std::fs::TryLockError::WouldBlock) => {
                if Instant::now() >= deadline {
                    return Ok(None);
                }
                std::thread::sleep(POLL);
            }
            Err(std::fs::TryLockError::Error(error)) => return Err(error),
        }
    }
}

/// FNV-1a 64 位：跨进程稳定，不依赖随机状态。
fn hash(bytes: &[u8]) -> u64 {
    let mut value: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        value ^= u64::from(*byte);
        value = value.wrapping_mul(0x0000_0100_0000_01b3);
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_is_stable_and_distinguishes_paths() {
        assert_eq!(hash(b"/tmp/a"), hash(b"/tmp/a"));
        assert_ne!(hash(b"/tmp/a"), hash(b"/tmp/b"));
    }

    #[test]
    fn same_path_blocks_second_lock_until_released() {
        let first = try_lock_named("tests-lock", "same")
            .expect("lock")
            .expect("free");
        let second = try_lock_named("tests-lock", "same").expect("lock");
        assert!(second.is_none(), "同一路径第二次取锁必须等待/失败");
        drop(first);
        let third = try_lock_named("tests-lock", "same").expect("lock");
        assert!(third.is_some(), "释放后可以重新取得");
    }

    #[test]
    fn different_paths_can_lock_in_parallel() {
        let first = try_lock_named("tests-lock", "a").expect("lock");
        let second = try_lock_named("tests-lock", "b").expect("lock");
        assert!(first.is_some() && second.is_some());
    }
}
