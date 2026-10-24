//! 读后版本保护：`read` 返回的版本 token 与写工具提交前的 CAS 核对。
//!
//! 三层，每层都用**同一份** token 计算口径（[`crate::platform::version_token`]）：
//!
//! 1. **读取登记**（本模块）：`read` 把“规范化目标路径 → 当前版本 token”记进会话级
//!    登记表，并把 token 回给模型（`file_version`）；
//! 2. **写入前核对**（本模块 + 写工具）：在**路径锁内**读取当前镜像，与登记版本比较；
//!    不一致即拒绝，磁盘零写入。新建文件则要求目标仍不存在；
//! 3. **持久化核对**（`MutationRecorder::prepare`）：票据里携带的期望 token 与
//!    before 镜像重算的 token 必须在同一事务内一致，否则整批零写入。
//!
//! 没有 token 不等于“放行”：模型没读过、或登记表里没有该路径，一律拒绝并要求重读。

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use crate::platform::{RawImage, version_token_of};

use super::path;
use super::types::{ToolErrorCode, ToolFailure};

/// 一次读取登记：规范化目标路径 → 读到的镜像版本。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadVersion {
    /// 模型给出的原始路径（错误消息里回显这个，模型才认得）。
    pub path: String,
    /// 读到的镜像版本 token。
    pub token: String,
}

/// 一个会话内的读版本登记表（进程内、按 session 隔离）。
///
/// 只登记**最后一次读取**：模型重新读一次就会刷新期望版本，这正是“重新读取后重试”
/// 的语义。表项很小（路径 + token），不缓存文件内容。
#[derive(Debug, Default)]
pub struct ReadRegistry {
    entries: Mutex<HashMap<String, ReadVersion>>,
}

impl ReadRegistry {
    /// 登记一条读取版本；返回 token（供 `read` 结果回传模型）。
    pub fn record(&self, canonical_key: &str, path: &str, image: &RawImage) -> String {
        let token = version_token_of(image);
        if let Ok(mut entries) = self.entries.lock() {
            entries.insert(
                canonical_key.to_string(),
                ReadVersion {
                    path: path.to_string(),
                    token: token.clone(),
                },
            );
        }
        token
    }

    /// 查询某路径登记过的版本 token。
    pub fn token_for(&self, canonical_key: &str) -> Option<String> {
        self.entries
            .lock()
            .ok()
            .and_then(|entries| entries.get(canonical_key).map(|entry| entry.token.clone()))
    }

    /// 登记条数（诊断与测试）。
    pub fn len(&self) -> usize {
        self.entries
            .lock()
            .map(|entries| entries.len())
            .unwrap_or(0)
    }

    /// 是否为空。
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// 全局会话登记表：按 session id 惰性建立。
///
/// 生命周期跟随进程：会话删除后旧条目不会造成误放行（CAS 仍要求内容一致），
/// 过期 token 只会导致“要求重新读取”，不会导致越权写入。
fn registries() -> &'static Mutex<HashMap<String, Arc<ReadRegistry>>> {
    static TABLE: OnceLock<Mutex<HashMap<String, Arc<ReadRegistry>>>> = OnceLock::new();
    TABLE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 取（必要时新建）某会话的登记表。
pub fn registry(session_id: &str) -> Arc<ReadRegistry> {
    let mut table = match registries().lock() {
        Ok(table) => table,
        Err(poisoned) => poisoned.into_inner(),
    };
    Arc::clone(
        table
            .entry(session_id.to_string())
            .or_insert_with(|| Arc::new(ReadRegistry::default())),
    )
}

/// 会话销毁时丢弃登记表（`/new`、删除会话、会话进程回收）。
pub fn drop_registry(session_id: &str) {
    let mut table = match registries().lock() {
        Ok(table) => table,
        Err(poisoned) => poisoned.into_inner(),
    };
    table.remove(session_id);
}

/// 修改已有文件的前置条件：本会话读过该路径，且当前磁盘版本与读到的版本一致。
///
/// 成功返回期望 token（写入票据携带它，持久层再核对一次）。所有失败都**零写入**：
/// - 没有登记版本：没读过，要求先读；
/// - 登记版本与当前镜像不一致：读取后被外部改过，要求重读。
pub fn require_read_version(
    registry: &ReadRegistry,
    canonical_key: &str,
    display_path: &str,
    current: &RawImage,
) -> Result<String, ToolFailure> {
    let Some(recorded) = registry.token_for(canonical_key) else {
        return Err(ToolFailure::new(
            ToolErrorCode::StaleRead,
            format!(
                "{display_path} has no recorded read version; read it first, then retry the write"
            ),
        ));
    };
    let actual = version_token_of(current);
    if actual != recorded {
        return Err(ToolFailure::new(
            ToolErrorCode::StaleRead,
            format!(
                "{display_path} changed on disk since it was read; read it again before writing"
            ),
        ));
    }
    Ok(recorded)
}

/// 新建文件的前置条件：目标必须仍不存在。
///
/// 目录项存在（含符号链接、悬空链接）即拒绝——**不判断指向**，否则“新文件”会静默
/// 覆盖别人刚创建的东西。目标刚被创建时提示先读，而不是直接覆盖。
pub fn require_missing(target: &std::path::Path) -> Result<(), ToolFailure> {
    if crate::platform::entry_exists(target) {
        return Err(ToolFailure::new(
            ToolErrorCode::StaleRead,
            format!(
                "{} already exists; read it first (or choose another path) instead of creating it",
                path::display(target)
            ),
        ));
    }
    Ok(())
}

/// 规范化登记键：与路径锁同一口径（canonicalize，新建时回退父目录 + 文件名）。
pub fn canonical_key(target: &std::path::Path) -> String {
    super::lock::lock_key(target)
}

#[cfg(test)]
mod tests;
