//! Linux bubblewrap 后端。
//!
//! 只使用系统 `PATH` 上的 `bwrap`；缺失或无法创建 namespace 时由调用方
//! fail-closed（本模块不降级裸跑）。参数顺序对齐 Codex `linux-sandbox/src/bwrap.rs`。

use std::collections::BTreeSet;
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;

use super::glob;
use super::{SandboxError, SandboxSpec, Wrapped};

const BWRAP: &str = "bwrap";

/// `bwrap` 的 user namespace 能力模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum UserNsMode {
    /// 支持 `--unshare-user`（非特权 userns 可用）。
    UnshareUser,
    /// 内核禁用了非特权 userns，但 setuid/特权 `bwrap` 可不带 `--unshare-user` 运行。
    SetuidFallback,
    /// `bwrap` 不可用或无法创建所需 namespace。
    Unavailable,
}

/// 宿主机 `bwrap` 运行时探测能力。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct BwrapCapabilities {
    /// `bwrap` 是否可用。
    pub(super) available: bool,
    /// user namespace 模式。
    pub(super) user_ns: UserNsMode,
    /// 是否支持 `--perms` 参数（`bwrap >= 0.5.0`）。
    pub(super) supports_perms: bool,
}

/// 进程级缓存一次 `bwrap` 能力探测。
fn capabilities() -> BwrapCapabilities {
    static CAPS: OnceLock<BwrapCapabilities> = OnceLock::new();
    *CAPS.get_or_init(probe_capabilities)
}

/// 探测 `bwrap` 是否存在、`--perms` 支持度以及 userns/setuid 回退模式。
fn probe_capabilities() -> BwrapCapabilities {
    if !command_on_path(BWRAP) {
        return BwrapCapabilities {
            available: false,
            user_ns: UserNsMode::Unavailable,
            supports_perms: false,
        };
    }
    let supports_perms = Command::new(BWRAP)
        .arg("--help")
        .output()
        .map(|output| {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            stdout.contains("--perms") || stderr.contains("--perms")
        })
        .unwrap_or(false);

    let user_ns = if probe_bwrap(&["--unshare-user", "--ro-bind", "/", "/", "--", "/bin/true"]) {
        UserNsMode::UnshareUser
    } else if probe_bwrap(&[
        "--ro-bind",
        "/",
        "/",
        "--unshare-pid",
        "--unshare-ipc",
        "--",
        "/bin/true",
    ]) {
        tracing::info!(
            target: "lx::sandbox",
            "bwrap unshare-user unavailable; using setuid/privileged fallback without --unshare-user"
        );
        UserNsMode::SetuidFallback
    } else {
        tracing::warn!(
            target: "lx::sandbox",
            "bwrap is installed but cannot create namespaces; marking sandbox unavailable"
        );
        UserNsMode::Unavailable
    };

    BwrapCapabilities {
        available: user_ns != UserNsMode::Unavailable,
        user_ns,
        supports_perms,
    }
}

fn probe_bwrap(args: &[&str]) -> bool {
    Command::new(BWRAP)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

/// 探测当前主机 `bwrap` 是否可用（用于 `platform::sandbox::support()`）。
pub(super) fn is_available() -> bool {
    capabilities().available
}

/// 包装为 `bwrap ... -- <program> <args>`。
pub(super) fn wrap(
    spec: &SandboxSpec<'_>,
    program: &OsStr,
    args: &[OsString],
) -> Result<Wrapped, SandboxError> {
    build_args_with_caps(spec, program, args, capabilities())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum MountAccess {
    Write,
    ReadOnly,
}

/// 按给定能力构造 `bwrap` 参数与待清理的合成占位路径（供单元测试注入能力）。
pub(super) fn build_args_with_caps(
    spec: &SandboxSpec<'_>,
    program: &OsStr,
    args: &[OsString],
    caps: BwrapCapabilities,
) -> Result<Wrapped, SandboxError> {
    if !caps.available || caps.user_ns == UserNsMode::Unavailable {
        return Err(SandboxError::Unavailable(
            "bubblewrap (bwrap) is unavailable or cannot create required namespaces; install/configure bwrap or switch to danger-full-access"
                .into(),
        ));
    }
    if !spec.cwd.is_absolute() {
        return Err(SandboxError::Invalid(
            "sandbox cwd must be an absolute path".into(),
        ));
    }
    let mut argv: Vec<OsString> = Vec::new();
    for arg in [
        "--new-session",
        "--die-with-parent",
        "--ro-bind",
        "/",
        "/",
        "--dev",
        "/dev",
    ] {
        argv.push(OsString::from(arg));
    }
    if caps.user_ns == UserNsMode::UnshareUser {
        argv.push(OsString::from("--unshare-user"));
    }
    for arg in ["--unshare-pid", "--unshare-ipc"] {
        argv.push(OsString::from(arg));
    }
    if !spec.network_access {
        argv.push(OsString::from("--unshare-net"));
    }

    // 先过滤出真实存在的可写根，再参与前缀与挂载排序，避免对缺失写根在只读 `/` 上建占位挂载。
    let mut writable: Vec<PathBuf> = spec
        .writable_roots
        .iter()
        .filter(|root| {
            if root.exists() {
                true
            } else {
                tracing::warn!(path = %root.display(), "sandbox writable root does not exist; skipping");
                false
            }
        })
        .cloned()
        .collect();
    writable.sort_by(|left, right| {
        left.components()
            .count()
            .cmp(&right.components().count())
            .then_with(|| left.cmp(right))
    });
    writable.dedup();

    // 合并可写根与写根内只读 carveout，按浅→深交替挂载（同深度下 ReadOnly 覆盖 Write）。
    let mut read_only: Vec<PathBuf> = spec
        .read_only_paths
        .iter()
        .filter(|path| writable.iter().any(|root| path.starts_with(root)))
        .cloned()
        .collect();
    read_only.sort_by(|left, right| {
        left.components()
            .count()
            .cmp(&right.components().count())
            .then_with(|| left.cmp(right))
    });
    read_only.dedup();
    for path in &read_only {
        if let Some(symlink) = writable_symlink_component(path, &writable) {
            return Err(SandboxError::Invalid(format!(
                "cannot enforce read-only path {} because it crosses writable symlink {}",
                path.display(),
                symlink.display()
            )));
        }
    }

    let mut layered: Vec<(usize, MountAccess, PathBuf)> = Vec::new();
    for root in &writable {
        layered.push((root.components().count(), MountAccess::Write, root.clone()));
    }
    for path in read_only {
        layered.push((path.components().count(), MountAccess::ReadOnly, path));
    }
    layered.sort_by(|left, right| {
        left.0
            .cmp(&right.0)
            .then_with(|| left.1.cmp(&right.1))
            .then_with(|| left.2.cmp(&right.2))
    });

    let mut synthetic_targets: BTreeSet<PathBuf> = BTreeSet::new();
    let mut ro_null_mounts: Vec<PathBuf> = Vec::new();
    let mut emitted_mounts: Vec<(MountAccess, PathBuf)> = Vec::new();
    for (_, access, path) in layered {
        match access {
            MountAccess::Write => {
                push_bind(&mut argv, "--bind", &path, &path);
                emitted_mounts.push((MountAccess::Write, path));
            }
            MountAccess::ReadOnly => {
                if path.exists() {
                    push_bind(&mut argv, "--ro-bind", &path, &path);
                    emitted_mounts.push((MountAccess::ReadOnly, path));
                } else if !is_system_temp_auto_git(&path)
                    && let Some(first_missing) = first_missing_component(&path)
                    && is_currently_writable(&first_missing, &emitted_mounts)
                    && !writable
                        .iter()
                        .any(|root| root != &first_missing && root.starts_with(&first_missing))
                    && !ro_null_mounts
                        .iter()
                        .any(|existing| first_missing.starts_with(existing))
                {
                    if std::fs::symlink_metadata(&first_missing).is_err() {
                        synthetic_targets.insert(first_missing.clone());
                    }
                    ro_null_mounts.push(first_missing.clone());
                    // 不存在的只读 carveout 也要阻止被创建（空文件占位，退出后清理）。
                    push_bind(
                        &mut argv,
                        "--ro-bind",
                        Path::new("/dev/null"),
                        &first_missing,
                    );
                    emitted_mounts.push((MountAccess::ReadOnly, first_missing));
                }
            }
        }
    }

    // Deny：字面路径 + 有界 glob 展开；写根外已存在路径同样遮盖，`read-only` 才会生效。
    let mut denied: Vec<PathBuf> = spec.deny_paths.to_vec();
    denied.extend(glob::scan_matches(
        spec.deny_globs,
        spec.glob_scan_max_depth,
    )?);
    denied.sort_by(|left, right| {
        left.components()
            .count()
            .cmp(&right.components().count())
            .then_with(|| left.cmp(right))
    });
    denied.dedup();
    let mut deny_ctx = DenyContext {
        argv: &mut argv,
        writable_roots: &writable,
        caps,
        emitted_mounts: &emitted_mounts,
        masked_prefixes: Vec::new(),
        ro_null_mounts: &ro_null_mounts,
        synthetic_targets: &mut synthetic_targets,
    };
    for path in &denied {
        append_deny(&mut deny_ctx, path)?;
    }

    argv.push(OsString::from("--proc"));
    argv.push(OsString::from("/proc"));
    argv.push(OsString::from("--chdir"));
    argv.push(spec.cwd.as_os_str().to_owned());
    argv.push(OsString::from("--cap-drop"));
    argv.push(OsString::from("ALL"));
    argv.push(OsString::from("--"));
    argv.push(program.to_owned());
    argv.extend(args.iter().cloned());
    Ok(Wrapped {
        program: PathBuf::from(BWRAP),
        args: argv,
        synthetic_targets: synthetic_targets.into_iter().collect(),
    })
}

fn push_bind(argv: &mut Vec<OsString>, kind: &str, source: &Path, destination: &Path) {
    argv.push(OsString::from(kind));
    argv.push(source.as_os_str().to_owned());
    argv.push(destination.as_os_str().to_owned());
}

struct DenyContext<'a> {
    argv: &'a mut Vec<OsString>,
    writable_roots: &'a [PathBuf],
    caps: BwrapCapabilities,
    emitted_mounts: &'a [(MountAccess, PathBuf)],
    masked_prefixes: Vec<PathBuf>,
    ro_null_mounts: &'a [PathBuf],
    synthetic_targets: &'a mut BTreeSet<PathBuf>,
}

fn is_currently_writable(path: &Path, mounts: &[(MountAccess, PathBuf)]) -> bool {
    let mut writable = false;
    for (access, mount_path) in mounts {
        if path.starts_with(mount_path) {
            writable = *access == MountAccess::Write;
        }
    }
    writable
}

/// 遮盖一个 deny 路径；写根外仅跳过不存在路径，已存在路径同样遮盖。
fn append_deny(ctx: &mut DenyContext<'_>, path: &Path) -> Result<(), SandboxError> {
    if ctx
        .masked_prefixes
        .iter()
        .any(|prefix| path.starts_with(prefix))
    {
        return Ok(());
    }
    // 跨可写符号链接的 deny 目标无法可靠保护（TOCTOU），直接 fail-closed。
    if let Some(symlink) = writable_symlink_component(path, ctx.writable_roots) {
        return Err(SandboxError::Invalid(format!(
            "cannot enforce deny path {} because it crosses writable symlink {}",
            path.display(),
            symlink.display()
        )));
    }
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() => {
            if !ctx.caps.supports_perms {
                return Err(SandboxError::Unavailable(
                    "bubblewrap (bwrap) on this host does not support --perms required for denied directories"
                        .into(),
                ));
            }
            ctx.argv.push(OsString::from("--perms"));
            ctx.argv.push(OsString::from("000"));
            ctx.argv.push(OsString::from("--tmpfs"));
            ctx.argv.push(path.as_os_str().to_owned());
            ctx.argv.push(OsString::from("--remount-ro"));
            ctx.argv.push(path.as_os_str().to_owned());
            ctx.masked_prefixes.push(path.to_path_buf());
        }
        Ok(_) => push_bind(ctx.argv, "--ro-bind", Path::new("/dev/null"), path),
        Err(_) => {
            // 写根外或当前只读区内不存在的路径由已挂载的只读层阻止创建，无需额外挂载。
            if !ctx.writable_roots.iter().any(|root| path.starts_with(root)) {
                return Ok(());
            }
            let Some(first_missing) = first_missing_component(path) else {
                return Ok(());
            };
            if is_currently_writable(&first_missing, ctx.emitted_mounts)
                && !ctx
                    .masked_prefixes
                    .iter()
                    .any(|prefix| first_missing.starts_with(prefix))
            {
                ctx.masked_prefixes.push(first_missing.clone());
                if !ctx.ro_null_mounts.contains(&first_missing) {
                    if std::fs::symlink_metadata(&first_missing).is_err() {
                        ctx.synthetic_targets.insert(first_missing.clone());
                    }
                    push_bind(
                        ctx.argv,
                        "--ro-bind",
                        Path::new("/dev/null"),
                        &first_missing,
                    );
                }
            }
        }
    }
    Ok(())
}

/// 系统共享临时目录（`/tmp`、`/var/tmp`、`$TMPDIR`）下不存在的自动 `.git` 保护跳过，
/// 避免在多用户粘滞目录残留 0 字节 `.git` 文件。
fn is_system_temp_auto_git(path: &Path) -> bool {
    if path.file_name() != Some(OsStr::new(".git")) {
        return false;
    }
    let Some(parent) = path.parent() else {
        return false;
    };
    let mut temp_roots = vec![PathBuf::from("/tmp"), PathBuf::from("/var/tmp")];
    if let Some(tmpdir) = std::env::var_os("TMPDIR").filter(|value| !value.is_empty()) {
        temp_roots.push(PathBuf::from(tmpdir));
    }
    temp_roots.into_iter().any(|root| {
        parent == root
            || std::fs::canonicalize(&root)
                .map(|canon| parent == canon)
                .unwrap_or(false)
    })
}

/// 路径上第一个位于写根内的符号链接组件。
fn writable_symlink_component(path: &Path, writable_roots: &[PathBuf]) -> Option<PathBuf> {
    let mut prefix = PathBuf::new();
    for component in path.components() {
        prefix.push(component);
        if !writable_roots.iter().any(|root| prefix.starts_with(root)) {
            continue;
        }
        if std::fs::symlink_metadata(&prefix)
            .map(|metadata| metadata.file_type().is_symlink())
            .unwrap_or(false)
        {
            return Some(prefix);
        }
    }
    None
}

/// 沿路径找到最深的已存在祖先，返回其下的第一个缺失组件。
fn first_missing_component(path: &Path) -> Option<PathBuf> {
    let mut cursor = path;
    loop {
        let parent = cursor.parent()?;
        if parent.exists() {
            return Some(cursor.to_path_buf());
        }
        cursor = parent;
    }
}

fn command_on_path(name: &str) -> bool {
    std::env::var_os("PATH")
        .map(|paths| {
            std::env::split_paths(&paths)
                .map(|dir| dir.join(name))
                .any(|candidate| candidate.is_file())
        })
        .unwrap_or(false)
}

#[cfg(test)]
mod tests;
