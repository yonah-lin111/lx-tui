//! 统一 Web 控制台前端资源：`assets/` 下的 HTML/CSS/JS，经 `include_str!` 编译进二进制。
//!
//! `GET /`、`GET /settings`、`GET /usage`、`GET /todo` 都返回同一份 `web.html`，由前端按
//! `location.pathname` / `location.hash` 选择初始视图；`web.css` / `web.js` 提供
//! 全局外壳、共享核心与路由，`settings*.{css,js}`、`usage*.{css,js}`、`todo*.{css,js}`
//! 仍按视图独立加载。所有资源自包含、无外部依赖，遵循 `docs/standards/html-assets.md`。

/// 统一 Web 控制台正文；`token` 从 `location.search` 读取，所有请求附带。
pub fn html() -> &'static str {
    include_str!("../../../assets/web.html")
}

/// 静态资源：返回 `(content_type, body)`；未知路径返回 None（交由路由回 404）。
///
/// 资源不含敏感内容，故不要求 token；页面与 JSON API 仍逐请求校验。
pub fn asset(path: &str) -> Option<(&'static str, &'static str)> {
    match path {
        "/web.css" => Some((
            "text/css; charset=utf-8",
            include_str!("../../../assets/web.css"),
        )),
        "/web.js" => Some((
            "text/javascript; charset=utf-8",
            include_str!("../../../assets/web.js"),
        )),
        "/settings.css" => Some((
            "text/css; charset=utf-8",
            include_str!("../../../assets/settings.css"),
        )),
        "/settings.js" => Some((
            "text/javascript; charset=utf-8",
            include_str!("../../../assets/settings.js"),
        )),
        "/settings_models.js" => Some((
            "text/javascript; charset=utf-8",
            include_str!("../../../assets/settings_models.js"),
        )),
        "/settings_models_card.js" => Some((
            "text/javascript; charset=utf-8",
            include_str!("../../../assets/settings_models_card.js"),
        )),
        "/settings_connect.js" => Some((
            "text/javascript; charset=utf-8",
            include_str!("../../../assets/settings_connect.js"),
        )),
        "/settings_mcp.js" => Some((
            "text/javascript; charset=utf-8",
            include_str!("../../../assets/settings_mcp.js"),
        )),
        "/settings_skills.js" => Some((
            "text/javascript; charset=utf-8",
            include_str!("../../../assets/settings_skills.js"),
        )),
        "/settings_subagents.js" => Some((
            "text/javascript; charset=utf-8",
            include_str!("../../../assets/settings_subagents.js"),
        )),
        "/usage.css" => Some((
            "text/css; charset=utf-8",
            include_str!("../../../assets/usage.css"),
        )),
        "/usage.js" => Some((
            "text/javascript; charset=utf-8",
            include_str!("../../../assets/usage.js"),
        )),
        "/usage_charts.js" => Some((
            "text/javascript; charset=utf-8",
            include_str!("../../../assets/usage_charts.js"),
        )),
        "/todo.css" => Some((
            "text/css; charset=utf-8",
            include_str!("../../../assets/todo.css"),
        )),
        "/todo.js" => Some((
            "text/javascript; charset=utf-8",
            include_str!("../../../assets/todo.js"),
        )),
        _ => None,
    }
}
