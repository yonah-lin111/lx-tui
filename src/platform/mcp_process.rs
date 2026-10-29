//! MCP stdio 子进程：环境白名单、项目 cwd、独立进程组。
//!
//! 子进程只继承白名单变量，再叠加显式透传（`env_vars`）与显式覆盖（`env`）。
//! 任何密钥都必须走显式透传，不能依赖父进程环境污染子进程。

use std::collections::BTreeMap;
use std::path::Path;

/// Unix 子进程默认继承的父进程环境变量（对齐 codex `DEFAULT_ENV_VARS`）。
#[cfg(unix)]
pub const MCP_DEFAULT_ENV_VARS: &[&str] = &[
    "HOME",
    "LOGNAME",
    "PATH",
    "SHELL",
    "USER",
    "__CF_USER_TEXT_ENCODING",
    "LANG",
    "LC_ALL",
    "TERM",
    "TMPDIR",
    "TZ",
];

/// Windows 子进程默认继承的父进程环境变量。
#[cfg(windows)]
pub const MCP_DEFAULT_ENV_VARS: &[&str] = &[
    "PATH",
    "PATHEXT",
    "SYSTEMROOT",
    "SYSTEMDRIVE",
    "WINDIR",
    "COMSPEC",
    "USERPROFILE",
    "APPDATA",
    "LOCALAPPDATA",
    "PROGRAMDATA",
    "TEMP",
    "TMP",
    "USERNAME",
    "HOMEDRIVE",
    "HOMEPATH",
];

/// 构造 MCP stdio 子进程命令。
///
/// `cwd` 缺省为 `default_cwd`（项目根）；相对 `cwd` 以项目根为基准。
pub fn mcp_command(
    program: &str,
    args: &[String],
    cwd: Option<&Path>,
    default_cwd: &Path,
    env_vars: &[String],
    env: &BTreeMap<String, String>,
) -> tokio::process::Command {
    let mut command = tokio::process::Command::new(resolve_program(program));
    command.args(args);
    command.env_clear();
    for name in MCP_DEFAULT_ENV_VARS {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
    for name in env_vars {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
    for (name, value) in env {
        command.env(name, value);
    }
    let cwd = match cwd {
        Some(cwd) if cwd.is_absolute() => cwd.to_path_buf(),
        Some(cwd) => default_cwd.join(cwd),
        None => default_cwd.to_path_buf(),
    };
    command.current_dir(cwd);
    #[cfg(unix)]
    {
        // 独立进程组：退出时整组终止，不残留 npx/node 子进程。
        command.process_group(0);
    }
    command
}

/// Windows 下把 `npx` 解析为 `npx.cmd`；其他平台原样返回。
fn resolve_program(program: &str) -> String {
    #[cfg(windows)]
    {
        use std::path::Path;
        let path = Path::new(program);
        if path.extension().is_some() {
            return program.to_string();
        }
        let Some(paths) = std::env::var_os("PATH") else {
            return program.to_string();
        };
        let extensions = std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".into());
        for dir in std::env::split_paths(&paths) {
            for extension in extensions.split(';') {
                let candidate = dir.join(format!("{program}{extension}"));
                if candidate.is_file() {
                    return candidate.to_string_lossy().into_owned();
                }
            }
        }
    }
    program.to_string()
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[tokio::test]
    async fn child_env_is_whitelisted_and_overrides_apply() {
        // 测试内修改变量是进程级操作；用 unsafe 明确表达（生产代码不使用）。
        unsafe { std::env::set_var("LX_MCP_SECRET_TEST", "leaked") };
        let cwd = std::env::temp_dir();
        unsafe { std::env::set_var("LX_MCP_PASS", "forwarded") };
        let env = BTreeMap::from([("LOG_LEVEL".to_string(), "warn".to_string())]);
        let mut command = mcp_command(
            "sh",
            &[
                "-c".to_string(),
                "printf '%s|' \"$LX_MCP_SECRET_TEST\" \"$LX_MCP_PASS\" \"$LOG_LEVEL\" \"$PATH\""
                    .to_string(),
            ],
            None,
            &cwd,
            &["LX_MCP_PASS".to_string()],
            &env,
        );
        let output = command.output().await.expect("run");
        let text = String::from_utf8_lossy(&output.stdout);
        assert!(
            !text.contains("leaked"),
            "未列入白名单的变量不得泄漏：{text}"
        );
        assert!(text.contains("forwarded"));
        assert!(text.contains("warn"));
        assert!(text.contains('/'));
        unsafe {
            std::env::remove_var("LX_MCP_SECRET_TEST");
            std::env::remove_var("LX_MCP_PASS");
        }
    }
}
