//! 日志路径展示与“打开目录”接口，与通用配置处理器分离。

use super::handlers::{Response, json_string};

/// 日志路径信息：设置页展示与“打开目录”使用。
pub(super) fn logging_json() -> String {
    let dir = crate::logging::active_log_dir().or_else(|| crate::platform::log::log_dir().ok());
    let dir = dir
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_default();
    let file = crate::logging::active_log_file()
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_default();
    format!(
        "{{\"dir\":{},\"file\":{}}}",
        json_string(&dir),
        json_string(&file)
    )
}

/// `POST /api/logs/open`：在系统文件管理器中打开日志目录。
pub(super) async fn open_logs() -> Response {
    let Some(dir) =
        crate::logging::active_log_dir().or_else(|| crate::platform::log::log_dir().ok())
    else {
        return Response::error(500, "log directory unavailable");
    };
    match tokio::task::spawn_blocking(move || crate::platform::reveal_in_file_manager(&dir)).await {
        Ok(Ok(())) => Response::json(200, "{\"ok\":true}"),
        Ok(Err(error)) => Response::error(500, &error.to_string()),
        Err(error) => Response::error(500, &error.to_string()),
    }
}
