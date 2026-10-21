//! 聊天步骤视图：一条 assistant 消息的**唯一有序时间线**（正文 / 思考 / 工具调用）。
//!
//! 每轮只有一条 assistant 消息；`steps` 的 `position` 由 app 层在内存中按
//! 「已有最大 position + 1」分配，DB 的 `agent_message_part` 与 `agent_tool_call`
//! 两表共用同一消息内编号。UI、鼠标命中、滚动几何与上下文回放都以本时间线为
//! 唯一顺序来源，不得再按时间戳二次排序。

use crate::chat::events::{MAX_SUMMARY_CHARS, PartKind};

/// 一个展示用内容块（消息内的有序 part）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatPartView {
    /// `text` / `reasoning`。
    pub kind: PartKind,
    pub content: String,
    /// 供应商元数据 JSON（如 Anthropic signature）。
    pub provider_metadata_json: Option<String>,
    /// 该 part 首个增量到达时刻（毫秒）；本轮流式写入，历史加载时从 DB 恢复。
    pub started_at_ms: Option<i64>,
    /// 该 part 最后一个增量到达时刻（毫秒）；None 表示未知（思考耗时不显示）。
    /// 注意它不是「已完成」：思考完成看 [`ChatPartView::completed_at_ms`]。
    pub ended_at_ms: Option<i64>,
    /// 该 part 独立完成时刻（毫秒）；仅 reasoning 使用，None 表示尚未完成或非思考块。
    pub completed_at_ms: Option<i64>,
}

impl ChatPartView {
    /// 存储用稳定 kind 字符串。
    pub fn kind_str(&self) -> &'static str {
        self.kind.as_str()
    }

    /// 是否是一个**仍开放**的思考块（独立完成字段为准，不看消息状态）。
    pub fn reasoning_open(&self) -> bool {
        self.kind == PartKind::Reasoning && self.completed_at_ms.is_none()
    }
}

/// 工具调用状态（UI 语义）。
///
/// 状态机 `input_streaming -> pending -> running -> completed | failed | cancelled`，
/// 另有异常恢复写入的 `interrupted`。`InputStreaming` 只存在于内存（参数未完整接收，
/// 尚无副作用）；`Pending` 必须已入库（DB 由 agent loop 阶段保证）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolStatus {
    /// 正在接收参数（不可执行）。
    InputStreaming,
    /// 参数完整且已（或即将）入库，等待执行。
    Pending,
    /// 执行中。
    Running,
    /// 成功完成。
    Completed,
    /// 执行失败（工具级错误）。
    Failed,
    /// 用户取消（尚未开始执行）。
    Cancelled,
    /// 异常中断（崩溃/连接断开后恢复；结果未知）。
    Interrupted,
}

impl ToolStatus {
    /// DB 状态字符串 → 视图状态；未知值返回 None（调用方按 `interrupted` 保守处理）。
    pub fn from_stored(value: &str) -> Option<Self> {
        match value {
            "pending" => Some(Self::Pending),
            "running" => Some(Self::Running),
            "completed" => Some(Self::Completed),
            "failed" => Some(Self::Failed),
            "cancelled" => Some(Self::Cancelled),
            "interrupted" => Some(Self::Interrupted),
            _ => None,
        }
    }

    /// 是否终态（不再变化）。
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Completed | Self::Failed | Self::Cancelled | Self::Interrupted
        )
    }

    /// 是否仍未进入终态。
    pub fn is_open(self) -> bool {
        !self.is_terminal()
    }

    /// 存储用稳定值（与 DB `agent_tool_call.status` 同一口径）。
    ///
    /// `input_streaming` 只存在于内存（参数未完整、尚未入库），DB 不会出现该值；
    /// 上下文回放只认 `completed` / `failed`，其余状态即使出现也不会伪造结果。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::InputStreaming => "input_streaming",
            Self::Pending => "pending",
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Interrupted => "interrupted",
        }
    }
}

/// 工具步骤的 UI 摘要：只装路径/行范围/计数，**不装**完整文件内容。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolSummary {
    /// 目标路径（相对/绝对按模型给出的形式，超长按 [`MAX_SUMMARY_CHARS`] 截断）。
    pub path: Option<String>,
    /// 1-based 起止行（`read` 类工具的行范围）。
    ///
    /// **唯一权威来源是结果体的 `start_line`/`end_line`**（见 [`ToolSummary::absorb_result_json`]）；
    /// 参数侧的 `offset`/`limit` 只有在两者都给全时才是兜底值（见 [`ToolSummary::from_arguments`]）。
    pub range: Option<(usize, usize)>,
    /// 新增行数。
    pub added: Option<usize>,
    /// 删除行数。
    pub removed: Option<usize>,
    /// 命中条目数（`search` 命中数 / 目录读取的条目数）。
    pub matches: Option<usize>,
    /// `search` 的搜索模式（**只来自调用参数**，结果体里没有）。
    ///
    /// 它是搜索行唯一有意义的「对什么搜」，没有它界面只剩一个「3 matches」的裸计数；
    /// 因此必须进摘要、并在结算/重载两条路径上都不被覆盖丢失。
    pub pattern: Option<String>,
    /// `read` **实际读到的行数**（`end_line - start_line + 1`，只来自结果体）。
    ///
    /// 与 [`Self::total_lines`]（文件总行数 / 目录条目数）是两件不同的事：UI 的
    /// `(N lines)` 只用本字段，绝不用总行数兜底（否则 `12-40 (1000 lines)` 会被读成
    /// 「读了 1000 行」）。
    pub read_lines: Option<usize>,
    /// 文件总行数（目录读取时为条目数）。
    pub total_lines: Option<usize>,
    /// 内容是否被截断（必须显式告知用户）。
    pub truncated: bool,
    /// `edit` 是否有 hunk 应用后内容与原文相同（无实际变更）。
    ///
    /// 语义见设计 §20.2：本次补丁里至少有一个 hunk「应用了但内容没变」。结果体 JSON
    /// 带 `"partial"`，重载路径从这里补；UI 在 `edit`/`write` 的详情行尾部显示
    /// `· partial`。其它工具恒为 false。
    pub partial: bool,
    /// 增删计数是否已知（旧内容不可解码时为 false，added/removed 不得当事实展示）。
    pub counts_known: bool,
    /// 计数是否来自超限输入的线性简化脚本。
    pub simplified: bool,
    /// 因元数据超限而未保存摘要的文件数（> 0 时 UI 显示省略提示）。
    pub omitted_files: usize,
}

impl Default for ToolSummary {
    /// `counts_known` 默认 true：旧结果与非文件工具都按已知处理。
    fn default() -> Self {
        Self {
            path: None,
            range: None,
            added: None,
            removed: None,
            matches: None,
            pattern: None,
            read_lines: None,
            total_lines: None,
            truncated: false,
            partial: false,
            counts_known: true,
            simplified: false,
            omitted_files: 0,
        }
    }
}

impl ToolSummary {
    /// 从参数 JSON 提取最小摘要（路径 + `search` 模式 + `read` 行范围兜底）。
    ///
    /// 只读参数，不执行任何 IO；执行结果（增删计数/命中数/实际行数/截断）由工具执行层在
    /// 结算时补齐。参数不是 JSON 对象或没有可用字段时返回空摘要。
    ///
    /// **区间以结果体为准，参数侧只是兜底**：只有 `offset` 与 `limit` **同时**给出
    /// （且 `limit > 0`）时才推导 `Some((offset, offset + limit - 1))`；只有 `offset`
    /// 或只有 `limit` 时一律 `None`——单边参数推不出闭区间（旧的「只有 offset 塌成
    /// `(start, start)`」会把 12-1000 显示成 12-12）。
    pub fn from_arguments(arguments_json: &str) -> Self {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(arguments_json) else {
            return Self::default();
        };
        let path = value
            .get("path")
            .and_then(|path| path.as_str())
            .filter(|path| !path.is_empty())
            .map(truncate_summary);
        // `edit` 的参数是 patch 文本，没有 `path` 字段；从 `*** Update File:` 行
        // 取目标路径，否则工具行只剩方块与空 `└─`（重进会话尤其明显）。
        let path = path.or_else(|| patch_target(value.get("patch").and_then(|p| p.as_str())));
        // `search` 的 pattern 只存在于参数里（结果体不带），必须在这里取。
        let pattern = value
            .get("pattern")
            .and_then(|pattern| pattern.as_str())
            .filter(|pattern| !pattern.is_empty())
            .map(truncate_summary);
        let offset = value
            .get("offset")
            .and_then(|offset| offset.as_u64())
            .map(|offset| usize::try_from(offset).unwrap_or(usize::MAX));
        let limit = value
            .get("limit")
            .and_then(|limit| limit.as_u64())
            .map(|limit| usize::try_from(limit).unwrap_or(usize::MAX));
        // 只有 offset + limit 都给全才推得出闭区间（见方法文档）。
        let range = match (offset, limit) {
            (Some(offset), Some(limit)) if limit > 0 => {
                let start = offset.max(1);
                Some((start, start.saturating_add(limit).saturating_sub(1)))
            }
            _ => None,
        };
        Self {
            path,
            range,
            pattern,
            ..Self::default()
        }
    }

    /// 用**落库的结果体**补全摘要（历史重载路径）。
    ///
    /// 增删计数 / 命中数 / 实际行数与区间 / 截断标记只存在于结果体里；只靠
    /// [`Self::from_arguments`] 重建会让重进会话的工具行丢掉这些细节
    /// （旧 bug：写文件类操作重进 session 后只剩方块与空 `└─`）。
    ///
    /// 纯解析、无 IO。字段优先级：
    /// - **`range` / `read_lines` / `matches` / `added` / `removed` / `total_lines`**：
    ///   结果体已给出的值覆盖参数侧（区间与行数的唯一权威来源是结果体）；
    /// - **`path` / `pattern`**：参数侧已有则保留，结果体只填空缺。
    ///
    /// 结果体不是合法 JSON 时原样返回。
    pub fn absorb_result_json(mut self, result_json: Option<&str>) -> Self {
        let Some(result_json) = result_json else {
            return self;
        };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(result_json) else {
            return self;
        };
        let number = |key: &str| {
            value
                .get(key)
                .and_then(serde_json::Value::as_u64)
                .map(|number| usize::try_from(number).unwrap_or(usize::MAX))
        };
        if self.path.is_none() {
            self.path = value
                .get("path")
                .and_then(|path| path.as_str())
                .filter(|path| !path.is_empty())
                .map(truncate_summary);
        }
        // `read` 的区间与「实际读到的行数」**只以结果体为准**（参数侧只是兜底，
        // 见 `from_arguments`）：结果体给了 `kind` 就按它的区间口径改写——
        // 空文件（`start_line == 0`）与目录读取（`start_line` 是条目序号）都没有行区间。
        // 这里必须清掉参数侧推导值，否则重载路径清空、实时路径留着，两条路不一致。
        let kind = value.get("kind").and_then(serde_json::Value::as_str);
        match kind {
            Some("file") => {
                let start = number("start_line").unwrap_or(0);
                let end = number("end_line").unwrap_or(0);
                if start > 0 {
                    self.range = Some((start, end));
                    self.read_lines = Some(end.saturating_sub(start).saturating_add(1));
                } else {
                    self.range = None;
                    self.read_lines = None;
                }
            }
            Some("directory") => {
                self.range = None;
                self.read_lines = None;
                // 条目数在结果体里叫 `total_lines`（没有 `matches` 数组），
                // 映射到 `matches`，与 `to_summary` 的口径一致。
                if self.matches.is_none()
                    && let Some(entries) = number("total_lines")
                {
                    self.matches = Some(entries);
                }
            }
            _ => {}
        }
        // `search` 结果体带 matches 数组；命中数就是它的长度。
        if self.matches.is_none()
            && let Some(matches) = value.get("matches").and_then(|m| m.as_array())
        {
            self.matches = Some(matches.len());
        }
        // `edit` 的增删计数在 `changes` 数组里逐文件给，需要汇总；
        // `write` 则直接在顶层。两种形状都要认。
        if let Some(changes) = value.get("changes").and_then(|c| c.as_array()) {
            let sum = |key: &str| {
                changes
                    .iter()
                    .filter_map(|change| change.get(key).and_then(serde_json::Value::as_u64))
                    .fold(None::<usize>, |total, value| {
                        Some(total.unwrap_or(0) + usize::try_from(value).unwrap_or(usize::MAX))
                    })
            };
            if self.added.is_none() {
                self.added = sum("added");
            }
            if self.removed.is_none() {
                self.removed = sum("removed");
            }
            // 单文件补丁：路径也从 changes 里取（`edit` 的参数是 patch 文本，不含 path）。
            if self.path.is_none()
                && let Some(path) = changes
                    .first()
                    .and_then(|change| change.get("path"))
                    .and_then(|path| path.as_str())
                    .filter(|path| !path.is_empty())
            {
                self.path = Some(truncate_summary(path));
            }
        }
        if self.added.is_none() {
            self.added = number("added");
        }
        if self.removed.is_none() {
            self.removed = number("removed");
        }
        if self.total_lines.is_none() {
            self.total_lines = number("total_lines");
        }
        if value
            .get("truncated")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
        {
            self.truncated = true;
        }
        // `edit` 的部分成功标记（`to_result_json` 的顶层 `"partial"`）。
        if value
            .get("partial")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
        {
            self.partial = true;
        }
        // 调用总计是摘要的唯一权威来源：裁切后的 `changes` 前缀不能重新求和冒充总计。
        // 旧结果没有 `file_totals`，上面的逐项汇总保持原口径。
        if let Some(totals) = value.get("file_totals") {
            let known = !totals.get("added").is_some_and(serde_json::Value::is_null)
                && !totals
                    .get("removed")
                    .is_some_and(serde_json::Value::is_null);
            self.counts_known = known;
            self.added = if known {
                totals
                    .get("added")
                    .and_then(serde_json::Value::as_u64)
                    .map(|number| usize::try_from(number).unwrap_or(usize::MAX))
            } else {
                None
            };
            self.removed = if known {
                totals
                    .get("removed")
                    .and_then(serde_json::Value::as_u64)
                    .map(|number| usize::try_from(number).unwrap_or(usize::MAX))
            } else {
                None
            };
            self.simplified = totals
                .get("simplified")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false);
            self.omitted_files = totals
                .get("omitted_files")
                .and_then(serde_json::Value::as_u64)
                .map_or(0, |number| usize::try_from(number).unwrap_or(usize::MAX));
        }
        // 不识别的文件结果没有计数标志：按已知处理（旧 JSON 兼容）。
        if let Some(known) = value
            .get("counts_known")
            .and_then(serde_json::Value::as_bool)
        {
            self.counts_known = known;
            if !known {
                // 未知必须清掉数字，不能因 None 而保留参数/前缀里的旧值。
                self.added = None;
                self.removed = None;
            }
        }
        if let Some(simplified) = value.get("simplified").and_then(serde_json::Value::as_bool) {
            self.simplified |= simplified;
        }
        self
    }

    /// 用**结果侧摘要**合并参数派生摘要（实时结算路径）。
    ///
    /// `self` 是 [`Self::from_arguments`] 的参数派生值，`result` 是
    /// `chat::tools::to_summary` 从执行结果算出的摘要。合并规则与重载路径
    /// （[`Self::absorb_result_json`]）**同一口径**，两条路径的产出必须逐字段相等：
    ///
    /// - 结果侧已给出的字段**优先覆盖**：`range` / `read_lines` / `matches` /
    ///   `added` / `removed` / `total_lines` / `truncated`；
    /// - `pattern` 与 `path` **参数侧优先**（`search` 的 pattern 只存在于参数里，
    ///   `edit` 的参数是 patch 文本、路径也可能比结果侧的更早可用），结果侧只填空缺。
    pub fn absorb_summary(mut self, result: &Self) -> Self {
        // 参数侧优先、结果侧填空缺的字段。
        if self.path.is_none() {
            self.path.clone_from(&result.path);
        }
        if self.pattern.is_none() {
            self.pattern.clone_from(&result.pattern);
        }
        // 结果侧优先、未给出才保留参数值的字段。
        //
        // `range` / `read_lines` 需要一条额外口径：`read` 的结果侧摘要**一定带
        // `total_lines`**（文件总行数 / 目录条目数），它出现就说明结果体已经对区间下了
        // 结论——此时结果侧的 `None`（空文件、目录读取）也是权威答案，必须覆盖参数
        // 兜底值，否则实时路径会留着参数推测的区间、重载路径已清空，两条路又不一致。
        if let Some(total_lines) = result.total_lines {
            self.range = result.range;
            self.read_lines = result.read_lines;
            self.total_lines = Some(total_lines);
        }
        if result.matches.is_some() {
            self.matches = result.matches;
        }
        if result.added.is_some() {
            self.added = result.added;
        }
        if result.removed.is_some() {
            self.removed = result.removed;
        }
        self.truncated |= result.truncated;
        // `partial` 是「本次补丁里存在无实际变更的 hunk」，只增不减：参数侧永远推不出它，
        // 结果侧为 false 时也不能把已有的 true 覆盖掉（与 `truncated` 同口径）。
        self.partial |= result.partial;
        // 计数已知性是结果侧事实：未知时必须清掉数字，不能保留参数侧的旧值。
        self.counts_known = result.counts_known;
        if !result.counts_known {
            self.added = None;
            self.removed = None;
        }
        self.simplified |= result.simplified;
        self.omitted_files = result.omitted_files;
        self
    }

    /// 摘要是否**完全没有可展示信息**（所有字段为 None 且两个标志都为 false）。
    ///
    /// UI 用它决定是否把摘要挂到步骤上：`Some(空摘要)` 会让工具行画出孤立的空
    /// `└─`；调用方必须用 [`Self::is_empty`] 判空而不是手写字段 `||` 链——后者
    /// 新增字段时会被静默漏掉（P1-1 的回归温床）。
    pub fn is_empty(&self) -> bool {
        self.path.is_none()
            && self.range.is_none()
            && self.added.is_none()
            && self.removed.is_none()
            && self.matches.is_none()
            && self.pattern.is_none()
            && self.read_lines.is_none()
            && self.total_lines.is_none()
            && !self.truncated
            && !self.partial
            && self.omitted_files == 0
            && !self.simplified
    }
}

/// 从 `edit` 的 patch 文本里取第一个被修改文件的路径。
///
/// 只做**展示用**的轻量提取：找 `*** Update File: ` / `*** Add File: ` /
/// `*** Delete File: ` 之后到行尾的那段文本。不做补丁校验（校验在执行层）。
fn patch_target(patch: Option<&str>) -> Option<String> {
    const MARKERS: [&str; 3] = ["*** Update File: ", "*** Add File: ", "*** Delete File: "];
    let patch = patch?;
    patch.lines().find_map(|line| {
        MARKERS
            .iter()
            .find_map(|marker| line.strip_prefix(marker))
            .map(str::trim)
            .filter(|path| !path.is_empty())
            .map(truncate_summary)
    })
}

/// 摘要单字段截断（按字符数，避免切断 UTF-8）。
fn truncate_summary(value: &str) -> String {
    if value.chars().count() <= MAX_SUMMARY_CHARS {
        return value.to_string();
    }
    value.chars().take(MAX_SUMMARY_CHARS).collect()
}

/// 一次工具调用的展示数据。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolCallView {
    /// 供应商 call ID（无原生 ID 时由传输层合成的稳定内部 ID）；同一会话内唯一。
    pub call_id: String,
    /// 工具名（`read` / `search` / `edit` / `write`）。
    pub name: String,
    /// 该步骤在 assistant 时间线内的位置（与 part 共用编号空间）。
    pub position: i64,
    /// 已接收的参数 JSON（`InputStreaming` 期间可能是半截内容）。
    pub arguments_json: String,
    pub status: ToolStatus,
    /// UI 摘要（路径/行范围/增删计数等），不装完整文件内容。
    ///
    /// 装箱：摘要有十余个可选字段，直接内联会让整个工具步骤（以及步骤枚举的
    /// 最大变体）膨胀到 300+ 字节，而摘要本身通常很小甚至为 None。
    pub summary: Option<Box<ToolSummary>>,
    /// 失败/中断原因（稳定错误码）；仅 `Failed` / `Interrupted` 等有值。
    pub error: Option<String>,
    /// 落库的结果体原文（规范化 JSON）；仅历史加载与结算后填充。
    ///
    /// 上下文回放需要它把「本轮已发生的调用/结果」与历史同源重建；UI 只按需读取摘要，
    /// 绝不把本字段铺进历史正文。
    pub result_json: Option<String>,
    /// 进程/计划工具的**类型化展示数据**；文件工具为 None（沿用 `summary`）。
    ///
    /// 由事件层在结算与重载两条路径上从同一条结果 JSON 生成，渲染层只读、不解析。
    /// 装箱是为了不让单个工具步骤把整个步骤时间线撑大（卡片数据远大于文件摘要）。
    pub detail: Option<Box<super::tool_view::ToolDetailView>>,
}

impl ToolCallView {
    /// 新建一个正在接收参数的工具步骤。
    pub fn new(call_id: impl Into<String>, name: impl Into<String>, position: i64) -> Self {
        Self {
            call_id: call_id.into(),
            name: name.into(),
            position,
            arguments_json: String::new(),
            status: ToolStatus::InputStreaming,
            summary: None,
            error: None,
            result_json: None,
            detail: None,
        }
    }

    /// 参数接收完成：写入完整 JSON 并转 `Pending`；JSON 非法返回 false 并记为失败。
    pub fn accept_arguments(&mut self, arguments_json: String) -> bool {
        self.arguments_json = arguments_json;
        if serde_json::from_str::<serde_json::Value>(&self.arguments_json).is_err() {
            return false;
        }
        self.summary = Some(Box::new(ToolSummary::from_arguments(&self.arguments_json)));
        self.status = ToolStatus::Pending;
        true
    }

    /// 命令卡片视图（仅 `exec_command` 有）。
    pub fn command_view(&self) -> Option<&super::tool_view::CommandView> {
        match self.detail.as_deref()? {
            super::tool_view::ToolDetailView::Command(view) => Some(view),
            _ => None,
        }
    }

    /// 该工具步骤关联的**进程会话 ID**（进程类卡片才有）。
    ///
    /// `[open]` 点击时用它把后台任务面板的高亮定位到这条任务；文件工具与计划
    /// 卡片返回 None（它们没有可打开的后台进程）。
    pub fn process_session_id(&self) -> Option<u64> {
        match self.detail.as_deref()? {
            super::tool_view::ToolDetailView::Command(view) => view.process_session_id,
            super::tool_view::ToolDetailView::Interaction(view) => Some(view.process_session_id),
            super::tool_view::ToolDetailView::Plan(_)
            | super::tool_view::ToolDetailView::File(_) => None,
        }
    }

    /// 命令卡片视图（可变）。
    pub fn command_view_mut(&mut self) -> Option<&mut super::tool_view::CommandView> {
        match self.detail.as_deref_mut()? {
            super::tool_view::ToolDetailView::Command(view) => Some(view),
            _ => None,
        }
    }

    /// 交互卡片视图（仅 `write_stdin` 有）。
    pub fn interaction_view(&self) -> Option<&super::tool_view::ProcessInteractionView> {
        match self.detail.as_deref()? {
            super::tool_view::ToolDetailView::Interaction(view) => Some(view),
            _ => None,
        }
    }

    /// 计划卡片视图（仅 `update_plan` 有）。
    pub fn plan_view(&self) -> Option<&super::tool_view::PlanSnapshotView> {
        match self.detail.as_deref()? {
            super::tool_view::ToolDetailView::Plan(view) => Some(view),
            _ => None,
        }
    }

    /// 文件工具的类型化差异详情。
    pub fn file_view(&self) -> Option<&super::file_diff::FileDiffDetailView> {
        match self.detail.as_deref()? {
            super::tool_view::ToolDetailView::File(view) => Some(view),
            _ => None,
        }
    }

    /// 是否带可展开的类型化详情（进程 / 计划 / 文件卡片）。
    pub fn has_detail(&self) -> bool {
        self.detail.is_some()
    }

    /// 记为失败（参数超限/非法、执行失败等）。
    pub fn fail(&mut self, error: impl Into<String>) {
        self.status = ToolStatus::Failed;
        self.error = Some(error.into());
    }

    /// 结算为终态（不伪造结果：只写状态与原因）。
    pub fn settle(&mut self, status: ToolStatus, error: Option<String>) {
        self.status = status;
        self.error = error;
    }
}

/// assistant 时间线中的一个步骤：内容块或工具调用（按 `position` 唯一有序）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AssistantStepView {
    Part { position: i64, part: ChatPartView },
    Tool { position: i64, call: ToolCallView },
}

impl AssistantStepView {
    /// 时间线位置（跨 part/tool 共用的消息内编号）。
    pub fn position(&self) -> i64 {
        match self {
            Self::Part { position, .. } | Self::Tool { position, .. } => *position,
        }
    }

    /// 内容块视图（工具步骤为 None）。
    pub fn part(&self) -> Option<&ChatPartView> {
        match self {
            Self::Part { part, .. } => Some(part),
            Self::Tool { .. } => None,
        }
    }

    /// 可变内容块视图（工具步骤为 None）。
    pub fn part_mut(&mut self) -> Option<&mut ChatPartView> {
        match self {
            Self::Part { part, .. } => Some(part),
            Self::Tool { .. } => None,
        }
    }

    /// 工具调用视图（内容块步骤为 None）。
    pub fn tool(&self) -> Option<&ToolCallView> {
        match self {
            Self::Tool { call, .. } => Some(call),
            Self::Part { .. } => None,
        }
    }

    /// 可变工具调用视图（内容块步骤为 None）。
    pub fn tool_mut(&mut self) -> Option<&mut ToolCallView> {
        match self {
            Self::Tool { call, .. } => Some(call),
            Self::Part { .. } => None,
        }
    }
}

/// 一条展示用消息。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatMessageView {
    pub id: String,
    /// `user` / `assistant`。
    pub role: String,
    /// `complete` / `streaming` / `interrupted` / `failed`。
    pub status: String,
    pub turn_index: i64,
    /// 有序步骤时间线（用户消息只有单个 text part；assistant 可含思考与工具）。
    pub steps: Vec<AssistantStepView>,
    /// 该轮实际模型展示名（assistant 行）。
    pub model_label: Option<String>,
    /// 失败原因码（如 `http 503`、`timeout`）；仅 `failed` 时有值，用于历史行提示。
    pub error_code: Option<String>,
    /// 该轮供应商报的输入 / 输出 / 缓存读取 token（未知为 None，不写 0 冒充）。
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub cache_read_tokens: Option<i64>,
    /// 该轮耗时（毫秒；未完成或时间戳缺失为 None）。
    pub duration_ms: Option<i64>,
}

impl ChatMessageView {
    /// 全部内容块（按时间线顺序；不含工具步骤）。
    pub fn parts(&self) -> impl Iterator<Item = &ChatPartView> {
        self.steps.iter().filter_map(AssistantStepView::part)
    }

    /// 正文内容块。
    pub fn text_parts(&self) -> impl Iterator<Item = &ChatPartView> {
        self.parts().filter(|part| part.kind == PartKind::Text)
    }

    /// 思考内容块。
    pub fn reasoning_parts(&self) -> impl Iterator<Item = &ChatPartView> {
        self.parts().filter(|part| part.kind == PartKind::Reasoning)
    }

    /// 全部工具步骤。
    pub fn tools(&self) -> impl Iterator<Item = &ToolCallView> {
        self.steps.iter().filter_map(AssistantStepView::tool)
    }

    /// 指定索引的步骤（索引即时间线下标，也是 UI 命中用的 `step_index`）。
    pub fn step_at(&self, index: usize) -> Option<&AssistantStepView> {
        self.steps.get(index)
    }

    /// 拼接全部正文（不含思考与工具）。
    pub fn text(&self) -> String {
        self.text_parts()
            .map(|part| part.content.as_str())
            .collect()
    }

    /// 消息内已分配的最大时间线位置；空消息为 None。
    pub fn max_position(&self) -> Option<i64> {
        self.steps.iter().map(AssistantStepView::position).max()
    }

    /// 下一个可分配的时间线位置（已有最大 + 1）。
    pub fn next_position(&self) -> i64 {
        self.max_position()
            .map_or(0, |position| position.saturating_add(1))
    }

    /// 按 call ID 查找工具步骤。
    pub fn tool(&self, call_id: &str) -> Option<&ToolCallView> {
        self.tools().find(|call| call.call_id == call_id)
    }

    /// 按 call ID 查找可变工具步骤。
    pub fn tool_mut(&mut self, call_id: &str) -> Option<&mut ToolCallView> {
        self.steps
            .iter_mut()
            .filter_map(AssistantStepView::tool_mut)
            .find(|call| call.call_id == call_id)
    }

    /// 追加一个内容块步骤，返回分配的 position。
    pub fn push_part(&mut self, part: ChatPartView) -> i64 {
        let position = self.next_position();
        self.steps.push(AssistantStepView::Part { position, part });
        position
    }

    /// 追加一个工具步骤，返回分配的 position（`call.position` 以分配值为准）。
    pub fn push_tool(&mut self, mut call: ToolCallView) -> i64 {
        let position = self.next_position();
        call.position = position;
        self.steps.push(AssistantStepView::Tool { position, call });
        position
    }

    /// 最后一个步骤若是**未完成**的思考块则返回其可变引用（当前唯一「开放」定义）。
    pub fn open_reasoning_mut(&mut self) -> Option<&mut ChatPartView> {
        match self.steps.last_mut() {
            Some(AssistantStepView::Part { part, .. }) if part.reasoning_open() => Some(part),
            _ => None,
        }
    }

    /// 最后一个步骤若是工具步骤则返回其可变引用。
    pub fn last_tool_mut(&mut self) -> Option<&mut ToolCallView> {
        match self.steps.last_mut() {
            Some(AssistantStepView::Tool { call, .. }) => Some(call),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests;
