//! Linux bubblewrap 后端。
//!
//! 只使用系统 `PATH` 上的 `bwrap`；缺失或无法创建 user namespace 时由调用方
//! fail-closed（本模块不降级裸跑）。参数顺序对齐 Codex `linux-sandbox/src/bwrap.rs`。

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

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
    for root in roots {
        if !root.exists() {
            tracing::warn!(path = %root.display(), "sandbox writable root does not exist; skipping");
            continue;
        }
        argv.push(OsString::from("--bind"));
        argv.push(root.as_os_str().to_owned());
        argv.push(root.as_os_str().to_owned());
    }

    // 保护路径排在可写 bind 之后，重新以只读方式挂载。
    for protected in spec.protected {
        if !protected.exists() {
            continue;
        }
        if spec
            .protected
            .iter()
            .any(|other| other != protected && protected.starts_with(other) && other.exists())
        {
            // 被另一个保护路径覆盖时无需重复挂载。
            continue;
        }
        argv.push(OsString::from("--ro-bind"));
        argv.push(protected.as_os_str().to_owned());
        argv.push(protected.as_os_str().to_owned());
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
