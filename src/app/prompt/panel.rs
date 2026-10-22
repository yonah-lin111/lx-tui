//! prompt 命令面板生命周期：块命令与斜杠命令面板的导航、确认、关闭与重算。

use super::super::markdown::{self, BlockPanel, SlashPanel};
use super::EditKind;
use super::Prompt;

impl Prompt {
    /// 当前块命令面板；未打开时为 None。
    pub fn panel(&self) -> Option<&BlockPanel> {
        self.panel.as_ref()
    }

    /// 面板打开时按偏移循环移动高亮；返回是否消费该按键。
    pub fn panel_move(&mut self, delta: isize) -> bool {
        match self.panel.as_mut() {
            Some(panel) => {
                panel.move_active(delta);
                true
            }
            None => false,
        }
    }

    /// 块命令面板悬停高亮：设置高亮索引、窗口锚点不动；越界或未变化返回 false。
    pub fn panel_set_active(&mut self, index: usize) -> bool {
        let Some(panel) = self.panel.as_mut() else {
            return false;
        };
        if index >= panel.items().len() || panel.active() == index {
            return false;
        }
        panel.set_active(index);
        true
    }

    /// 滚轮在块命令面板上滚动可见窗口：视口从 `base` 起偏移、越界钳制不循环；
    /// 高亮不动；未打开或视口未移动返回 false。
    pub fn panel_scroll(&mut self, delta: isize, base: usize) -> bool {
        let Some(panel) = self.panel.as_mut() else {
            return false;
        };
        panel.scroll_viewport(delta, base)
    }

    /// 块命令面板点选：设置高亮并确认插入；返回是否消费。
    pub fn panel_confirm_at(&mut self, index: usize) -> bool {
        self.panel_set_active(index);
        self.panel_confirm()
    }

    /// 面板打开时确认高亮命令：替换触发区间并压制重弹；返回是否消费该按键。
    pub fn panel_confirm(&mut self) -> bool {
        let Some(panel) = self.panel.as_ref() else {
            return false;
        };
        let trigger = panel.trigger();
        if trigger.from > trigger.to || trigger.to > self.text.len() {
            return false;
        }
        let Some(id) = panel.items().get(panel.active()).copied() else {
            return false;
        };
        let insertion = markdown::block_insertion(id);
        self.panel = None;
        self.break_group();
        self.record(EditKind::Other);
        self.splice_text(trigger.from, trigger.to, &insertion.text);
        self.cursor = trigger.from + insertion.cursor;
        self.panel_suppressed = true;
        self.scroll_cursor_into_view();
        true
    }

    /// 面板打开时关闭且不改文本；返回是否消费该按键。
    pub fn panel_escape(&mut self) -> bool {
        if self.panel.is_none() {
            return false;
        }
        self.panel = None;
        self.panel_suppressed = true;
        true
    }

    /// 当前斜杠命令面板；未打开时为 None。
    pub fn slash_panel(&self) -> Option<&SlashPanel> {
        self.slash.as_ref()
    }

    /// 斜杠命令面板打开时按偏移移动高亮；选择器跳过占位行；返回是否消费该按键。
    pub fn slash_move(&mut self, delta: isize) -> bool {
        let Some(panel) = self.slash.as_mut() else {
            return false;
        };
        panel.move_active_selectable(delta);
        true
    }

    /// 斜杠面板条目总数。
    pub fn slash_entry_count(&self) -> usize {
        self.slash.as_ref().map_or(0, SlashPanel::entry_count)
    }

    /// 选择器条目是否可选中（占位行不可）。
    pub fn slash_entry_selectable(&self, index: usize) -> bool {
        self.slash
            .as_ref()
            .and_then(|panel| panel.items().get(index))
            .is_some_and(|item| item.selectable)
    }

    /// 重建选择器条目：worktree 列表变化时调用；保持当前高亮位置近似稳定。
    pub(crate) fn rebuild_worktree_entries(&mut self) {
        let Some(panel) = self.slash.as_ref() else {
            return;
        };
        let markdown::SlashPanelKind::WorktreeChooser {
            block_end_line,
            line,
        } = panel.kind()
        else {
            return;
        };
        let block_end_line = *block_end_line;
        let line = line.clone();
        let trigger = panel.trigger().clone();
        let entries = self.worktree_entries();
        let candidates = self.worktree_candidates();
        let bound = markdown::block_worktree_branch_at_line(&self.text, block_end_line);
        let previous = self.slash.as_ref().map_or(0, SlashPanel::active);
        // 结果到达时高亮当前绑定项；否则回退旧高亮（占位时钳到 0）。
        let active = candidates
            .and_then(|entries| {
                bound.as_ref().and_then(|(_, branch)| {
                    entries
                        .iter()
                        .position(|candidate| candidate.branch == *branch)
                })
            })
            .unwrap_or(previous)
            .min(entries.len().saturating_sub(1));
        self.slash = Some(SlashPanel::with_kind(
            trigger,
            markdown::SlashPanelKind::WorktreeChooser {
                block_end_line,
                line,
            },
            entries,
            active,
        ));
    }

    /// 从 worktree 状态派生选择器条目；加载中或空列表给一个不可选占位行。
    fn worktree_entries(&self) -> Vec<markdown::SlashItem> {
        if self.mention.worktree_loading() {
            return vec![markdown::SlashItem::placeholder("loading…")];
        }
        let Some(candidates) = self.worktree_candidates() else {
            return vec![markdown::SlashItem::placeholder("loading…")];
        };
        if candidates.is_empty() {
            return vec![markdown::SlashItem::placeholder("no worktrees found")];
        }
        candidates
            .iter()
            .map(markdown::SlashItem::worktree_entry)
            .collect()
    }

    /// 斜杠命令面板悬停高亮：设置高亮索引；未打开、越界或未变化返回 false。
    pub fn slash_set_active(&mut self, index: usize) -> bool {
        let Some(panel) = self.slash.as_mut() else {
            return false;
        };
        if index >= panel.items().len() || panel.active() == index {
            return false;
        }
        panel.set_active(index);
        true
    }

    /// 滚轮在斜杠命令面板上滚动可见窗口：视口从 `base` 起偏移、越界钳制不循环；
    /// 高亮不动；未打开或视口未移动返回 false。
    pub fn slash_scroll(&mut self, delta: isize, base: usize) -> bool {
        let Some(panel) = self.slash.as_mut() else {
            return false;
        };
        panel.scroll_viewport(delta, base)
    }

    /// 斜杠命令面板点选：设置高亮并确认插入；返回是否消费。
    pub fn slash_confirm_at(&mut self, index: usize) -> bool {
        self.slash_set_active(index);
        self.slash_confirm()
    }

    /// 斜杠命令面板确认：内置/自定义命令整行替换，`/gitWorkTree` 打开二级选择器，
    /// 选择器确认把分支写回块结束行上方的 `{wt:}` 行；返回是否消费。
    pub fn slash_confirm(&mut self) -> bool {
        let Some(panel) = self.slash.as_ref() else {
            return false;
        };
        let trigger = panel.trigger().clone();
        if trigger.line_start > trigger.line_end || trigger.line_end > self.text.len() {
            return false;
        }
        let Some(item) = panel.items().get(panel.active()).cloned() else {
            return false;
        };
        match item.insertion {
            markdown::SlashInsertion::WorktreeChooser => {
                if self.open_worktree_chooser(trigger) {
                    self.panel_suppressed = true;
                    return true;
                }
                false
            }
            markdown::SlashInsertion::WorktreeBinding { branch, is_main } => {
                self.confirm_worktree_binding(&branch, is_main)
            }
            markdown::SlashInsertion::Placeholder => false,
            _ => {
                let Some((content, cursor)) = markdown::slash_insertion(&item) else {
                    return false;
                };
                self.slash = None;
                self.break_group();
                self.record(EditKind::Other);
                let supple = content.starts_with("+++");
                self.splice_text(trigger.line_start, trigger.line_end, &content);
                if supple {
                    // 新建补充块默认展开：光标落在标题内，折叠会把它藏起来。
                    self.expanded_supplements.insert(trigger.line_start);
                }
                self.cursor = trigger.line_start + cursor;
                self.panel_suppressed = true;
                self.scroll_cursor_into_view();
                true
            }
        }
    }

    /// 选择器确认：把选中分支写到块底部（结束行上一行）的 `{wt:}` 绑定行并
    /// 消耗 `/gitWorkTree` 命令，作为一步撤销；命令所在行保留（不删行），
    /// 恰位于绑定位置时该行直接转为绑定行。选择主 checkout 表示解除绑定：
    /// 移除 `{wt:}` 绑定行（保留其上方空行：块底部留白）。
    fn confirm_worktree_binding(&mut self, branch: &str, is_main: bool) -> bool {
        let Some(panel) = self.slash.as_ref() else {
            return false;
        };
        let block_end_line = match panel.kind() {
            markdown::SlashPanelKind::WorktreeChooser { block_end_line, .. } => *block_end_line,
            _ => return false,
        };
        let trigger = panel.trigger().clone();
        let valid_trigger =
            trigger.line_start < trigger.line_end && trigger.line_end <= self.text.len();
        let mut edits = Vec::new();
        let mut replaces_trigger = false;
        if is_main {
            // 主 checkout 即默认工作区：不写绑定，已有绑定行移除（保留底部空行）。
            if let Some(clear) = markdown::clear_line_worktree(&self.text, block_end_line) {
                replaces_trigger = !clear.0.is_empty() && clear.0.start == trigger.line_start;
                edits.push(clear);
            }
        } else {
            // 命令行所在行号：恰占绑定位置（结束行上一行）时绑定直接占用该行。
            let trigger_line = self.text[..trigger.line_start.min(self.text.len())]
                .matches('\n')
                .count();
            let occupy = valid_trigger.then_some(trigger_line);
            let Some((marker_range, marker_text)) =
                markdown::set_line_worktree(&self.text, block_end_line, branch, occupy)
            else {
                return false;
            };
            // 绑定编辑从命令行行首起替换即为占用，无需再清空该行。
            replaces_trigger = !marker_range.is_empty() && marker_range.start == trigger.line_start;
            edits.push((marker_range, marker_text));
        }
        if valid_trigger && !replaces_trigger {
            // 清空命令行内容、保留该行与换行（对齐 lx-agent `/gitn` 清空命令行的语义）。
            edits.push((trigger.line_start..trigger.line_end, String::new()));
        }
        self.slash = None;
        self.break_group();
        if edits
            .iter()
            .any(|(range, text)| !range.is_empty() || !text.is_empty())
        {
            self.record(EditKind::Other);
            self.apply_worktree_edits(edits);
        }
        self.panel_suppressed = true;
        self.scroll_cursor_into_view();
        true
    }

    /// 同一步撤销内应用多段替换：从后往前执行，偏移互不影响；光标按编辑位置
    /// 平移（区间前累计位移，区间内落到替换文本末），避免插入/删除后落到下一行。
    fn apply_worktree_edits(&mut self, mut edits: Vec<(std::ops::Range<usize>, String)>) {
        edits.sort_by_key(|(range, _)| range.start);
        let mut cursor = self.cursor;
        for (range, replacement) in edits.into_iter().rev() {
            if range.start > range.end || range.end > self.text.len() {
                continue;
            }
            self.splice_text(range.start, range.end, &replacement);
            if cursor >= range.end {
                cursor = cursor - (range.end - range.start) + replacement.len();
            } else if cursor > range.start {
                cursor = range.start + replacement.len();
            }
        }
        self.cursor = cursor.min(self.text.len());
    }

    /// 打开二级 worktree 选择器：光标处必须有模板块（绑定目标即该块结束行）。
    ///
    /// 先把命令行整行回显为规范命令文本（`/gitWorkTree `）再打开选择器并锚定该行
    /// （对齐 lx-agent：选择型命令先回显命令、再开二级面板）。回显作为独立一步撤销，
    /// 撤销后回到用户输入中的命令行。选择器条目由 UI 直接读取 worktree 状态；
    /// 列表刷新由事件层调度。
    fn open_worktree_chooser(&mut self, trigger: markdown::SlashTrigger) -> bool {
        let Some(block_end_line) = markdown::block_end_line_at(&self.text, self.cursor) else {
            return false;
        };
        self.break_group();
        self.record(EditKind::Other);
        let line = markdown::WORKTREE_COMMAND_LINE.to_string();
        self.splice_text(trigger.line_start, trigger.line_end, &line);
        self.cursor = trigger.line_start + line.len();
        self.scroll_cursor_into_view();
        let echo = markdown::SlashTrigger {
            line_start: trigger.line_start,
            line_end: self.cursor,
            cursor: self.cursor,
            // 选择器候选来自 worktree 状态，不按 query 过滤；占位即可。
            query: String::new(),
        };
        self.mention.request_worktree_list();
        self.slash = Some(SlashPanel::with_kind(
            echo,
            markdown::SlashPanelKind::WorktreeChooser {
                block_end_line,
                line,
            },
            Vec::new(),
            0,
        ));
        true
    }

    /// 斜杠命令面板打开时关闭且不改文本；二级选择器同样只关闭、命令行保留
    /// （对齐 lx-agent 的 `Esc` 只收起面板）；返回是否消费该按键。
    pub fn slash_escape(&mut self) -> bool {
        if self.slash.is_none() {
            return false;
        }
        self.slash = None;
        self.panel_suppressed = true;
        true
    }

    /// 重算块命令面板：触发标记决定候选列表，同类同位置保留高亮。
    pub(super) fn refresh_panel(&mut self) {
        let Some(trigger) = markdown::block_trigger(&self.text, self.cursor) else {
            self.panel = None;
            return;
        };
        let previous = self.panel.as_ref().filter(|panel| {
            panel.trigger().kind == trigger.kind && panel.trigger().to == trigger.to
        });
        let active = previous.map_or(0, BlockPanel::active);
        let items = markdown::block_commands(trigger.kind);
        let active = active.min(items.len().saturating_sub(1));
        self.panel = Some(BlockPanel::new(trigger, items, active));
    }

    /// 重算斜杠命令面板：整行 `/` 触发决定候选，同位置保留高亮。
    ///
    /// 二级 worktree 选择器不改算候选（由 worktree 结果与导航驱动），但按命令行锚定
    /// 同步：光标离开该行或该行文本变化即关闭（对齐 lx-agent 的二级面板同步），
    /// 关闭不改文本（`Esc` 同样只关闭）。
    pub(super) fn refresh_slash_panel(&mut self) {
        if self.worktree_chooser_open() {
            let anchored = self
                .slash
                .as_ref()
                .is_some_and(|panel| panel.worktree_anchored(&self.text, self.cursor));
            if anchored {
                return;
            }
            self.slash = None;
        }
        let Some(trigger) = markdown::slash_trigger(&self.text, self.cursor) else {
            self.slash = None;
            return;
        };
        let context = markdown::SlashContext {
            context: markdown::block_context(&self.text, self.cursor),
            // 绑定行需要锚定已闭合的结束行：块未闭合时不提供 `/gitWorkTree`。
            worktree_available: self.worktree_available
                && markdown::block_end_line_at(&self.text, self.cursor).is_some(),
        };
        let items = markdown::slash_filter(&self.custom, context, &trigger.query);
        if items.is_empty() {
            self.slash = None;
            return;
        }
        let previous = self
            .slash
            .as_ref()
            .filter(|panel| panel.trigger().line_start == trigger.line_start);
        let active = previous.map_or(0, SlashPanel::active).min(items.len() - 1);
        self.slash = Some(SlashPanel::new(trigger, items, active));
    }
}
