//! Linux bubblewrap 后端。
//!
//! 只使用系统 `PATH` 上的 `bwrap`；缺失或无法创建 user namespace 时由调用方
//! fail-closed（本模块不降级裸跑）。参数顺序对齐 Codex `linux-sandbox/src/bwrap.rs`。

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

use super::glob;
use super::{SandboxError, SandboxSpec, Wrapped};

const BWRAP: &str = "bwrap";

/// 包装为 `bwrap ... -- <program> <args>`。
pub(super) fn wrap(
    spec: &SandboxSpec<'_>,
    program: &OsStr,
    args: &[OsString],
) -> Result<Wrapped, SandboxError> {
    if !command_on_path(BWRAP) {
        return Err(SandboxError::Unavailable(
            "bubblewrap (bwrap) is not installed; install it or switch to danger-full-access"
                .into(),
        ));
    }
    let argv = build_args(spec, program, args)?;
    Ok(Wrapped {
        program: PathBuf::from(BWRAP),
        args: argv,
    })
}

fn build_args(
    spec: &SandboxSpec<'_>,
    program: &OsStr,
    args: &[OsString],
) -> Result<Vec<OsString>, SandboxError> {
    if !spec.cwd.is_absolute() {
        return Err(SandboxError::Invalid(
            "sandbox cwd must be an absolute path".into(),
        ));
    }
    let mut argv: Vec<OsString> = Vec::new();
    for arg in ["--new-session", "--die-with-parent", "--ro-bind", "/", "/"] {
        argv.push(OsString::from(arg));
    }
    for arg in ["--dev", "/dev"] {
        argv.push(OsString::from(arg));
    }
    for arg in ["--unshare-user", "--unshare-pid", "--unshare-ipc"] {
        argv.push(OsString::from(arg));
    }
    if !spec.network_access {
        argv.push(OsString::from("--unshare-net"));
    }

    let mut roots: Vec<&PathBuf> = spec.writable_roots.iter().collect();
    roots.sort_by_key(|root| root.components().count());
    for root in &roots {
        if !root.exists() {
            tracing::warn!(path = %root.display(), "sandbox writable root does not exist; skipping");
            continue;
        }
        push_bind(&mut argv, "--bind", root, root);
    }

    // 只读 carveout 排在可写 bind 之后，重新以只读方式挂载。
    let mut read_only: Vec<&PathBuf> = spec.read_only_paths.iter().collect();
    read_only.sort_by_key(|path| path.components().count());
    read_only.dedup();
    for path in read_only {
        if !roots.iter().any(|root| path.starts_with(root)) {
            continue;
        }
        if path.exists() {
            push_bind(&mut argv, "--ro-bind", path, path);
        } else if let Some(first_missing) = first_missing_component(path) {
            // 不存在的只读 carveout 也要阻止被创建（空文件占位）。
            push_bind(
                &mut argv,
                "--ro-bind",
                Path::new("/dev/null"),
                &first_missing,
            );
        }
    }

    // Deny：字面路径 + 有界 glob 展开，统一在可写 bind 之后遮盖。
    let mut denied: Vec<PathBuf> = spec.deny_paths.clone();
    denied.extend(glob::scan_matches(
        spec.deny_globs,
        spec.glob_scan_max_depth,
    )?);
    denied.sort_by_key(|path| path.components().count());
    denied.dedup();
    let writable: Vec<PathBuf> = roots.into_iter().cloned().collect();
    for path in &denied {
        append_deny(&mut argv, path, &writable)?;
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
    Ok(argv)
}

fn push_bind(argv: &mut Vec<OsString>, kind: &str, source: &Path, destination: &Path) {
    argv.push(OsString::from(kind));
    argv.push(source.as_os_str().to_owned());
    argv.push(destination.as_os_str().to_owned());
}

/// 遮盖一个 deny 路径；不在任何写根内的路径由 `--ro-bind / /` 天然保护。
fn append_deny(
    argv: &mut Vec<OsString>,
    path: &Path,
    writable_roots: &[PathBuf],
) -> Result<(), SandboxError> {
    if !writable_roots.iter().any(|root| path.starts_with(root)) {
        return Ok(());
    }
    // 跨可写符号链接的 deny 目标无法可靠保护（TOCTOU），直接 fail-closed。
    if let Some(symlink) = writable_symlink_component(path, writable_roots) {
        return Err(SandboxError::Invalid(format!(
            "cannot enforce deny path {} because it crosses writable symlink {}",
            path.display(),
            symlink.display()
        )));
    }
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() => {
            argv.push(OsString::from("--perms"));
            argv.push(OsString::from("000"));
            argv.push(OsString::from("--tmpfs"));
            argv.push(path.as_os_str().to_owned());
            argv.push(OsString::from("--remount-ro"));
            argv.push(path.as_os_str().to_owned());
        }
        Ok(_) => push_bind(argv, "--ro-bind", Path::new("/dev/null"), path),
        Err(_) => {
            // 不存在：用空文件占住第一个缺失组件，阻止其下创建任何内容。
            let Some(first_missing) = first_missing_component(path) else {
                return Ok(());
            };
            if writable_roots
                .iter()
                .any(|root| first_missing.starts_with(root))
            {
                push_bind(argv, "--ro-bind", Path::new("/dev/null"), &first_missing);
            }
        }
    }
    Ok(())
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
