//! 应用层效应出箱：纯数据描述需要受控服务执行的外部操作（DB 读写与剪贴板）。
//!
//! `app` 层只登记效应、不执行 IO；事件循环内的持久化服务按序执行并向状态层回报。

use std::path::PathBuf;

use super::trust::TrustTrigger;

/// 临时提示词升格为正式条目时携带的一页（名称与正文）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptPageDraft {
    pub name: String,
    pub content: String,
}

/// 一次待执行的副作用。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    /// 信任检查：路径或所属仓库根已信任则执行续作，否则弹出信任浮层。
    ///
    /// `root` 为已解析的仓库主 checkout（worktree 对话框）；None 表示仓库归属未知，
    /// 服务未命中快速判定时由事件层后台解析后回投。
    CheckTrust {
        path: PathBuf,
        root: Option<PathBuf>,
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
    /// 新建提示词（建立单一空 `Page 1`）；`folder_id` 为 None 时挂根层。
    CreatePrompt {
        workspace: PathBuf,
        folder_id: Option<i64>,
        name: String,
    },
    /// 保存临时草稿为正式条目（连页写入指定文件夹）；成功后移除临时对象。
    SaveDraft {
        workspace: PathBuf,
        folder_id: Option<i64>,
        name: String,
        /// 临时提示词的全部页（名称、正文）。
        pages: Vec<PromptPageDraft>,
        /// 保存后应显示的页索引（临时对象当前页）。
        active: usize,
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
    /// 保存提示词页正文；回执携带页 id 与修订号，成功后只确认对应页。
    SavePromptContent {
        workspace: PathBuf,
        id: i64,
        /// 数据库页 id。
        page_id: i64,
        content: String,
        /// 发起保存时的 prompt 编辑器标识与内容修订号。
        prompt: crate::layout::PaneId,
        revision: u64,
    },
    /// 追加一页到末尾并设为当前页（新页立即进入命名编辑）。
    CreatePage {
        workspace: PathBuf,
        id: i64,
        name: String,
    },
    /// 重命名一页。
    RenamePage {
        workspace: PathBuf,
        page_id: i64,
        name: String,
    },
    /// 删除一页（最后一页不可删除由应用层阻止）。
    DeletePage {
        workspace: PathBuf,
        id: i64,
        page_id: i64,
    },
    /// 记录提示词的当前页（跨面板同步“上次打开的页”）。
    SetActivePage {
        workspace: PathBuf,
        id: i64,
        page_id: i64,
    },
    /// 添加引用文件夹记录（路径已由后台校验与规范化）。
    ///
    /// `anchor` 为来源菜单锚点；写成功后据此重开菜单并把新增项滚入可见范围。
    AddReferencedFolder {
        workspace: PathBuf,
        path: String,
        anchor: (u16, u16),
    },
    /// 删除引用文件夹记录（仅移除记录，不动磁盘内容）。
    DeleteReferencedFolder { workspace: PathBuf, id: i64 },
    /// 复制文本到系统剪贴板并弹出反馈 toast。
    CopyToClipboard(String),
}
