//! 浏览器资源语法回归：Node.js 可用时对 `assets/*.js` 执行 `node --check`。
//!
//! Usage 看板曾因 `usage_charts.js` 内重复声明 `const hasAnyCost` 导致整个脚本
//! `SyntaxError`，页面表现为「无法拉取数据」。该测试在 Node.js 存在时守住所有
//! 浏览器脚本的解析基线；CI/开发机没有 Node.js 时跳过，不阻断 Rust 测试。

use std::path::Path;
use std::process::Command;

const SCRIPTS: &[&str] = &[
    "web.js",
    "settings.js",
    "settings_models_card.js",
    "settings_models.js",
    "settings_connect.js",
    "settings_mcp.js",
    "settings_skills.js",
    "settings_subagents.js",
    "usage_charts.js",
    "usage.js",
];

#[test]
fn browser_scripts_parse_with_node() {
    let node = Command::new("node").arg("--version").output();
    if node.is_err() {
        eprintln!("skipping browser asset parse check: node not found");
        return;
    }

    let assets = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets");
    for name in SCRIPTS {
        let path = assets.join(name);
        let output = Command::new("node")
            .arg("--check")
            .arg(&path)
            .output()
            .expect("node --check starts");
        assert!(
            output.status.success(),
            "{} failed to parse:\n{}",
            name,
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
