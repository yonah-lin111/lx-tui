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
    ///
    /// `anchor` 决定 toast 位置：`None` 走屏幕顶部居中；`Some(pane)` 落在该窗格
    /// （main content）顶部居中——复制动作发生在哪儿，反馈就出现在哪儿。
    CopyToClipboard {
        text: String,
        anchor: Option<crate::layout::PaneId>,
    },
    // ===== lx 聊天 =====
    /// 开启一轮聊天：DB 写入 session(首轮)+user+streaming assistant。
    ///
    /// 成功回投 `ChatTurnCommitted`；失败回投 `ChatTurnFailed`（草稿保留可重试）。
    BeginChatTurn {
        pane: crate::layout::PaneId,
        generation: u64,
        workspace: PathBuf,
        session_id: Option<String>,
        draft: String,
        model: crate::db::chat::ModelSnapshot,
    },
    /// 启动网络流：按冻结模型快照与装配上下文执行，结果按 key 回投。
    StartChatStream {
        key: crate::app::state::lx::RequestKey,
        assistant_id: String,
    },
    /// 取消在途网络流（流取消后仍走 Finishing 收尾保存 partial）。
    CancelChatStream {
        key: crate::app::state::lx::RequestKey,
    },
    /// 提交一次 `ask` 作答；事件层据此唤醒 agent loop。
    ///
    /// `call_id` 是防串台的唯一凭据：UI 面板可能已被新提问替换，旧面板的迟到提交
    /// 必须与当前挂起项比对后丢弃。
    AnswerAsk {
        key: crate::app::state::lx::RequestKey,
        call_id: String,
        answers: crate::chat::tools::AskAnswers,
    },
    /// Committing 阶段取消：等 DB ack 后直接落 interrupted、不发 HTTP。
    CancelChatCommit {
        pane: crate::layout::PaneId,
        generation: u64,
    },
    /// 终态写入：最后 parts、outcome 与 usage 同事务落库。
    FinishChatTurn {
        key: crate::app::state::lx::RequestKey,
        assistant_id: String,
        parts: Vec<crate::db::chat::ChatPart>,
        outcome: crate::db::chat::TurnOutcome,
        usage: crate::db::chat::Usage,
    },
    /// 流中 checkpoint：完整 parts 快照幂等覆盖 streaming 行（≤500ms/4KiB 合并）。
    CheckpointChatTurn {
        workspace: PathBuf,
        assistant_id: String,
        parts: Vec<crate::db::chat::ChatPart>,
    },
    /// 读取当前工作区会话列表（`/resume` 首页）。
    ListChatSessions { pane: crate::layout::PaneId },
    /// 列出本会话仍在运行的后台进程（后台任务页的进程 tab）。
    ///
    /// 进程句柄不进 `AppState`（`app/` 不持有进程），因此与 `/resume` 同一模式：
    /// 这里只发请求，由事件层从运行实例取快照后回投。
    ListSessionProcesses { pane: crate::layout::PaneId },
    /// 停止指定会话进程（后台任务页按 `^d`）。
    StopSessionProcess {
        pane: crate::layout::PaneId,
        /// 模型见过的进程会话 ID。
        process_session_id: u64,
    },
    /// 加载单个会话及有序消息（恢复旧会话）。
    LoadChatSession {
        pane: crate::layout::PaneId,
        session_id: String,
    },
    /// 重命名会话。
    RenameChatSession {
        workspace: PathBuf,
        session_id: String,
        title: String,
    },
    /// 删除会话（先取消在途、等最终写入，再事务级级联删除）。
    DeleteChatSession {
        workspace: PathBuf,
        session_id: String,
    },
    /// 更新已保存会话的当前模型选择三列。
    SetChatSessionModel {
        workspace: PathBuf,
        session_id: String,
        model: crate::db::chat::ModelSnapshot,
    },
    /// 把当前选择的模型写回 `config/ai.json` 的 `defaultModel`（下次启动沿用）。
    SaveDefaultModel {
        selection: crate::config::ai::ModelSelection,
    },
    /// 持久化一条已发送 prompt 到全局历史表（去重置顶、裁剪到上限）。
    ///
    /// 内存历史已在 `on_turn_committed` 同步更新；本效应只负责落库，
    /// 失败仅记录日志，不影响这一轮聊天。
    RecordPromptHistory { text: String },
    /// 读取最新轮次的文件快照可用性（只读，事件层同步执行）。
    PrepareRewind {
        pane: crate::layout::PaneId,
        session_id: String,
        user_message_id: String,
    },
    /// 执行一次最新轮次撤销（唯一 operation id；事件层后台执行）。
    ExecuteRewind(RewindRequest),
    /// 执行一次上下文压缩：独立摘要请求 → 事务写检查点 → 重载视图。
    ///
    /// 事件层负责整条链路（网络请求不经过持久化泵），成功/失败都回投
    /// `AppEvent::CompactFinished`。
    ExecuteCompact(CompactRequest),
    /// 撤销一次 compact 检查点（只切状态与指针，不触碰文件）。
    RevertCompact(CompactRevertRequest),
    /// 提交一条本地命令：事件层先落库预登记，成功后才 spawn。
    ///
    /// 不占用模型 `RequestState`；`workspace` 是会话 DB 键，`cwd` 是冻结的执行目录。
    BeginUserShell {
        pane: crate::layout::PaneId,
        generation: u64,
        workspace: PathBuf,
        cwd: PathBuf,
        session_id: Option<String>,
        /// 去模式前缀后的原始脚本（未 trim，保留换行/引号）。
        command: String,
        /// 实际使用的登录 shell。
        shell: String,
        /// 提交时的输入修订号：只有仍匹配才允许清空草稿。
        draft_revision: u64,
    },
    /// 停止该窗格当前在途/运行中的本地命令（双 Esc）。
    CancelUserShells { pane: crate::layout::PaneId },
    // ===== todo（全局列表）=====
    /// 写入一条（UPSERT：不存在则按 `position` 插入并顺移，存在则只改文本/状态）。
    SaveTodo { record: crate::db::todo::TodoRecord },
    /// 删除一条 todo。
    DeleteTodo { id: String },
    /// 删除某状态的全部 todo（`[clear done]`）。
    DeleteTodosWithStatus { status: crate::db::todo::TodoStatus },
    /// 恢复一批被删 todo（撤销）。
    RestoreTodos {
        records: Vec<crate::db::todo::TodoRecord>,
    },
}

/// 撤销执行请求：目标身份、动作与回填所需的原文全部在发起时冻结。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RewindRequest {
    pub pane: crate::layout::PaneId,
    pub session_id: String,
    pub target_user_message_id: String,
    pub expected_revision: u64,
    pub action: crate::app::state::RewindAction,
    /// 会话所属工作区键（DB 归属校验；不是执行 cwd）。
    pub workspace: PathBuf,
    pub input_at_open: String,
    pub user_text: String,
    pub operation_id: String,
}

/// 一次压缩请求：会话、冻结的 revision 与当前模型全部在发起时确定。
///
/// 摘要请求是**独立请求**（不带工具、不落普通消息）；事件层用它装配请求并执行。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompactRequest {
    pub pane: crate::layout::PaneId,
    /// 会话所属工作区键（DB 归属校验）。
    pub workspace: PathBuf,
    pub session_id: String,
    /// 发起时冻结的会话 revision（成功后提交时核对）。
    pub expected_revision: u64,
    /// 上一个生效检查点（增量范围起点）。
    pub base_checkpoint_id: Option<String>,
    /// 本次使用的模型快照（摘要请求与展示标签同源）。
    pub model: crate::db::chat::ModelSnapshot,
    /// 模型显示名（写入检查点的 `model_label`，UI 原样展示）。
    pub model_label: String,
    pub transport: String,
    pub base_url: String,
    pub api_key: String,
    pub variant_params: Option<serde_json::Value>,
    pub idle_timeout_ms: u64,
    /// 幂等键（同一次压缩的重复回执只写一次）。
    pub operation_id: String,
}

/// 一次 compact 撤销请求。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompactRevertRequest {
    pub pane: crate::layout::PaneId,
    pub workspace: PathBuf,
    pub session_id: String,
    pub checkpoint_id: String,
    /// 发起时冻结的会话 revision。
    pub expected_revision: u64,
    pub operation_id: String,
}
