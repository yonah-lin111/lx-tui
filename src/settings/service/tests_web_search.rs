//! 设置页 Web Search 测试：页面字段、`PUT /api/config` 原子校验与 `ai.json` 修复。

use super::tests::start;
use super::tests_helpers::{body, get, request, status, write};
use super::*;

#[tokio::test]
async fn web_search_settings_page_exposes_fields_and_script() {
    let (service, _events, _dir) = start("web-search-page").await;
    let port = service.port();
    let token = service.token().to_string();

    let html = body(&request(port, &get(&format!("/?token={token}"))).await).to_string();
    for entry in [
        "Web Search",
        "id=\"web_search_exa_api_key\"",
        "id=\"web_search_tavily_api_key\"",
        "id=\"exa-key-reveal\"",
        "id=\"tavily-key-reveal\"",
    ] {
        assert!(html.contains(entry), "web search field: {entry}");
    }
    // 与 Models 视图的 Key 输入框同构：密码框 + 小眼睛按钮。
    let key_inputs = html
        .split("id=\"web_search_exa_api_key\"")
        .nth(1)
        .expect("exa input present");
    assert!(
        key_inputs.starts_with(" type=\"password\""),
        "exa key masked by default"
    );

    let js = body(&request(port, &get("/settings.js")).await).to_string();
    assert!(js.contains("WEB_SEARCH_KEYS"), "web search keys wired");
    assert!(
        js.contains("bindPasswordReveal") && js.contains("exa-key-reveal"),
        "reveal handlers wired"
    );
    // 明文态必须在数据重填 / Reset / 切页时收回。
    assert!(
        js.contains("function resetWebSearchReveals"),
        "reveal reset helper wired"
    );
    assert!(
        js.contains("resetWebSearchReveals();"),
        "reveal reset invoked"
    );
    assert!(
        js.contains("exa_api_key") && js.contains("tavily_api_key"),
        "payload fields wired"
    );
    assert!(js.contains("web_search"), "save payload wired");
    service.shutdown();
}

#[tokio::test]
async fn web_search_state_and_config_round_trip_preserves_models() {
    let (service, mut events, _dir) = start("web-search-api").await;
    let port = service.port();
    let token = service.token().to_string();
    let origin = Some(format!("http://127.0.0.1:{port}"));
    let url = format!("/api/config?token={token}");

    // 无 ai.json 时 GET /api/state 返回内置默认 Key。
    let state = body(&request(port, &get(&format!("/api/state?token={token}"))).await).to_string();
    assert!(
        state.contains(&format!(
            "\"exa_api_key\":\"{}\"",
            crate::config::ai::DEFAULT_EXA_API_KEY
        )),
        "{state}"
    );
    assert!(
        state.contains(&format!(
            "\"tavily_api_key\":\"{}\"",
            crate::config::ai::DEFAULT_TAVILY_API_KEY
        )),
        "{state}"
    );

    // 预置一份含 provider 的模型配置，模拟用户已有模型设置。
    let root = service.user_root_for_tests();
    let ai_path = root.join("config").join("ai.json");
    std::fs::create_dir_all(ai_path.parent().expect("config dir")).expect("config dir");
    std::fs::write(
        &ai_path,
        r#"{"ai":{"enabled_providers":["edge"],"defaultModel":{"provider":"edge","model":"edge-chat"},"providers":{"edge":{"id":"edge","type":"openai-compatible","name":"Edge","options":{"apiKey":"keep-me","baseURL":"https://api.edge.invalid/v1"},"models":{"edge-chat":{"id":"edge-chat","name":"Edge Chat"}}}}}}"#,
    )
    .expect("seed ai.json");

    // PUT 同时带 config 与 web_search：Key 做 trim 后落盘。
    let saved = request(
        port,
        &write(
            "PUT",
            &url,
            "{\"config\":{\"sidebar_width\":30},\"web_search\":{\"exa_api_key\":\"  new-exa  \",\"tavily_api_key\":\"new-tavily\"}}",
            origin.as_deref(),
        ),
    )
    .await;
    assert_eq!(status(&saved), 200, "{saved}");
    match events.try_recv() {
        Ok(SettingsEvent::ConfigChanged(config)) => assert_eq!(config.sidebar_width, 30),
        other => panic!("expected ConfigChanged, got {other:?}"),
    }
    match events.try_recv() {
        Ok(SettingsEvent::ModelsChanged(ai)) => {
            assert_eq!(ai.web_search.exa_api_key, "new-exa");
            assert_eq!(ai.web_search.tavily_api_key, "new-tavily");
            assert!(ai.providers.contains_key("edge"), "providers preserved");
        }
        other => panic!("expected ModelsChanged, got {other:?}"),
    }
    let text = std::fs::read_to_string(&ai_path).expect("read ai.json");
    assert!(text.contains("\"webSearch\""), "{text}");
    assert!(text.contains("\"exaApiKey\": \"new-exa\""), "{text}");
    assert!(text.contains("\"tavilyApiKey\": \"new-tavily\""), "{text}");
    // 既有模型配置不能被 General 页的保存覆盖。
    assert!(text.contains("keep-me"), "{text}");
    assert!(text.contains("edge-chat"), "{text}");
    assert!(text.contains("\"enabled_providers\": ["), "{text}");

    // 只带其中一个 Key：另一个保持磁盘上的当前值。
    let partial = request(
        port,
        &write(
            "PUT",
            &url,
            "{\"config\":{\"sidebar_width\":30},\"web_search\":{\"exa_api_key\":\"exa-only\"}}",
            origin.as_deref(),
        ),
    )
    .await;
    assert_eq!(status(&partial), 200, "{partial}");
    match events.try_recv() {
        Ok(SettingsEvent::ConfigChanged(config)) => assert_eq!(config.sidebar_width, 30),
        other => panic!("expected ConfigChanged, got {other:?}"),
    }
    match events.try_recv() {
        Ok(SettingsEvent::ModelsChanged(ai)) => {
            assert_eq!(ai.web_search.exa_api_key, "exa-only");
            assert_eq!(
                ai.web_search.tavily_api_key, "new-tavily",
                "缺失字段保持当前值"
            );
        }
        other => panic!("expected ModelsChanged, got {other:?}"),
    }
    let text = std::fs::read_to_string(&ai_path).expect("re-read after partial");
    assert!(text.contains("\"tavilyApiKey\": \"new-tavily\""), "{text}");

    // 不带 web_search 的旧请求仍然可用，且不改动 ai.json。
    let config_only = request(
        port,
        &write(
            "PUT",
            &url,
            "{\"config\":{\"sidebar_width\":26}}",
            origin.as_deref(),
        ),
    )
    .await;
    assert_eq!(status(&config_only), 200, "{config_only}");
    match events.try_recv() {
        Ok(SettingsEvent::ConfigChanged(config)) => assert_eq!(config.sidebar_width, 26),
        other => panic!("expected ConfigChanged, got {other:?}"),
    }
    assert!(
        events.try_recv().is_err(),
        "no ModelsChanged without web_search"
    );
    assert_eq!(
        std::fs::read_to_string(&ai_path).expect("re-read"),
        text,
        "config-only save leaves ai.json untouched"
    );

    // 显式清空 Key：空字符串原样保留，不回落到默认值。
    let cleared = request(
        port,
        &write(
            "PUT",
            &url,
            "{\"config\":{\"sidebar_width\":26},\"web_search\":{\"exa_api_key\":\"\",\"tavily_api_key\":\"\"}}",
            origin.as_deref(),
        ),
    )
    .await;
    assert_eq!(status(&cleared), 200, "{cleared}");
    match events.try_recv() {
        Ok(SettingsEvent::ConfigChanged(_)) => {}
        other => panic!("expected ConfigChanged, got {other:?}"),
    }
    match events.try_recv() {
        Ok(SettingsEvent::ModelsChanged(ai)) => {
            assert_eq!(ai.web_search.exa_api_key, "");
            assert_eq!(ai.web_search.tavily_api_key, "");
        }
        other => panic!("expected ModelsChanged, got {other:?}"),
    }
    let state = body(&request(port, &get(&format!("/api/state?token={token}"))).await).to_string();
    assert!(state.contains("\"exa_api_key\":\"\""), "{state}");
    assert!(state.contains("\"tavily_api_key\":\"\""), "{state}");
    service.shutdown();
}

/// 非法 `web_search` 一律 422：非对象、Key 非字符串、Key 含控制字符；
/// 且失败请求不写 `settings.json`、不创建 `ai.json`、不投递事件。
#[tokio::test]
async fn put_config_rejects_invalid_web_search_without_side_effects() {
    let (service, mut events, dir) = start("web-search-invalid").await;
    let port = service.port();
    let token = service.token().to_string();
    let url = format!("/api/config?token={token}");
    let origin = Some(format!("http://127.0.0.1:{port}"));

    // 先落一份合法配置：失败请求必须让它保持不变。
    let saved = request(
        port,
        &write(
            "PUT",
            &url,
            "{\"config\":{\"sidebar_width\":26}}",
            origin.as_deref(),
        ),
    )
    .await;
    assert_eq!(status(&saved), 200, "{saved}");
    assert!(events.try_recv().is_ok(), "initial ConfigChanged");
    let before = std::fs::read_to_string(dir.join("settings.json")).expect("settings written");
    let ai_path = service.user_root_for_tests().join("config").join("ai.json");

    for (name, payload) in [
        (
            "non-object",
            "{\"config\":{\"sidebar_width\":30},\"web_search\":\"not-an-object\"}",
        ),
        (
            "non-string key",
            "{\"config\":{\"sidebar_width\":30},\"web_search\":{\"exa_api_key\":42}}",
        ),
        (
            "control characters",
            "{\"config\":{\"sidebar_width\":30},\"web_search\":{\"exa_api_key\":\"bad\\u0007key\"}}",
        ),
    ] {
        let response = request(port, &write("PUT", &url, payload, origin.as_deref())).await;
        assert_eq!(status(&response), 422, "{name}: {response}");
        assert!(
            events.try_recv().is_err(),
            "{name}: no event after rejection"
        );
        assert_eq!(
            std::fs::read_to_string(dir.join("settings.json")).expect("settings kept"),
            before,
            "{name}: settings.json untouched"
        );
        assert!(!ai_path.exists(), "{name}: ai.json untouched");
    }

    // 非对象字段的报错要明确指出字段名，便于前端定位。
    let response = request(
        port,
        &write(
            "PUT",
            &url,
            "{\"config\":{\"sidebar_width\":30},\"web_search\":[]}",
            origin.as_deref(),
        ),
    )
    .await;
    assert_eq!(status(&response), 422, "{response}");
    assert!(
        body(&response).contains("expected object for field web_search"),
        "{response}"
    );
    service.shutdown();
}

/// 磁盘上 `ai.json` 损坏时，保存 Key 必须以空配置为基底重写该文件（200），
/// 而不是像旧行为那样 422 让用户无法修复。
#[tokio::test]
async fn put_config_repairs_malformed_ai_json() {
    let (service, mut events, _dir) = start("web-search-repair").await;
    let port = service.port();
    let token = service.token().to_string();
    let url = format!("/api/config?token={token}");
    let origin = Some(format!("http://127.0.0.1:{port}"));
    let ai_path = service.user_root_for_tests().join("config").join("ai.json");
    std::fs::create_dir_all(ai_path.parent().expect("config dir")).expect("config dir");
    std::fs::write(&ai_path, "{ this is not json").expect("seed broken ai.json");

    let response = request(
        port,
        &write(
            "PUT",
            &url,
            "{\"config\":{\"sidebar_width\":28},\"web_search\":{\"exa_api_key\":\" repaired-exa \",\"tavily_api_key\":\"repaired-tavily\"}}",
            origin.as_deref(),
        ),
    )
    .await;
    assert_eq!(status(&response), 200, "{response}");
    match events.try_recv() {
        Ok(SettingsEvent::ConfigChanged(config)) => assert_eq!(config.sidebar_width, 28),
        other => panic!("expected ConfigChanged, got {other:?}"),
    }
    match events.try_recv() {
        Ok(SettingsEvent::ModelsChanged(ai)) => {
            assert_eq!(ai.web_search.exa_api_key, "repaired-exa");
            assert_eq!(ai.web_search.tavily_api_key, "repaired-tavily");
        }
        other => panic!("expected ModelsChanged, got {other:?}"),
    }
    let text = std::fs::read_to_string(&ai_path).expect("repaired ai.json");
    let parsed: serde_json::Value = serde_json::from_str(&text).expect("valid json after repair");
    assert_eq!(
        parsed["ai"]["webSearch"]["exaApiKey"],
        serde_json::json!("repaired-exa"),
        "{text}"
    );
    assert_eq!(
        parsed["ai"]["webSearch"]["tavilyApiKey"],
        serde_json::json!("repaired-tavily"),
        "{text}"
    );
    // 修复后 GET /api/state 能看到新 Key。
    let state = body(&request(port, &get(&format!("/api/state?token={token}"))).await).to_string();
    assert!(
        state.contains("\"exa_api_key\":\"repaired-exa\""),
        "{state}"
    );
    service.shutdown();
}
