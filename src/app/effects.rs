//! 应用层效应出箱：纯数据描述需要受控服务执行的外部操作（DB 读写与剪贴板）。
//!
//! `app` 层只登记效应、不执行 IO；事件循环内的持久化服务按序执行并向状态层回报。

use std::path::PathBuf;

use super::prompt::PromptItemStatus;
use super::trust::TrustTrigger;

/// 草稿保存目标：现有文件夹（或根层）或先建新文件夹再保存。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DraftTarget {
    /// `None` 为工作区根层。
    Folder(Option<i64>),
    NewFolder {
        parent_id: Option<i64>,
        name: String,
    },
}

/// 一次待执行的副作用。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    /// 信任检查：路径已信任则执行续作，否则弹出信任浮层。
    CheckTrust {
        path: PathBuf,
        trigger: TrustTrigger,
    },
    /// 记录工作区信任（写库）。
    GrantTrust { path: PathBuf, name: String },
    /// 新建文件夹（根层或子层）。
    CreateFolder {
        workspace: PathBuf,
        parent_id: Option<i64>,
        name: String,
    },
    /// 新建提示词；`folder_id` 为 None 时挂根层。
    CreatePrompt {
        workspace: PathBuf,
        folder_id: Option<i64>,
        name: String,
    },
    /// 保存临时草稿为正式条目（可选先建新文件夹）；成功后清空草稿槽位。
    SaveDraft {
        workspace: PathBuf,
        target: DraftTarget,
        name: String,
        content: String,
    },
    /// 重命名文件夹。
    RenameFolder {
        workspace: PathBuf,
        id: i64,
        name: String,
    },
    /// 删除文件夹（级联子文件夹与条目）。
    DeleteFolder { workspace: PathBuf, id: i64 },
    /// 重命名提示词。
    RenamePrompt {
        workspace: PathBuf,
        id: i64,
        name: String,
    },
    /// 删除提示词。
    DeletePrompt { workspace: PathBuf, id: i64 },
    /// 保存提示词正文。
    SavePromptContent {
        workspace: PathBuf,
        id: i64,
        content: String,
    },
    /// 更新提示词状态。
    SetPromptStatus {
        workspace: PathBuf,
        id: i64,
        status: PromptItemStatus,
    },
    /// 复制文本到系统剪贴板并弹出反馈 toast。
    CopyToClipboard(String),
}
