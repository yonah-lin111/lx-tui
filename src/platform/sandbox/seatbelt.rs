//! macOS seatbelt 后端：生成 SBPL profile 并用 `/usr/bin/sandbox-exec` 包装命令。
//!
//! 基础策略与网络策略来自 OpenAI Codex（Apache-2.0），见 `assets/sandbox/`。
//! 路径一律通过 `-D` 参数传入，不拼进 profile 字符串。

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

use super::{SandboxError, SandboxSpec, Wrapped};

const BASE_POLICY: &str = include_str!("../../../assets/sandbox/seatbelt_base_policy.sbpl");
const NETWORK_POLICY: &str = include_str!("../../../assets/sandbox/seatbelt_network_policy.sbpl");
const SANDBOX_EXEC: &str = "/usr/bin/sandbox-exec";

/// 生成 profile 与 `-D` 参数。
pub(super) fn build_profile(
    spec: &SandboxSpec<'_>,
) -> Result<(String, Vec<(String, PathBuf)>), SandboxError> {
    let mut params: Vec<(String, PathBuf)> = Vec::new();
    let mut write_rules = Vec::new();

    for (index, root) in spec.writable_roots.iter().enumerate() {
        validate_path(root)?;
        let key = format!("WRITE{index}");
        params.push((key.clone(), root.clone()));

        let mut require_parts = vec![format!("(subpath (param \"{key}\"))")];
        for (protect_index, protected) in spec.protected.iter().enumerate() {
            if !protected.starts_with(root) {
                continue;
            }
            validate_path(protected)?;
            let protect_key = format!("PROTECT{protect_index}");
            if !params.iter().any(|(existing, _)| existing == &protect_key) {
                params.push((protect_key.clone(), protected.clone()));
            }
            require_parts.push(format!("(require-not (literal (param \"{protect_key}\")))"));
            require_parts.push(format!("(require-not (subpath (param \"{protect_key}\")))"));
        }
        write_rules.push(format!(
            "(allow file-write* (require-all {}))",
            require_parts.join(" ")
        ));
    }

    // 保护祖先目录重新挂载：重命名可写父目录会绕过子路径 carveout。
    let mut ancestor_params: Vec<(String, PathBuf)> = Vec::new();
    for root in spec.writable_roots {
        for protected in spec.protected {
            if !protected.starts_with(root) {
                continue;
            }
            for ancestor in protected.ancestors().skip(1) {
                if !ancestor.starts_with(root) {
                    break;
                }
                if ancestor_params.iter().any(|(_, path)| path == ancestor) {
                    continue;
                }
                let key = format!("PROTECT_ANCESTOR{}", ancestor_params.len());
                ancestor_params.push((key, ancestor.to_path_buf()));
            }
        }
    }

    let mut sections: Vec<String> = vec![BASE_POLICY.to_string(), "(allow file-read*)".to_string()];
    sections.extend(write_rules);
    if spec.network_access {
        sections.push(NETWORK_POLICY.to_string());
        sections.push("(allow network-outbound)".to_string());
        sections.push("(allow network-inbound)".to_string());
    }
    sections.push("(deny mach-lookup (xpc-service-name-prefix \"\"))".to_string());
    sections.extend(
        ancestor_params
            .iter()
            .map(|(key, _)| {
                format!(
                    "(deny file-write-unlink (require-all (vnode-type DIRECTORY) (literal (param \"{key}\"))))"
                )
            }),
    );
    sections.push("(deny system-fcntl (fcntl-command 80 110))".to_string());

    params.extend(ancestor_params);
    Ok((sections.join("\n"), params))
}

/// 包装为 `sandbox-exec -p <profile> -D... -- <program> <args>`。
pub(super) fn wrap(
    spec: &SandboxSpec<'_>,
    program: &OsStr,
    args: &[OsString],
) -> Result<Wrapped, SandboxError> {
    if !Path::new(SANDBOX_EXEC).exists() {
        return Err(SandboxError::Unavailable(
            "sandbox-exec is not available on this macOS".into(),
        ));
    }
    let (profile, params) = build_profile(spec)?;
    let mut argv = vec![OsString::from("-p"), OsString::from(profile)];
    for (key, value) in params {
        let mut arg = OsString::from(format!("-D{key}="));
        arg.push(&value);
        argv.push(arg);
    }
    argv.push(OsString::from("--"));
    argv.push(program.to_owned());
    argv.extend(args.iter().cloned());
    Ok(Wrapped {
        program: PathBuf::from(SANDBOX_EXEC),
        args: argv,
    })
}

/// 路径必须是绝对路径且不含控制字符（`-D` 可承载空格与引号）。
fn validate_path(path: &Path) -> Result<(), SandboxError> {
    if !path.is_absolute() {
        return Err(SandboxError::Invalid(format!(
            "sandbox path {} is not absolute",
            path.display()
        )));
    }
    if path.to_string_lossy().chars().any(char::is_control) {
        return Err(SandboxError::Invalid(
            "sandbox path must not contain control characters".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests;
