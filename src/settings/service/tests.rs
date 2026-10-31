use super::tests_helpers::{body, get, request, status, write};
use super::*;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::mpsc;

/// 隔离临时目录。
fn temp_dir(name: &str) -> PathBuf {
    let unique = format!(
        "lx-tui-service-tests-{}-{}-{}",
        name,
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or_default()
    );
    let path = std::env::temp_dir().join(unique);
    std::fs::create_dir_all(&path).expect("temp dir");
    path
}

/// 启动测试服务：注入隔离的 settings.json 路径。
async fn start(
    name: &str,
) -> (
    SettingsService,
    mpsc::UnboundedReceiver<SettingsEvent>,
    PathBuf,
) {
    let dir = temp_dir(name);
    let config_path = dir.join("settings.json");
    let user_root = dir.join("root");
    std::fs::create_dir_all(&user_root).expect("user root");
    let (sender, receiver) = mpsc::unbounded_channel();
    let service = SettingsService::start_in(sender, config_path, user_root)
        .await
        .expect("service starts");
    (service, receiver, dir)
}

#[tokio::test]
async fn page_requires_token_and_serves_self_contained_assets() {
    let (service, _events, _dir) = start("page").await;
    let port = service.port();
    let token = service.token().to_string();

    let denied = request(port, &get("/")).await;
    assert_eq!(status(&denied), 403);

    let ok = request(port, &get(&format!("/?token={token}"))).await;
    assert_eq!(status(&ok), 200);
    let html = body(&ok);
    assert!(html.contains("General"), "tabs present");
    assert!(html.contains("Commands"), "commands tab present");
    assert!(html.contains("Models"), "models tab present");
    assert!(html.contains("view-models"), "models view container");
    assert!(html.contains("provider-list"), "provider list container");
    assert!(html.contains("prov-api-key"), "api key input present");
    assert!(html.contains("MCP"), "mcp tab present");
    assert!(html.contains("view-mcp"), "mcp view container");
    assert!(html.contains("Skills"), "skills tab present");
    assert!(html.contains("view-skills"), "skills view container");
    assert!(
        html.contains("id=\"skill-body\" rows=\"22\""),
        "tall skill body"
    );
    assert!(
        html.contains("type=\"password\""),
        "api key input masked by default"
    );
    assert!(
        html.contains("fetch-candidates"),
        "remote fetch candidates container"
    );
    assert!(html.contains("editor-empty"), "empty placeholder container");
    assert!(html.contains("/settings.css"), "styles linked");
    assert!(html.contains("/settings.js"), "script linked");
    assert!(!html.contains("cmd-preview"), "no preview pane");

    // 静态资源以裸路径提供（浏览器 link/script 不携带 token），且带正确 MIME。
    let css = request(port, &get("/settings.css")).await;
    assert_eq!(status(&css), 200);
    assert!(css.contains("text/css"), "css content type");
    assert!(body(&css).contains("color-scheme: dark"), "dark theme");
    assert!(body(&css).contains(".models-grid"), "models layout styles");
    assert!(
        body(&css).contains("#mcp-editor") && body(&css).contains("#skill-editor"),
        "mcp/skills editors participate in the flex scroll layout"
    );
    assert!(
        body(&css).contains("#skill-body"),
        "skill body height style"
    );

    let js = request(port, &get("/settings.js")).await;
    assert_eq!(status(&js), 200);
    assert!(js.contains("javascript"), "js content type");
    // 动态文本用 textContent 而非 innerHTML 拼接。
    let js_body = body(&js);
    assert!(
        js_body.contains("textContent"),
        "dynamic text via textContent"
    );

    // Models 视图独立资源：加载/保存/复制/删除/远端获取与模态、思考等级入口。
    let models_js = request(port, &get("/settings_models.js")).await;
    assert_eq!(status(&models_js), 200);
    assert!(models_js.contains("javascript"), "models js content type");
    let models_js_body = body(&models_js);
    for entry in [
        "loadModels",
        "saveModels",
        "duplicateProvider",
        "deleteProvider",
        "fetchModels",
        "renderDefaultModel",
        "/api/models/settings",
        "/api/models/fetch",
    ] {
        assert!(models_js_body.contains(entry), "models js entry: {entry}");
    }
    assert!(
        !models_js_body.contains(".innerHTML"),
        "no innerHTML for dynamic data"
    );

    let mcp_js = request(port, &get("/settings_mcp.js")).await;
    assert_eq!(status(&mcp_js), 200);
    let mcp_js_body = body(&mcp_js);
    for entry in ["loadMcp", "saveMcpServer", "mcpProbe", "/api/mcp/"] {
        assert!(mcp_js_body.contains(entry), "mcp js entry: {entry}");
    }
    let skills_js = request(port, &get("/settings_skills.js")).await;
    assert_eq!(status(&skills_js), 200);
    let skills_js_body = body(&skills_js);
    for entry in ["loadSkills", "saveSkill", "/api/skills"] {
        assert!(skills_js_body.contains(entry), "skills js entry: {entry}");
    }

    // 六份资源均无外部引用。
    for (name, text) in [
        ("html", html),
        ("css", body(&css)),
        ("js", js_body),
        ("models_js", models_js_body),
        ("mcp_js", mcp_js_body),
        ("skills_js", skills_js_body),
    ] {
        assert!(
            !text.contains("https://"),
            "{name} has no external resources"
        );
        assert!(
            !text.contains("http://"),
            "{name} has no external resources"
        );
    }
    service.shutdown();
}

#[tokio::test]
async fn unknown_asset_is_not_found() {
    let (service, _events, _dir) = start("asset-404").await;
    let token = service.token().to_string();
    // 非白名单路径不享受静态放行：无 token 先被拒绝。
    let denied = request(service.port(), &get("/settings.map")).await;
    assert_eq!(status(&denied), 403);
    // 带 token 时落到正常路由，未知路径回 404。
    let response = request(
        service.port(),
        &get(&format!("/settings.map?token={token}")),
    )
    .await;
    assert_eq!(status(&response), 404);
    service.shutdown();
}

#[tokio::test]
async fn page_locks_viewport_and_avoids_extra_scrollbars() {
    let (service, _events, _dir) = start("page-layout").await;
    let response = request(service.port(), &get("/settings.css")).await;
    assert_eq!(status(&response), 200);
    let css = body(&response);

    // 根容器锁定视口高度，永不出现根滚动条。
    assert!(
        css.contains("html, body { height: 100%; }"),
        "viewport height locked"
    );
    assert!(
        css.contains("display: flex; overflow: hidden;"),
        "body overflow hidden"
    );
    // 宽屏时命令网格的行高被视口约束，滚动只发生在栏内。
    assert!(
        css.contains("grid-template-rows: minmax(0, 1fr)"),
        "wide grid rows bounded by viewport"
    );
    assert!(
        css.contains("grid-template-rows: max-content max-content"),
        "narrow stacked grid sizes rows to content and scrolls inside"
    );
    // 左栏列表独立滚动但隐藏滚动条；右栏卡片依赖细滚动条兜底。
    assert!(
        css.contains(".command-list::-webkit-scrollbar { display: none; }"),
        "left list scrollbar hidden"
    );
    assert!(
        css.contains(".editor-body::-webkit-scrollbar { width: 8px; }"),
        "editor fallback scrollbar styled"
    );
    // 左栏条目两行排版：第一行斜杠+名称，第二行 tag 与名称左对齐。
    assert!(
        css.contains(".nav-item { display: grid; grid-template-columns: auto minmax(0, 1fr);"),
        "command items stack name over tag"
    );
    assert!(
        css.contains(".nav-item .tag { grid-column: 2; grid-row: 2;"),
        "tag on second line aligned to name"
    );
    assert!(
        css.contains("scrollbar-width: thin"),
        "editor fallback scrollbar thin"
    );
    service.shutdown();
}

#[tokio::test]
async fn models_page_scrolls_and_uses_shared_select_toast() {
    let (service, _events, _dir) = start("page-models-ui").await;
    let port = service.port();
    let token = service.token().to_string();
    let html = body(&request(port, &get(&format!("/?token={token}"))).await).to_string();
    let css = body(&request(port, &get("/settings.css")).await).to_string();
    let js = body(&request(port, &get("/settings.js")).await).to_string();
    let models_js = body(&request(port, &get("/settings_models.js")).await).to_string();
    assert!(
        html.contains("/settings.js") && html.contains("/settings_models.js"),
        "scripts linked"
    );

    // 右栏被视口高度约束，模型很多时编辑容器内出现可见滚动条。
    assert!(
        css.contains(".models-grid { display: grid; flex: 1 1 auto; min-height: 0;"),
        "models grid bounded by viewport"
    );
    assert!(
        css.contains(".editor-body::-webkit-scrollbar-thumb"),
        "editor scrollbar thumb visible"
    );
    assert!(
        css.contains(".provider-list::-webkit-scrollbar"),
        "provider list scrollbar styled"
    );
    // 左侧列表与右侧内容卡之间是可拖拽分隔条（对齐 Commands 页）。
    assert!(
        html.contains("id=\"models-resizer\""),
        "models list resizer present"
    );
    assert!(css.contains(".models-resizer"), "models resizer styled");
    assert!(
        css.contains("--models-list-width, 220px"),
        "models list width variable wired"
    );
    assert!(
        js.contains("modelsListWidth"),
        "models list width clamp wired"
    );

    // 页面上不再有原生下拉，全部换成全局 select 组件挂载点。
    for id in [
        "cmd-filter-select",
        "cmd-scope-select",
        "cmd-kind-select",
        "cmd-md-scope-select",
        "cmd-block-type-select",
        "prov-type-host",
        "default-model-host",
    ] {
        assert!(html.contains(&format!("id=\"{id}\"")), "select mount: {id}");
    }
    assert!(!html.contains("<select"), "no native select left");

    // 全局 toast 与自绘 select 组件入口和样式。
    assert!(js.contains("function showToast"), "toast component");
    assert!(js.contains("function createSelect"), "select component");
    assert!(css.contains(".toast-host"), "toast host styles");
    assert!(css.contains(".select-menu"), "select menu styles");
    // 黑灰分层：色板走变量，悬浮层（toast / select 菜单）比卡片更亮并带阴影层级。
    assert!(css.contains("--bg: #0f0f0f"), "page uses dark gray base");
    assert!(css.contains("--overlay: #272727"), "overlay tone defined");
    assert!(
        css.contains(".select-menu { position: fixed; z-index: var(--z-menu);"),
        "select menu is an elevated fixed layer"
    );
    assert!(
        css.contains("background: var(--overlay); box-shadow: var(--shadow-overlay);"),
        "toast/menu share overlay tone and shadow"
    );
    assert!(
        css.contains("--z-toast: 1000") && css.contains("--z-menu: 60"),
        "toast stacks above menus"
    );
    // 菜单内选项高亮必须比菜单底色更亮，否则 hover/选中在悬浮层上不可见。
    assert!(
        css.contains("--option-hover: #575757") && css.contains("--option-selected: #666666"),
        "option highlight tones above overlay"
    );
    assert!(
        css.contains(
            ".select-option:hover, .select-option.active { background: var(--option-hover); color: #fff; }"
        ),
        "option hover uses elevated tone"
    );
    assert!(
        css.contains("background: var(--option-selected); color: #fff; font-weight: 600;"),
        "selected option marked by tone and weight"
    );
    // 选中项只靠底色 + 字重区分，不再加左侧竖条。
    assert!(
        !css.contains(".select-option.selected::before"),
        "no selected left bar"
    );
    assert!(
        css.contains(".select-option.selected:hover, .select-option.selected.active"),
        "selected keeps tone while hovered"
    );

    // 模态勾选与思考等级编辑入口（Models 视图资源）。
    assert!(
        models_js.contains("INPUT_MODALITIES") && models_js.contains("OUTPUT_MODALITIES"),
        "modality options"
    );
    assert!(
        models_js.contains("Thinking variants"),
        "thinking variants editor"
    );
    assert!(models_js.contains("VARIANT_PRESETS"), "thinking presets");
    assert!(models_js.contains("reasoningEffort"), "OpenAI preset wired");

    // 全局内存草稿 + 视图操作条统一保存 + 未保存状态。
    assert!(models_js.contains("modelsDraft"), "global draft");
    assert!(models_js.contains("modelsBaseline"), "reset baseline");
    assert!(
        models_js.contains("buildPayload"),
        "single save builds full payload"
    );
    assert!(
        js.contains("createActionBar"),
        "shared action bar component"
    );
    assert!(js.contains("Unsaved changes"), "dirty indicator");
    // provider 启停改为编辑卡内复选框，列表行不再有圆点。
    assert!(
        html.contains("id=\"prov-enabled\""),
        "provider enable checkbox"
    );
    assert!(!css.contains(".prov-toggle"), "no toggle dot styles");
    // API key 明文查看：小眼睛按钮 + 切换逻辑。
    assert!(html.contains("id=\"prov-key-reveal\""), "key reveal button");
    assert!(models_js.contains("Show API key"), "reveal labels wired");
    // 已移除 Clear stored key 复选框与 key 状态行；key 由 GET 直接回传编辑。
    assert!(!html.contains("prov-clear-key"), "no clear-key checkbox");
    assert!(!html.contains("prov-key-state"), "no key state line");
    assert!(!html.contains("Clear stored key"), "no clear-key label");
    assert!(
        models_js.contains("provider.apiKey"),
        "key round-trips via API"
    );
    // toast 顶部居中。
    assert!(
        css.contains("left: 50%; top: 16px;"),
        "toast anchored top center"
    );
    service.shutdown();
}

#[tokio::test]
async fn page_header_actions_fill_width_and_resizers() {
    let (service, _events, _dir) = start("page-resize").await;
    let port = service.port();
    let token = service.token().to_string();
    let html = body(&request(port, &get(&format!("/?token={token}"))).await).to_string();
    let css = body(&request(port, &get("/settings.css")).await).to_string();
    let js = body(&request(port, &get("/settings.js")).await).to_string();

    // 保存入口统一到每个视图标题行右侧的全局操作条；页面内不再有独立保存按钮。
    for id in ["actions-general", "actions-commands", "actions-models"] {
        assert!(
            html.contains(&format!("id=\"{id}\"")),
            "action bar host: {id}"
        );
    }
    assert!(!html.contains("saveCommand()"), "no per-editor save button");
    assert!(!html.contains("saveConfig()"), "no general save button");
    assert!(!html.contains("saveModels()"), "no models save button");
    assert!(
        css.contains(".action-status") && css.contains(".status-dot"),
        "action bar status styles"
    );
    // 编辑器头部保留标题与字段。
    let legend = html.find("id=\"editor-legend\"").expect("legend present");
    let first_field = html.find("id=\"cmd-name\"").expect("name field present");
    assert!(legend < first_field, "editor head above fields");

    // 内容撑满：不再有 max-width 上限。
    assert!(
        !css.contains("max-width: 1440px"),
        "main fills available width"
    );

    // 两个拖拽分隔条与钳制常量。
    assert!(
        html.contains("id=\"side-resizer\""),
        "sidebar resizer present"
    );
    assert!(
        html.contains("id=\"cmd-resizer\""),
        "command list resizer present"
    );
    assert!(css.contains("cursor: col-resize"), "resize cursor styled");
    assert!(
        css.contains("--cmd-list-width, 220px"),
        "command list width variable wired"
    );
    assert!(
        js.contains("pointerdown"),
        "drag handled via pointer events"
    );
    assert!(
        js.contains("LIST_MAX_WIDTH = 560"),
        "list width clamp present"
    );
    // 编辑器字段两列排布，窄屏回落单列。
    assert!(
        css.contains("grid-template-columns: repeat(2, minmax(0, 1fr))"),
        "editor fields two per row"
    );
    assert!(
        css.contains(".field-grid { grid-template-columns: 1fr; }"),
        "editor fields collapse to one column when narrow"
    );
    service.shutdown();
}

#[tokio::test]
async fn page_editor_supports_rename_flow() {
    let (service, _events, _dir) = start("page-rename").await;
    let port = service.port();
    let token = service.token().to_string();

    // HTML/JS 层：Name 输入不再被禁用；保存时先调 rename 再 PUT。
    let page = request(port, &get(&format!("/?token={token}"))).await;
    assert_eq!(status(&page), 200);
    let html = body(&page).to_string();
    let css = body(&request(port, &get("/settings.css")).await).to_string();
    let js = body(&request(port, &get("/settings.js")).await).to_string();
    assert!(!html.contains("disabled"), "name input never disabled");
    assert!(!css.contains(".disabled"), "no disabled styling");
    assert!(js.contains("'/rename'"), "rename endpoint wired");
    assert!(
        js.contains("JSON.stringify({to: name})"),
        "rename payload wired"
    );

    // 协议层：复刻前端保存序列（先 rename 后 PUT 新路径），内容跟随新名。
    let origin = Some(format!("http://127.0.0.1:{port}"));
    let payload = "{\"description\":\"d\",\"md_scope\":\"global\",\"body\":\"first\"}";
    let created = request(
        port,
        &write(
            "PUT",
            &format!("/api/commands/user/agentMD/alpha?token={token}"),
            payload,
            origin.as_deref(),
        ),
    )
    .await;
    assert_eq!(status(&created), 200, "{created}");
    let renamed = request(
        port,
        &write(
            "POST",
            &format!("/api/commands/user/agentMD/alpha/rename?token={token}"),
            "{\"to\":\"beta\"}",
            origin.as_deref(),
        ),
    )
    .await;
    assert_eq!(status(&renamed), 200, "{renamed}");
    let saved = request(
        port,
        &write(
            "PUT",
            &format!("/api/commands/user/agentMD/beta?token={token}"),
            "{\"description\":\"d\",\"md_scope\":\"global\",\"body\":\"second\"}",
            origin.as_deref(),
        ),
    )
    .await;
    assert_eq!(status(&saved), 200, "{saved}");
    let state = request(port, &get(&format!("/api/state?token={token}"))).await;
    let state_body = body(&state);
    assert!(state_body.contains("\"name\":\"beta\""), "{state_body}");
    assert!(!state_body.contains("\"name\":\"alpha\""), "{state_body}");
    assert!(state_body.contains("second"), "{state_body}");

    // 改到已存在的名字必须失败，主流程因此不会覆盖另一条命令。
    let created = request(
        port,
        &write(
            "PUT",
            &format!("/api/commands/user/agentMD/gamma?token={token}"),
            payload,
            origin.as_deref(),
        ),
    )
    .await;
    assert_eq!(status(&created), 200, "{created}");
    let conflict = request(
        port,
        &write(
            "POST",
            &format!("/api/commands/user/agentMD/beta/rename?token={token}"),
            "{\"to\":\"gamma\"}",
            origin.as_deref(),
        ),
    )
    .await;
    assert_eq!(status(&conflict), 422, "{conflict}");
    let state = request(port, &get(&format!("/api/state?token={token}"))).await;
    assert!(body(&state).contains("\"name\":\"beta\""), "beta kept");
    service.shutdown();
}

#[tokio::test]
async fn state_returns_config_defaults() {
    let (service, _events, _dir) = start("state").await;
    let token = service.token().to_string();
    let response = request(service.port(), &get(&format!("/api/state?token={token}"))).await;
    assert_eq!(status(&response), 200);
    let json = body(&response);
    assert!(json.contains("\"sidebar_width\":24"), "{json}");
    assert!(json.contains("\"commands\":[]"), "{json}");
    service.shutdown();
}

#[tokio::test]
async fn config_save_validates_origin_and_values() {
    let (service, mut events, dir) = start("config").await;
    let port = service.port();
    let token = service.token().to_string();
    let url = format!("/api/config?token={token}");

    // 跨源写入被拒绝。
    let cross = request(
        port,
        &write(
            "PUT",
            &url,
            "{\"config\":{\"sidebar_width\":30}}",
            Some("http://evil.example"),
        ),
    )
    .await;
    assert_eq!(status(&cross), 403);

    // 非法值拒绝且不落盘。
    let invalid = request(
        port,
        &write(
            "PUT",
            &url,
            "{\"config\":{\"sidebar_width\":-3}}",
            Some("http://127.0.0.1"),
        ),
    )
    .await;
    assert_eq!(status(&invalid), 422);
    assert!(
        !dir.join("settings.json").exists(),
        "invalid config not written"
    );

    // 合法值保存并投递事件。
    let valid = request(
        port,
        &write(
            "PUT",
            &url,
            "{\"config\":{\"sidebar_width\":30,\"sidebar_min_width\":18,\"sidebar_max_width\":40,\"narrow_width\":80,\"min_width\":40,\"min_height\":10,\"min_pane_width\":10,\"min_pane_height\":3}}",
            Some(&format!("http://127.0.0.1:{port}")),
        ),
    )
    .await;
    assert_eq!(status(&valid), 200);
    let saved = std::fs::read_to_string(dir.join("settings.json")).expect("settings written");
    assert!(saved.contains("\"sidebar_width\": 30"), "{saved}");
    let event = events.try_recv().expect("config event");
    match event {
        SettingsEvent::ConfigChanged(config) => assert_eq!(config.sidebar_width, 30),
        other => panic!("unexpected event: {other:?}"),
    }
    service.shutdown();
}

#[tokio::test]
async fn command_crud_over_http_and_snapshot() {
    let (service, mut events, _dir) = start("commands").await;
    let port = service.port();
    let token = service.token().to_string();
    let save_url = format!("/api/commands/user/agentMD/review?token={token}");
    let payload = "{\"description\":\"review diff\",\"md_scope\":\"template\",\"body\":\"# Review\\ncheck it\"}";
    let saved = request(
        port,
        &write(
            "PUT",
            &save_url,
            payload,
            Some(&format!("http://127.0.0.1:{port}")),
        ),
    )
    .await;
    assert_eq!(status(&saved), 200, "{saved}");
    match events.try_recv() {
        Ok(SettingsEvent::CommandsChanged) => {}
        other => panic!("expected CommandsChanged, got {other:?}"),
    }

    // 保存后无需 TUI 推送：服务端已按磁盘刷新快照。
    let state = request(service.port(), &get(&format!("/api/state?token={token}"))).await;
    assert!(
        body(&state).contains("\"name\":\"review\""),
        "{}",
        body(&state)
    );

    // 非法名与保留名被拒绝。
    let bad = request(
        port,
        &write(
            "PUT",
            &format!("/api/commands/user/agentMD/..%2Fescape?token={token}"),
            payload,
            Some(&format!("http://127.0.0.1:{port}")),
        ),
    )
    .await;
    assert!(status(&bad) == 400 || status(&bad) == 404, "{bad}");
    let reserved = request(
        port,
        &write(
            "PUT",
            &format!("/api/commands/user/agentMD/add?token={token}"),
            payload,
            Some(&format!("http://127.0.0.1:{port}")),
        ),
    )
    .await;
    assert_eq!(status(&reserved), 400);

    // 重命名与删除。
    let renamed = request(
        port,
        &write(
            "POST",
            &format!("/api/commands/user/agentMD/review/rename?token={token}"),
            "{\"to\":\"review2\"}",
            Some(&format!("http://127.0.0.1:{port}")),
        ),
    )
    .await;
    assert_eq!(status(&renamed), 200, "{renamed}");
    let deleted = request(
        port,
        &write(
            "DELETE",
            &format!("/api/commands/user/agentMD/review2?token={token}"),
            "",
            Some(&format!("http://127.0.0.1:{port}")),
        ),
    )
    .await;
    assert_eq!(status(&deleted), 200, "{deleted}");
    let missing = request(
        port,
        &write(
            "DELETE",
            &format!("/api/commands/user/agentMD/review2?token={token}"),
            "",
            Some(&format!("http://127.0.0.1:{port}")),
        ),
    )
    .await;
    assert_eq!(status(&missing), 404);
    // 删除后无需 TUI 推送：快照同步移除。
    let after = request(service.port(), &get(&format!("/api/state?token={token}"))).await;
    assert!(
        !body(&after).contains("\"name\":\"review\""),
        "{}",
        body(&after)
    );
    service.shutdown();
}

#[tokio::test]
async fn oversized_body_is_rejected_and_service_survives() {
    let (service, _events, _dir) = start("oversize").await;
    let port = service.port();
    let token = service.token().to_string();
    let huge = "x".repeat(MAX_BODY_BYTES + 1);
    let raw = format!(
        "PUT /api/commands/user/agentMD/huge?token={token} HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: {}\r\n\r\n{huge}",
        huge.len()
    );
    let mut stream = TcpStream::connect(("127.0.0.1", port))
        .await
        .expect("connect");
    let _ = stream.write_all(raw.as_bytes()).await;
    let mut out = String::new();
    let _ = tokio::time::timeout(Duration::from_secs(2), stream.read_to_string(&mut out)).await;
    if !out.is_empty() {
        assert_ne!(status(&out), 200);
    }
    // 服务仍可响应正常请求。
    let alive = request(port, &get(&format!("/?token={token}"))).await;
    assert_eq!(status(&alive), 200);
    service.shutdown();
}

#[tokio::test]
async fn shutdown_stops_listening() {
    let (service, _events, _dir) = start("shutdown").await;
    let port = service.port();
    service.shutdown();
    let mut closed = false;
    for _ in 0..20 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        if TcpStream::connect(("127.0.0.1", port)).await.is_err() {
            closed = true;
            break;
        }
    }
    assert!(closed, "listener stops after shutdown");
}

#[tokio::test]
async fn agent_block_supple_type_round_trips_over_http() {
    let (service, _events, _dir) = start("commands-supple").await;
    let port = service.port();
    let token = service.token().to_string();
    let origin = Some(format!("http://127.0.0.1:{port}"));
    let url = format!("/api/commands/user/agentBlock/note?token={token}");
    let payload = "{\"description\":\"n\",\"md_scope\":\"template\",\"block_type\":\"supple\",\"title\":\"T\",\"body\":\"+++ note --start\\nx\\n+++ note --end\"}";
    let saved = request(port, &write("PUT", &url, payload, origin.as_deref())).await;
    assert_eq!(status(&saved), 200, "{saved}");

    let state = request(port, &get(&format!("/api/state?token={token}"))).await;
    let body = body(&state);
    assert!(body.contains("\"block_type\":\"supple\""), "{body}");

    // 非法 block_type 被拒绝。
    let bad = request(
        port,
        &write(
            "PUT",
            &format!("/api/commands/user/agentBlock/bad?token={token}"),
            "{\"body\":\"x\",\"block_type\":\"bogus\"}",
            origin.as_deref(),
        ),
    )
    .await;
    assert_eq!(status(&bad), 422, "{bad}");
    service.shutdown();
}

#[tokio::test]
async fn mcp_api_desensitizes_tokens_and_keeps_existing_secret() {
    let (service, _events, _dir) = start("mcp-api").await;
    let port = service.port();
    let token = service.token().to_string();
    let root = service.user_root_for_tests();
    let project_root = root.join("project");
    std::fs::create_dir_all(&project_root).expect("project");
    service.set_projects(vec![ProjectView {
        scope: crate::platform::path_text::scope_dir_name(&project_root).expect("scope"),
        root: project_root,
        name: "project".into(),
        trusted: true,
    }]);
    let config_dir = root.join("config");
    std::fs::create_dir_all(&config_dir).expect("config dir");
    std::fs::write(
        config_dir.join("mcp.json"),
        r#"{"mcp":{"servers":{"github":{"enabled":true,"transport":"http","url":"https://mcp.example/mcp","auth":"bearer","bearer_token":"secret-token"}}}}"#,
    )
    .expect("write mcp");

    let response = request(port, &get(&format!("/api/mcp?token={token}"))).await;
    assert_eq!(status(&response), 200, "{response}");
    let body = body(&response);
    assert!(
        !body.contains("secret-token"),
        "token must not be returned: {body}"
    );
    assert!(
        body.contains("\"bearer_token\":true"),
        "secret flag: {body}"
    );
    assert!(body.contains("\"scope\":\"user\""), "scope: {body}");

    // 空串表示保留原 token。
    let put = request(
        port,
        &write(
            "PUT",
            &format!("/api/mcp/user/github?token={token}"),
            r#"{"enabled":true,"transport":"http","url":"https://mcp.example/mcp","auth":"bearer","bearer_token":""}"#,
            None,
        ),
    )
    .await;
    assert_eq!(status(&put), 200, "{put}");
    let saved = std::fs::read_to_string(config_dir.join("mcp.json")).expect("read mcp");
    assert!(saved.contains("secret-token"), "token kept: {saved}");

    // 切到 auth=none 时必须清掉旧的 bearer 字段，否则校验会报二选一冲突。
    let switch = request(
        port,
        &write(
            "PUT",
            &format!("/api/mcp/user/github?token={token}"),
            r#"{"enabled":true,"transport":"http","url":"https://mcp.example/mcp","auth":"none"}"#,
            None,
        ),
    )
    .await;
    assert_eq!(status(&switch), 200, "{switch}");
    let saved = std::fs::read_to_string(config_dir.join("mcp.json")).expect("read mcp");
    assert!(!saved.contains("secret-token"), "token cleared: {saved}");
    service.shutdown();
}

#[tokio::test]
async fn mcp_project_scope_requires_trust_and_known_scope() {
    let (service, _events, _dir) = start("mcp-scope").await;
    let port = service.port();
    let token = service.token().to_string();
    let root = service.user_root_for_tests();
    let project_root = root.join("untrusted");
    std::fs::create_dir_all(&project_root).expect("project");
    let scope = crate::platform::path_text::scope_dir_name(&project_root).expect("scope");
    service.set_projects(vec![ProjectView {
        scope: scope.clone(),
        root: project_root,
        name: "project".into(),
        trusted: false,
    }]);
    let put = request(
        port,
        &write(
            "PUT",
            &format!("/api/mcp/{scope}/x?token={token}"),
            r#"{"enabled":true,"transport":"stdio","command":"true"}"#,
            None,
        ),
    )
    .await;
    assert_eq!(status(&put), 403, "{put}");

    let unknown = request(
        port,
        &write(
            "PUT",
            &format!("/api/mcp/nope/x?token={token}"),
            r#"{"enabled":true,"transport":"stdio","command":"true"}"#,
            None,
        ),
    )
    .await;
    assert_eq!(status(&unknown), 404, "{unknown}");
    service.shutdown();
}

#[tokio::test]
async fn skills_api_lists_creates_disables_and_deletes() {
    let (service, _events, _dir) = start("skills-api").await;
    let port = service.port();
    let token = service.token().to_string();
    let root = service.user_root_for_tests();

    let put = request(
        port,
        &write(
            "PUT",
            &format!("/api/skills/user/demo?token={token}"),
            r##"{"description":"Do demo things","body":"# Demo\n"}"##,
            None,
        ),
    )
    .await;
    assert_eq!(status(&put), 200, "{put}");
    let path = root.join("skills").join("demo").join("SKILL.md");
    assert!(path.is_file(), "skill written");

    let list = request(port, &get(&format!("/api/skills?token={token}"))).await;
    assert_eq!(status(&list), 200, "{list}");
    let listing = body(&list);
    assert!(listing.contains("\"name\":\"demo\""), "{listing}");
    assert!(listing.contains("\"editable\":true"), "{listing}");

    let canonical = std::fs::canonicalize(&path).expect("canonical");
    let disabled = request(
        port,
        &write(
            "PUT",
            &format!("/api/skills/enabled?token={token}"),
            &format!("{{\"path\":\"{}\",\"enabled\":false}}", canonical.display()),
            None,
        ),
    )
    .await;
    assert_eq!(status(&disabled), 200, "{disabled}");
    let config = std::fs::read_to_string(root.join("config").join("skills.json")).expect("config");
    assert!(config.contains("demo"), "{config}");

    let detail = request(
        port,
        &get(&format!(
            "/api/skills/detail?path={}&token={token}",
            canonical.display()
        )),
    )
    .await;
    assert_eq!(status(&detail), 200, "{detail}");
    assert!(body(&detail).contains("# Demo"), "{detail}");

    let delete = request(
        port,
        &write(
            "DELETE",
            &format!("/api/skills/user/demo?token={token}"),
            "{}",
            None,
        ),
    )
    .await;
    assert_eq!(status(&delete), 200, "{delete}");
    assert!(!path.exists(), "skill deleted");
    service.shutdown();
}

#[tokio::test]
async fn oauth_callback_rejects_unknown_state_without_token() {
    let (service, _events, _dir) = start("oauth-callback").await;
    let response = request(
        service.port(),
        &get("/mcp/oauth/callback?code=abc&state=unknown"),
    )
    .await;
    assert_eq!(status(&response), 400, "{response}");
    assert!(
        body(&response).contains("state expired or invalid"),
        "{response}"
    );
    service.shutdown();
}

#[tokio::test]
async fn page_preserves_subtab_drafts_and_dirty_dots_across_views() {
    let (service, _events, _dir) = start("subtab-drafts").await;
    let port = service.port();
    let token = service.token().to_string();

    let html = body(&request(port, &get(&format!("/?token={token}"))).await).to_string();
    let css = body(&request(port, &get("/settings.css")).await).to_string();
    let js = body(&request(port, &get("/settings.js")).await).to_string();
    let models_js = body(&request(port, &get("/settings_models.js")).await).to_string();
    let mcp_js = body(&request(port, &get("/settings_mcp.js")).await).to_string();
    let skills_js = body(&request(port, &get("/settings_skills.js")).await).to_string();

    assert!(
        css.contains(".nav-item.dirty::after") && css.contains("grid-column: 3; grid-row: 1;"),
        "sub-tab dirty dot styled on right column"
    );
    for hook in [
        "loadGeneral()",
        "reloadCommands(true)",
        "loadMcp(true)",
        "loadSkills(true)",
        "cancelProvider()",
    ] {
        assert!(html.contains(hook), "html isolated action hook: {hook}");
    }
    for symbol in [
        "commandBaselines",
        "commandDrafts",
        "newCommandDraft",
        "syncCommandEditorToDraft",
        "isCommandItemDirty",
        "isNewCommandDirty",
        "updateCommandDirtyDots",
    ] {
        assert!(js.contains(symbol), "commands draft symbol: {symbol}");
    }
    for symbol in [
        "isProviderDirty",
        "computeModelsDirty",
        "updateProviderDirtyDots",
        "cancelProvider",
    ] {
        assert!(models_js.contains(symbol), "models draft symbol: {symbol}");
    }
    for symbol in [
        "mcpBaselines",
        "mcpDrafts",
        "mcpNewDraft",
        "syncMcpEditorToDraft",
        "isMcpItemDirty",
        "isMcpNewDirty",
        "updateMcpDirtyDots",
    ] {
        assert!(mcp_js.contains(symbol), "mcp draft symbol: {symbol}");
    }
    for symbol in [
        "skillBaselines",
        "skillDrafts",
        "skillNewDraft",
        "syncSkillEditorToDraft",
        "isSkillItemDirty",
        "isSkillNewDirty",
        "updateSkillDirtyDots",
    ] {
        assert!(skills_js.contains(symbol), "skills draft symbol: {symbol}");
    }
    service.shutdown();
}
