//! 浏览器资源回归：Node.js 可用时检查脚本语法，并校验 JS 与 `web.html` 的契约。
//!
//! Usage 看板曾因 `usage_charts.js` 内重复声明 `const hasAnyCost` 导致整个脚本
//! `SyntaxError`，页面表现为「无法拉取数据」。统一控制台把全部脚本放进同一
//! 全局作用域，因此除了逐文件检查，还必须按 HTML 顺序拼接后再解析一次；同时
//! 守住 `getElementById` 引用的静态 id 必须存在于 `web.html`。Node.js 缺失时
//! 跳过语法检查（不阻断纯 Rust 环境），DOM 契约检查始终执行。

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
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
    "todo_charts.js",
    "todo.js",
    "home.js",
];

fn assets_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("assets")
}

fn read(name: &str) -> String {
    fs::read_to_string(assets_dir().join(name)).expect("asset readable")
}

fn html_scripts(html: &str) -> Vec<String> {
    let mut scripts = Vec::new();
    let mut rest = html;
    while let Some(start) = rest.find("<script src=\"/") {
        rest = &rest[start + "<script src=\"/".len()..];
        let Some(end) = rest.find('"') else { break };
        scripts.push(rest[..end].to_string());
        rest = &rest[end..];
    }
    scripts
}

fn html_ids(html: &str) -> HashSet<String> {
    let mut ids = HashSet::new();
    let mut rest = html;
    while let Some(start) = rest.find("id=\"") {
        rest = &rest[start + "id=\"".len()..];
        let Some(end) = rest.find('"') else { break };
        ids.insert(rest[..end].to_string());
        rest = &rest[end..];
    }
    ids
}

/// 收集 `getElementById('literal')` / `getElementById("literal")` 的静态 id；
/// `getElementById('tab-' + key)` 等拼接形式跳过。
fn literal_element_ids(js: &str) -> Vec<String> {
    let mut ids = Vec::new();
    let mut rest = js;
    while let Some(start) = rest.find("getElementById(") {
        rest = &rest[start + "getElementById(".len()..];
        let Some(quote) = rest.chars().next().filter(|c| *c == '\'' || *c == '"') else {
            continue;
        };
        rest = &rest[quote.len_utf8()..];
        let Some(end) = rest.find(quote) else { break };
        let id = &rest[..end];
        let after = rest[end + quote.len_utf8()..].trim_start();
        if after.starts_with(')') {
            ids.push(id.to_string());
        }
        rest = &rest[end..];
    }
    ids
}

fn node_available() -> bool {
    Command::new("node").arg("--version").output().is_ok()
}

fn node_check(path: &Path, label: &str) {
    let output = Command::new("node")
        .arg("--check")
        .arg(path)
        .output()
        .expect("node --check starts");
    assert!(
        output.status.success(),
        "{label} failed to parse:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn browser_scripts_parse_with_node() {
    let html = read("web.html");
    let linked = html_scripts(&html);
    assert_eq!(
        linked,
        SCRIPTS
            .iter()
            .map(|name| (*name).to_string())
            .collect::<Vec<_>>(),
        "web.html script order matches the shared-global dependency order"
    );

    if !node_available() {
        eprintln!("skipping browser asset parse check: node not found");
        return;
    }

    let assets = assets_dir();
    for name in SCRIPTS {
        node_check(&assets.join(name), name);
    }

    // 全部脚本共享同一全局作用域：拼成一个脚本再解析，跨文件重复 const/let 会在这里失败。
    let combined = SCRIPTS
        .iter()
        .map(|name| read(name))
        .collect::<Vec<_>>()
        .join("\n;\n");
    let temp = std::env::temp_dir().join(format!("lx-web-assets-{}.js", std::process::id()));
    fs::write(&temp, combined).expect("combined browser script written");
    let output = Command::new("node")
        .arg("--check")
        .arg(&temp)
        .output()
        .expect("node --check starts");
    let _ = fs::remove_file(&temp);
    assert!(
        output.status.success(),
        "combined browser scripts failed to parse:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn browser_scripts_reference_existing_ids() {
    let ids = html_ids(&read("web.html"));
    for name in SCRIPTS {
        for id in literal_element_ids(&read(name)) {
            assert!(
                ids.contains(&id),
                "{name} references missing id `{id}` in web.html"
            );
        }
    }
}

#[test]
fn assets_do_not_call_removed_element_helper() {
    assert!(
        read("web.js").contains("function domEl("),
        "web.js must define the shared domEl() helper"
    );
    for name in SCRIPTS {
        assert!(
            !read(name).contains("element("),
            "{name} still calls the removed `element()` helper; use domEl() instead"
        );
    }
}
