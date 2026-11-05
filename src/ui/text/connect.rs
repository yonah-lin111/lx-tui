//! `/connect` 快捷连接的用户可见文案。

/// `/connect` 命令描述。
pub const LX_CONNECT_COMMAND_DESC: &str = "Connect a provider with an API key or browser sign-in";
/// 居中模态供应商列表标题。
pub const LX_CONNECT_TITLE_PROVIDERS: &str = "Connect a provider";
/// 各阶段底边快捷键提示。
pub const LX_CONNECT_PROVIDERS_FOOTER: &str = "↑↓ select · ↵ open · ^r refresh models · esc close";
pub const LX_CONNECT_SELECT_FOOTER: &str = "↑↓ select · ↵ confirm · esc back";
pub const LX_CONNECT_MANAGE_FOOTER: &str = "↑↓ select · ↵ confirm · ^r refresh · esc back";
pub const LX_CONNECT_INPUT_FOOTER: &str = "type to edit · ↵ submit · esc back";
pub const LX_CONNECT_OAUTH_FOOTER: &str = "↵ open · o open · c copy · esc cancel";
pub const LX_CONNECT_OAUTH_CODE_FOOTER: &str = "paste code · ↵ submit · esc cancel";
/// 已连接供应商的管理动作。
pub const LX_CONNECT_ACTION_SELECT_MODEL: &str = "Select model";
pub const LX_CONNECT_ACTION_RECONNECT: &str = "Reconnect / Replace credentials";
pub const LX_CONNECT_ACTION_REFRESH_MODELS: &str = "Refresh models";
pub const LX_CONNECT_ACTION_DISCONNECT: &str = "Disconnect";
/// 认证方式右侧标签。
pub const LX_CONNECT_METHOD_OAUTH_TAG: &str = "OAuth";
pub const LX_CONNECT_METHOD_API_KEY_TAG: &str = "API key";
/// 表单/API Key 输入提示。
pub const LX_CONNECT_SEARCH_PLACEHOLDER: &str = "Search providers…";
pub const LX_CONNECT_API_KEY_PLACEHOLDER: &str = "Paste API key…";
pub const LX_CONNECT_OAUTH_CODE_PLACEHOLDER: &str = "Paste authorization code…";
pub const LX_CONNECT_VERIFYING: &str = "Saving API key…";
pub const LX_CONNECT_REFRESHING: &str = "Refreshing models…";
pub const LX_CONNECT_FIELD_REQUIRED: &str = "This field is required";
pub const LX_CONNECT_EMPTY: &str = "no providers";
pub const LX_CONNECT_NO_METHODS: &str = "No sign-in methods available for this provider";
pub const LX_CONNECT_API_KEY_EMPTY: &str = "API key cannot be empty";
/// OAuth 等待阶段的行文案。
pub const LX_CONNECT_OPEN_BROWSER: &str = "Open in browser";
pub const LX_CONNECT_COPY_CODE: &str = "Copy code / URL";
pub const LX_CONNECT_OAUTH_STARTING: &str = "Starting browser authorization…";
pub const LX_CONNECT_OAUTH_WAITING: &str = "Waiting for authorization…";
pub const LX_CONNECT_OAUTH_OPENED: &str = "Opened browser";
pub const LX_CONNECT_CONNECTED_PREFIX: &str = "Connected";
pub const LX_CONNECT_DISCONNECTED_PREFIX: &str = "Disconnected";

/// 供应商列表顶栏右侧摘要：`<n> connected`。
pub fn lx_connect_connected_count(count: usize) -> String {
    format!("{count} connected")
}

/// API Key 输入阶段提示：`Enter API key for <Provider>`。
pub fn lx_connect_api_key_instruction(provider: &str) -> String {
    format!("Enter API key for {provider}")
}

/// 模型刷新完成提示：`Refreshed <n> models`。
pub fn lx_connect_refreshed_models(count: usize) -> String {
    format!("Refreshed {count} models")
}

/// 模型刷新失败提示。
pub fn lx_connect_refresh_failed(error: &str) -> String {
    format!("Model refresh failed: {error}")
}

/// 管理菜单标题：`Manage · <Provider>`。
pub fn lx_connect_title_manage(provider: &str) -> String {
    format!("Manage · {provider}")
}

/// 连接阶段标题：`Connect · <Provider>`。
pub fn lx_connect_title_connect(provider: &str) -> String {
    format!("Connect · {provider}")
}

/// API Key 校验阶段标题：`Verify API key · <Provider>`。
pub fn lx_connect_title_verify(provider: &str) -> String {
    format!("Verify API key · {provider}")
}

/// OAuth 授权阶段标题：`Authorize · <Provider>`。
pub fn lx_connect_title_authorize(provider: &str) -> String {
    format!("Authorize · {provider}")
}

/// 连接成功提示：`Connected <Provider>`。
pub fn lx_connect_connected(provider: &str) -> String {
    format!("{LX_CONNECT_CONNECTED_PREFIX} {provider}")
}

/// 断开连接提示：`Disconnected <Provider>`。
pub fn lx_connect_disconnected(provider: &str) -> String {
    format!("{LX_CONNECT_DISCONNECTED_PREFIX} {provider}")
}

/// 保存连接失败提示。
pub fn lx_connect_failed(error: &str) -> String {
    format!("Connection failed: {error}")
}

/// OAuth 授权失败提示。
pub fn lx_connect_oauth_failed(error: &str) -> String {
    format!("Authorization failed: {error}")
}
