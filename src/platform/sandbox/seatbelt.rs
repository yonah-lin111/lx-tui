//! macOS seatbelt 后端：生成 SBPL profile 并用 `/usr/bin/sandbox-exec` 包装命令。
//!
//! 基础策略与网络策略来自 OpenAI Codex（Apache-2.0），见 `assets/sandbox/`。
//! 路径一律通过 `-D` 参数传入，不拼进 profile 字符串；glob 走 SBPL regex。

use std::collections::BTreeSet;
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

    // 同时保留逻辑路径与规范化路径别名，避免 `/tmp` → `/private/tmp` 等软链前缀导致规则失效。
    let writable_roots = expand_path_aliases(spec.writable_roots)?;
    let read_only_paths = expand_path_aliases(spec.read_only_paths)?;
    let deny_paths = expand_path_aliases(spec.deny_paths)?;
    let deny_globs = expand_glob_aliases(spec.deny_globs);

    // 显式 Deny：读与写都拒绝；字面路径覆盖自身与后代。
    for denied in &deny_paths {
        let key = param_for(&mut params, "DENY", denied);
        sections.push(format!("(deny file-read* (literal (param \"{key}\")))"));
        sections.push(format!("(deny file-read* (subpath (param \"{key}\")))"));
        sections.push(format!("(deny file-write* (literal (param \"{key}\")))"));
        sections.push(format!("(deny file-write* (subpath (param \"{key}\")))"));
    }

    // Deny glob：正则拒绝读与写（deny 优先于 allow）。
    let mut deny_glob_rules = Vec::new();
    for pattern in &deny_globs {
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
    for root in &writable_roots {
        let write_key = param_for(&mut params, "WRITE", root);
        let mut require_parts = vec![format!("(subpath (param \"{write_key}\"))")];
        for protected in &read_only_paths {
            if !protected.starts_with(root) {
                continue;
            }
            let key = param_for(&mut params, "READONLY", protected);
            require_parts.push(format!("(require-not (literal (param \"{key}\")))"));
            require_parts.push(format!("(require-not (subpath (param \"{key}\")))"));
        }
        for denied in &deny_paths {
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

    // 保护可写根自身与全部保护子路径/glob 静态前缀的祖先目录，防止通过重命名父目录绕过。
    for root in &writable_roots {
        let key = param_for(&mut params, "PROTECT_ANCHOR", root);
        sections.push(format!(
            "(deny file-write-unlink (require-all (vnode-type DIRECTORY) (literal (param \"{key}\"))))"
        ));
    }
    let mut ancestor_dirs: BTreeSet<PathBuf> = BTreeSet::new();
    for protected in read_only_paths.iter().chain(deny_paths.iter()) {
        for ancestor in protected.ancestors().skip(1) {
            if writable_roots.iter().any(|root| root == ancestor) {
                continue;
            }
            if writable_roots.iter().any(|root| ancestor.starts_with(root)) {
                ancestor_dirs.insert(ancestor.to_path_buf());
            }
        }
    }
    for pattern in &deny_globs {
        let Some(static_root) = glob::scan_root(pattern) else {
            continue;
        };
        for ancestor in static_root.ancestors() {
            if writable_roots.iter().any(|root| root == ancestor) {
                continue;
            }
            if writable_roots.iter().any(|root| ancestor.starts_with(root)) {
                ancestor_dirs.insert(ancestor.to_path_buf());
            }
        }
    }
    for ancestor in ancestor_dirs {
        let key = param_for(&mut params, "PROTECT_ANCESTOR", &ancestor);
        sections.push(format!(
            "(deny file-write-unlink (require-all (vnode-type DIRECTORY) (literal (param \"{key}\"))))"
        ));
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

fn expand_path_aliases(paths: &[PathBuf]) -> Result<Vec<PathBuf>, SandboxError> {
    let mut expanded = Vec::with_capacity(paths.len());
    for path in paths {
        validate_path(path)?;
        if !expanded.contains(path) {
            expanded.push(path.clone());
        }
        if let Some(parent_canon) = super::roots::canonical_parent_or_ancestor(path) {
            validate_path(&parent_canon)?;
            if !expanded.contains(&parent_canon) {
                expanded.push(parent_canon);
            }
        }
        if let Some(canon) = super::roots::resolve_write_target(path) {
            validate_path(&canon)?;
            if !expanded.contains(&canon) {
                expanded.push(canon);
            }
        }
    }
    Ok(expanded)
}

fn expand_glob_aliases(patterns: &[String]) -> Vec<String> {
    let mut expanded = Vec::with_capacity(patterns.len());
    for pattern in patterns {
        if !expanded.contains(pattern) {
            expanded.push(pattern.clone());
        }
        if let Some(aliased) = glob::canonical_glob_alias(pattern)
            && !expanded.contains(&aliased)
        {
            expanded.push(aliased);
        }
    }
    expanded
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
        synthetic_targets: Vec::new(),
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
