//! lx 输入框的历史浏览状态机：游标、进入浏览前的草稿与文本应用。
//!
//! 只做纯状态转换：历史条目来自全局 [`crate::app::state::PromptHistory`]，
//! 本模块不持有条目、不查库、不做 IO。接管条件对齐 opencode `movePromptHistory`：
//! `↑` 只在光标位于全文最开头、`↓` 只在光标位于全文末尾时接管。

/// 历史浏览状态：游标与进入浏览前的本地草稿。
#[derive(Debug, Default)]
pub(crate) struct HistoryBrowse {
    /// 当前浏览位置（`None` 表示未在浏览）。
    index: Option<usize>,
    /// 进入浏览时暂存的草稿；越过最新一条时原样恢复。
    draft: String,
}

impl HistoryBrowse {
    /// 是否处于浏览态。
    pub(crate) fn browsing(&self) -> bool {
        self.index.is_some()
    }

    /// 当前浏览位置。
    pub(crate) fn index(&self) -> Option<usize> {
        self.index
    }

    /// 退出浏览态（不改输入框文本）。
    pub(crate) fn end(&mut self) {
        self.index = None;
        self.draft.clear();
    }

    /// 进入浏览态并暂存当前草稿。
    pub(crate) fn begin(&mut self, draft: String) {
        self.index = None;
        self.draft = draft;
    }

    /// 移动游标到指定条目。
    pub(crate) fn set_index(&mut self, index: usize) {
        self.index = Some(index);
    }

    /// 越过最新一条：清除游标并交出暂存草稿。
    pub(crate) fn take_draft(&mut self) -> String {
        self.index = None;
        std::mem::take(&mut self.draft)
    }
}

/// 一次历史切换的结果：由 [`decide`] 判定，交给调用方写回输入框。
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum HistoryMove {
    /// 显示第 `index` 条历史；光标按方向落到文首（↑）或文末（↓）。
    Entry {
        index: usize,
        text: String,
        cursor_at_start: bool,
    },
    /// 越过最新一条：恢复进入浏览前暂存的草稿（光标置末尾）。
    Draft,
}

/// 按方向判定这次按键是否由历史接管，并给出要显示的文本。
///
/// `direction` 为 `-1`（↑）或 `1`（↓）。不接管时返回 `None`，调用方回落光标移动。
///
/// 接管条件（对齐 opencode）：
/// - 历史为空 → 不接管；
/// - `↑` 且光标不在全文最开头 → 不接管；`↓` 且光标不在全文末尾 → 不接管；
/// - 未在浏览态时 `↓` → 不接管（无可恢复的"更新的一条"）；
/// - 已在最旧一条继续 `↑` → 不接管（钳住不循环）。
pub(super) fn decide(
    browse: &HistoryBrowse,
    entries: &[String],
    direction: i8,
    input: &str,
    cursor: usize,
) -> Option<HistoryMove> {
    if entries.is_empty() {
        return None;
    }
    let up = direction < 0;
    if up && cursor != 0 {
        return None;
    }
    if !up && cursor != input.len() {
        return None;
    }
    match browse.index() {
        // 未在浏览：只有向上才进入历史，并暂存当前草稿。
        None if !up => None,
        None => {
            let index = entries.len() - 1;
            Some(HistoryMove::Entry {
                index,
                text: entries[index].clone(),
                // `↑` 恢复后光标在文首，才能继续 `↑` 向前翻。
                cursor_at_start: true,
            })
        }
        Some(current) => {
            let next = current as isize + direction as isize;
            if next < 0 {
                return None;
            }
            if next as usize >= entries.len() {
                // 越过最新一条：恢复进入浏览前的草稿。
                return Some(HistoryMove::Draft);
            }
            let index = next as usize;
            Some(HistoryMove::Entry {
                index,
                text: entries[index].clone(),
                // `↑` 落文首、`↓` 落文末：保证连续同向按键都能继续切换。
                cursor_at_start: up,
            })
        }
    }
}
