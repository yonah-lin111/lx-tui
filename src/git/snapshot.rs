//! Git 对象库快照：按**实际目标路径**查询所属 worktree 身份，并用对象库保存原始字节。
//!
//! 这里只写/读 blob，不做 reset/checkout/clean/stash，也不改 index、分支、提交或 refs。
//! 让对象保活不是本模块的职责：SQLite 同时保留字节备份，GC 后仍可恢复。
//!
//! 子进程一律独立传参、经 stdin 传字节、隔离继承的 `GIT_DIR`/`GIT_WORK_TREE` 等
//! 重定向变量，并带超时；超时或退出码非零都按失败返回，不伪造成功。

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Git 子进程超时（大文件哈希与读取都应在数秒内完成）。
const GIT_TIMEOUT: Duration = Duration::from_secs(15);

/// 实际目标所属 worktree 的冻结身份。
///
/// 相同目录切换分支不会改变 gitdir，因此恢复前必须同时核对符号 HEAD 与 HEAD OID。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeIdentity {
    /// 实际 worktree 根（不是会话共享的 repo key，也不是主 checkout）。
    pub root: PathBuf,
    pub git_dir: PathBuf,
    pub common_dir: PathBuf,
    /// 符号 HEAD（如 `refs/heads/dev`）；detached 为 None。
    pub head_ref: Option<String>,
    /// HEAD OID；unborn 仓库为 None。
    pub head_oid: Option<String>,
}

impl WorktreeIdentity {
    /// 恢复前复核：身份四要素必须一致。
    pub fn matches(&self, other: &Self) -> bool {
        self.root == other.root
            && self.git_dir == other.git_dir
            && self.common_dir == other.common_dir
            && self.head_ref == other.head_ref
            && self.head_oid == other.head_oid
    }
}

/// 查询目标路径所属 worktree；非 Git 目录、git 缺失或命令失败返回 None。
///
/// 目标可以尚不存在：沿现存祖先向上找目录，绝不用进程 cwd 兜底。
pub fn identify(path: &Path) -> Option<WorktreeIdentity> {
    let directory = containing_directory(path)?;
    let root = run_text(&directory, &["rev-parse", "--show-toplevel"])?;
    let git_dir = run_text(&directory, &["rev-parse", "--git-dir"])?;
    let common_dir = run_text(&directory, &["rev-parse", "--git-common-dir"])?;
    let head_ref = run_text(&directory, &["symbolic-full-name", "HEAD"])
        .filter(|value| value.starts_with("refs/"));
    // unborn 仓库没有 HEAD OID；显式保持 None，不伪造提交。
    let head_oid = run_text(&directory, &["rev-parse", "--verify", "HEAD"]);
    let root = canonical(&PathBuf::from(root));
    Some(WorktreeIdentity {
        git_dir: absolute_in(&root, &git_dir),
        common_dir: absolute_in(&root, &common_dir),
        root,
        head_ref,
        head_oid,
    })
}

/// 用原始字节写 blob（`hash-object -w --stdin`）；返回 OID。
pub fn write_blob(identity: &WorktreeIdentity, bytes: &[u8]) -> Option<String> {
    let output = run_with_input(&identity.root, &["hash-object", "-w", "--stdin"], bytes)?;
    let oid = String::from_utf8(output).ok()?.trim().to_string();
    (!oid.is_empty()).then_some(oid)
}

/// 读取 blob 原始字节（`cat-file blob <oid>`）。
pub fn read_blob(identity: &WorktreeIdentity, oid: &str) -> Option<Vec<u8>> {
    run_with_input(&identity.root, &["cat-file", "blob", oid], &[])
}

/// 目标所属的现存目录：文件用父目录，缺失路径向上找第一个存在的祖先。
fn containing_directory(path: &Path) -> Option<PathBuf> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().ok()?.join(path)
    };
    let mut current = if absolute.is_dir() {
        absolute
    } else {
        absolute.parent()?.to_path_buf()
    };
    loop {
        if current.is_dir() {
            return Some(current);
        }
        if !current.pop() {
            return None;
        }
    }
}

/// 执行 git 并返回去除首尾空白的 stdout。
fn run_text(directory: &Path, args: &[&str]) -> Option<String> {
    let output = run(directory, args, &[])?;
    let text = String::from_utf8(output).ok()?.trim().to_string();
    (!text.is_empty()).then_some(text)
}

/// 执行 git，可选写入 stdin；成功返回 stdout 原始字节。
fn run_with_input(directory: &Path, args: &[&str], input: &[u8]) -> Option<Vec<u8>> {
    run(directory, args, input)
}

/// 执行 git 子进程：隔离继承的重定向变量、写入 stdin、限时等待并检查退出码。
///
/// stdout 由独立线程持续排空：`cat-file blob` 可能输出远超管道缓冲的字节，
/// 不排空会让子进程阻塞、`try_wait` 永远看不到退出，最终被超时误杀。
fn run(directory: &Path, args: &[&str], input: &[u8]) -> Option<Vec<u8>> {
    let mut command = Command::new("git");
    command
        .arg("-C")
        .arg(directory)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    for key in [
        "GIT_DIR",
        "GIT_WORK_TREE",
        "GIT_INDEX_FILE",
        "GIT_OBJECT_DIRECTORY",
        "GIT_ALTERNATE_OBJECT_DIRECTORIES",
        "GIT_NAMESPACE",
    ] {
        command.env_remove(key);
    }
    let mut child = command.spawn().ok()?;
    if !input.is_empty() {
        let mut stdin = child.stdin.take()?;
        stdin.write_all(input).ok()?;
    } else {
        // 关闭 stdin，避免读命令等待输入。
        drop(child.stdin.take());
    }
    let mut stdout = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        let mut buffer = Vec::new();
        std::io::Read::read_to_end(&mut stdout, &mut buffer).ok()?;
        Some(buffer)
    });
    let deadline = Instant::now() + GIT_TIMEOUT;
    loop {
        match child.try_wait().ok()? {
            Some(status) if status.success() => {
                return reader.join().ok().flatten();
            }
            Some(_) => {
                let _ = reader.join();
                return None;
            }
            None if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = reader.join();
                return None;
            }
            None => std::thread::sleep(Duration::from_millis(5)),
        }
    }
}

/// 相对路径按 worktree 根解析；绝对路径原样规范化。
fn absolute_in(root: &Path, value: &str) -> PathBuf {
    let path = Path::new(value);
    if path.is_absolute() {
        canonical(path)
    } else {
        canonical(&root.join(path))
    }
}

/// 尽力规范化；失败保留原路径。
fn canonical(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

#[cfg(test)]
mod tests;
