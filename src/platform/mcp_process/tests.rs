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
