//! 设置页前端资源：`assets/` 下的 HTML/CSS/JS，经 `include_str!` 编译进二进制。
//!
//! 页面与静态资源分离：`GET /` 返回 `settings.html`，`GET /settings.css`、
//! `GET /settings.js` 返回对应资源。所有资源自包含、无外部依赖，遵循
//! `docs/standards/html-assets.md`。

/// 设置页正文；`token` 从 `location.search` 读取，所有请求附带。
pub fn html() -> &'static str {
    include_str!("../../../assets/settings.html")
}

/// 静态资源：返回 `(content_type, body)`；未知路径返回 None（交由路由回 404）。
///
/// 资源不含敏感内容，故不要求 token；页面与 JSON API 仍逐请求校验。
pub fn asset(path: &str) -> Option<(&'static str, &'static str)> {
    match path {
        "/settings.css" => Some((
            "text/css; charset=utf-8",
            include_str!("../../../assets/settings.css"),
        )),
        "/settings.js" => Some((
            "text/javascript; charset=utf-8",
            include_str!("../../../assets/settings.js"),
        )),
        _ => None,
    }
}
