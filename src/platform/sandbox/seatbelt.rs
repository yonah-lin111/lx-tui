//! macOS seatbelt 后端：生成 SBPL profile 并用 `/usr/bin/sandbox-exec` 包装命令。
//!
//! 基础策略与网络策略来自 OpenAI Codex（Apache-2.0），见 `assets/sandbox/`。
//! 路径一律通过 `-D` 参数传入，不拼进 profile 字符串；glob 走 SBPL regex。

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

use super::glob::{self, sbpl_regex_literal};
use super::{SandboxError, SandboxSpec, Wrapped};

const BASE_POLICY: &str = include_str!("../../../assets/sandbox/seatbelt_base_policy.sbpl");
const NETWORK_POLICY: &str = include_str!("../../../assets/sandbox/seatbelt_network_policy.sbpl");
const SANDBOX_EXEC: &str = "/usr/bin/sandbox-exec";

/// 生成 profile 与 `-D` 参数。
pub(super) fn build_profile(
    spec: &SandboxSpec<'_>,
) -> Result<(String, Vec<(String, PathBuf)>), SandboxError> {
    let mut params: Vec<(String, PathBuf)> = Vec::new();
    let mut sections: Vec<String> = vec![BASE_POLICY.to_string(), "(allow file-read*)".to_string()];

    // 显式 Deny：读与写都拒绝；字面路径覆盖自身与后代。
    for (index, denied) in spec.deny_paths.iter().enumerate() {
        validate_path(denied)?;
        let key = param_for(&mut params, "DENY", denied);
        sections.push(format!("(deny file-read* (literal (param \"{key}\")))"));
        sections.push(format!("(deny file-read* (subpath (param \"{key}\")))"));
        sections.push(format!("(deny file-write* (literal (param \"{key}\")))"));
        sections.push(format!("(deny file-write* (subpath (param \"{key}\")))"));
        let _ = index;
    }

    // Deny glob：正则拒绝读与写（deny 优先于 allow）。
    let mut deny_glob_rules = Vec::new();
    for pattern in spec.deny_globs {
        let Some(regex) = glob::to_regex(pattern, true) else {
            return Err(SandboxError::Invalid(format!(
                "deny glob '{pattern}' cannot be translated to a seatbelt regex"
            )));
        };
        let literal = sbpl_regex_literal(&regex);
        deny_glob_rules.push(literal);
    }
    for literal in &deny_glob_rules {
        sections.push(format!("(deny file-read* (regex #\"{literal}\"))"));
        sections.push(format!("(deny file-write* (regex #\"{literal}\"))"));
    }

    // 写根：允许写入，但排除只读 carveout、Deny 路径与 Deny glob。
    for root in spec.writable_roots {
        validate_path(root)?;
        let write_key = param_for(&mut params, "WRITE", root);
        let mut require_parts = vec![format!("(subpath (param \"{write_key}\"))")];
        for protected in spec.read_only_paths {
            if !protected.starts_with(root) {
                continue;
            }
            validate_path(protected)?;
            let key = param_for(&mut params, "READONLY", protected);
            require_parts.push(format!("(require-not (literal (param \"{key}\")))"));
            require_parts.push(format!("(require-not (subpath (param \"{key}\")))"));
        }
        for denied in spec.deny_paths {
            if !denied.starts_with(root) {
                continue;
            }
            let key = param_for(&mut params, "DENY", denied);
            require_parts.push(format!("(require-not (literal (param \"{key}\")))"));
            require_parts.push(format!("(require-not (subpath (param \"{key}\")))"));
        }
        for literal in &deny_glob_rules {
            require_parts.push(format!("(require-not (regex #\"{literal}\"))"));
        }
        sections.push(format!(
            "(allow file-write* (require-all {}))",
            require_parts.join(" ")
        ));
    }

    // 保护祖先目录：重命名可写父目录会绕过子路径 carveout。
    let mut protected_paths: Vec<&PathBuf> = spec
        .read_only_paths
        .iter()
        .chain(spec.deny_paths.iter())
        .collect();
    protected_paths.sort();
    protected_paths.dedup();
    for protected in protected_paths {
        let Some(root) = spec
            .writable_roots
            .iter()
            .find(|root| protected.starts_with(root))
        else {
            continue;
        };
        if protected == root {
            continue;
        }
        for ancestor in protected.ancestors().skip(1) {
            if ancestor == root {
                break;
            }
            if !ancestor.starts_with(root) {
                break;
            }
            let key = param_for(&mut params, "PROTECT_ANCESTOR", ancestor);
            sections.push(format!(
                "(deny file-write-unlink (require-all (vnode-type DIRECTORY) (literal (param \"{key}\"))))"
            ));
        }
    }

    if spec.network_access {
        sections.push(NETWORK_POLICY.to_string());
        sections.push("(allow network-outbound)".to_string());
        sections.push("(allow network-inbound)".to_string());
    }
    sections.push("(deny mach-lookup (xpc-service-name-prefix \"\"))".to_string());
    sections.push("(deny system-fcntl (fcntl-command 80 110))".to_string());

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

/// 同一路径复用同一个 `-D` 键（避免 profile 里重复参数）。
fn param_for(params: &mut Vec<(String, PathBuf)>, prefix: &str, path: &Path) -> String {
    if let Some((key, _)) = params.iter().find(|(_, existing)| existing == path) {
        return key.clone();
    }
    let key = format!("{prefix}{}", params.len());
    params.push((key.clone(), path.to_path_buf()));
    key
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
