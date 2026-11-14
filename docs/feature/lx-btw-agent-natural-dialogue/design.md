# BTW / Agents 自然对话与子 agent `@` 输入 — 详细设计 (`design.md`)

- **特性目录**：`docs/feature/lx-btw-agent-natural-dialogue/`
- **关联任务**：[`task.md`](task.md)
- **目标范围**：`src/app/state/lx.rs`、`src/app/state/lx/{panels,mention,btw}.rs`、`src/app/prompt/mention.rs`、`src/app/update/pane/{task_page,popups}.rs`、`src/event/{mod,chat/btw}.rs`、`src/ui/main_content/lx/{mention.rs,task/{btw,agents}.rs}`、`src/ui/text/btw.rs`
- **当前状态**：**已实现于分支 `feat/btw-agent-natural-dialogue`，待合并**；验证见 [`task.md`](task.md) T6。

---

## 0. 代码库现状核验（codegraph / codebase-memory 结果）

| 事实维度 | 文件与行号 | 当前行数 | 与本设计的关系 |
| :--- | :--- | :--- | :--- |
| **BTW composer 右上角胶囊** | `src/ui/main_content/lx/task/btw.rs:160-219`（`draw_qa_badge`） | 306 行 | 当前拼出 ` QA 2 · 标题 · To: lx `。本次删除 ` · To: <回答者集合>` 整段。 |
| **BTW 回答者展示模型** | `src/app/state/lx/btw.rs:93-105`（`target_label`）、`120-127`（`effective_targets`） | 427 行 | 仅服务于胶囊文案；删胶囊后两者变成死代码，一并删除。 |
| **目标前缀文案** | `src/ui/text/btw.rs:32`（`LX_BTW_TARGETS_PREFIX = "To: "`） | 104 行 | 唯一使用点在 `target_label`，删除。 |
| **Agents composer 右上角胶囊** | `src/ui/main_content/lx/task/agents.rs:273-292`（`draw_agent_badge`） | 511 行 | 当前为 ` To: Turing [executor] `。本次改为 ` Turing [executor] `。 |
| **BTW assistant 的统计行标签来源** | `src/event/chat/btw.rs:665-701`（`start_recipient` 内 `new_btw_assistant_message` / `BtwRun.label`） | 1370 行（存量超 1000 行红线，本次不增加行数） | 当前把 `昵称 [目标类型]`（如 `lx [lx]`）塞进 `model_label`；统计行原样显示。本次改为该回答者的**模型显示名**。 |
| **统计行渲染** | `src/ui/main_content/lx/history/stats.rs:19-125`（`stats_line`） | 125 行 | `model_label` 位于统计行首段；主对话=模型名、BTW reload=模型名、BTW live=身份，本次统一为模型名后无需改渲染逻辑。 |
| **Agents 追问 composer 能力** | `src/app/state/lx.rs:266-272`（`agent_composer`） | 858 行 | 当前 `allow_panels = false`，`@` / `/` / md 块 / 路径粘贴面板全关。本次只开 `@`。 |
| **编辑器浮层总闸** | `src/app/state/lx/panels.rs:15-71`（`refresh_panel`） | 485 行 | 需要为 Agents composer 增加「只刷新 `@`」分支，其余面板保持关闭。 |
| **`@` 确认插入** | `src/app/state/lx/panels.rs:271-298`（`mention_confirm` / `mention_enter_folder`）、`src/app/state/lx/mention.rs:630-689`（`confirm`） | 826 行 | 当前所有入口都插入 `@[refer-*](...)` 标记。Agents composer 需要朴素文本 `@path `。 |
| **`@` 面板 Tab 集合** | `src/app/prompt/mention.rs:25-84`（`MentionTab` / `ALL` / `PROMPT_ALL` / `next` / `prev` / `next_prompt` / `prev_prompt`） | 828 行 | 已有「无 Subagents」的四项集合 `PROMPT_ALL`，Agents composer 直接复用。 |
| **Tab 切换与点击** | `src/app/state/lx/mention.rs:327-378`（`switch_tab_for_entry` / `set_tab_for_entry`）、`src/app/update/pane/popups.rs:147-157`（`switch_lx_mention_tab`） | 826 / 242 行 | 切换逻辑当前按「ai_web 布尔」分支；本次改为按编辑器可用 Tab 集合循环。 |
| **面板顶栏渲染** | `src/ui/main_content/lx/mention.rs:128-158`（`panel_data` 内 tabs） | 328 行 | 当前写死 `AiWeb → [Files]`，否则 5 项；本次改为读 `Lx::mention_tabs()`。 |
| **Agents 编辑态按键** | `src/app/update/pane/task_page.rs:237-254`（`apply_agent_edit_input`）、`256-272`（`edit_agent_composer`）、`538-574`（`enter_task_agent_edit`） | 718 行 | 需要插入 `@` 面板按键优先级：面板打开时 `Enter` 确认候选而不是发送。 |
| **BTW 键位参考实现** | `src/app/update/chat/btw/input.rs:263-303` | 344 行 | Agents 追问的 `@` 键位按这份实现镜像，保证两处手感一致。 |
| **`@` 扫描请求/回投路由** | `src/app/update/pane/popups.rs:19-30`（`sync_lx_mention_root`）、`50-74`（`take_lx_mention_scan_requests`）、`76-103`（`apply_lx_mention_entries`）、`src/event/mod.rs:56-63 / 723-729 / 985-999` | 242 / 1003 行 | 当前只区分「主输入框」与「BTW（按 QA）」；需要新增「Agents（按 agent_id）」归属，并做迟到结果校验。 |
| **`@` 上下文数据入口** | `src/event/inventory.rs:264-288`（MCP）、`570-596`（skills）、`675-690`（subagent roles） | 696 行 | skills / MCP 由事件层写入主编辑器；需要同步进 Agents composer。 |

> 结论：所有改动都落在既有模块职责内，不新增顶层模块、不改 DB schema、不新增迁移。

---

## 1. 目标与非目标

### 1.1 目标

1. **BTW composer 胶囊去目标段**：`QA 2 · 你好呀 · To: lx` → `QA 2 · 你好呀`。
2. **Agents composer 胶囊去 `To:` 前缀**：`To: Turing [executor]` → `Turing [executor]`。
3. **BTW 统计行不再显示回答者身份**：live 时把 `model_label` 从 `昵称 [目标类型]` 改为该回答者的模型显示名，与重启后从 DB 加载的展示一致；主对话与子 agent 详情继续显示各自模型名。
4. **Agents 追问 composer 支持 `@`**：
   - 只开启 `@` 提及面板；`/` 命令、markdown 块面板、路径粘贴参照面板保持关闭；
   - 顶栏只有 `Files / RefDir / Skills / MCP` 四项，不出现 `Subagents`；
   - 确认后插入朴素文本（`@src/main.rs `、`@/abs/folder `、`@skill-name `、`@server-name `），不生成 `@[refer-*]` 标记；
   - 发送给子 agent 的就是输入框里的同一段自然文本（`SubagentInputItem::Text`），不新增附件准备管线。
5. **键位手感与 BTW composer 一致**：`←/→` 切 Tab、`↑/↓` 移动高亮、`Tab/Enter` 确认候选、`Esc` 关闭面板（再按一次才退出编辑态）、`Shift+Enter` 在 Files 目录项上进入目录。

### 1.2 非目标

- 不改 SQLite schema，不新增迁移，不动 `agent_btw_*` 表。
- 不给子 agent 输入增加附件/引用内容注入（不会调用 `prepare_user_message`）。
- 不改主输入框与右侧 `Prompt` 编辑器的 `@` 行为。
- 不改 BTW 的目标解析、调度、只读回答与数据库语义。
- 不新增 `Subagents` 之外的 Tab，不调整 Tab 顺序。

---

## 2. Grill Me 已确认决策

| 编号 | 决策 | 用户确认结论 |
| :--- | :--- | :--- |
| **Q1** | 截图里两个被标记元素 | 指的是 composer 胶囊 `To: lx` 与统计行首段 `lx [lx]`；两处都要处理。 |
| **Q2** | 子 agent 回答是否保留身份 | 不保留。就是自然对话，不要身份标记；不区分默认 `lx` 还是 `@agent`。 |
| **Q3** | 最终形态 | BTW 胶囊 `QA 2 · 你好呀`；Agents 胶囊 `Turing [executor]`；BTW 统计行无身份；子 agent 详情统计行不动。 |
| **Q4** | BTW 统计行内容 | **显示模型即可**：live 与 reload 都用模型显示名（如 `deepseek-v4.1-flash · 5.4k in · …`）。 |
| **Q5** | Agents composer `@` 插入语义 | 朴素文本（方案 A）：插入 `@src/main.rs `，子 agent 收到同一段文本，不生成 marker、不做附件注入。 |
| **Q6** | Agents composer Tab 集合 | 复用 `MentionTab::PROMPT_ALL`：`Files / RefDir / Skills / MCP`，不显示 `Subagents`。 |
| **Q7** | 其它编辑浮层 | 只开 `@`；`/`、markdown 块面板、路径粘贴参照面板保持关闭。 |

---

## 3. 详细设计（含关键代码）

### 3.1 BTW composer 胶囊：删除目标段

**文件**：`src/ui/main_content/lx/task/btw.rs`（`draw_qa_badge`，160-219 行）
**关联删除**：`src/app/state/lx/btw.rs:93-105`、`:120-127`；`src/ui/text/btw.rs:32`

现状：胶囊由 `prefix`（`QA n · `）+ 截断后的标题 + `targets`（` · To: ...`）拼成，且 `targets` 与标题争抢 `QA_BADGE_MAX_COLS` 预算。

改后：

```rust
/// 主问题归属胶囊：` QA 2 · 标题 `，画在 composer 顶边框右侧。
fn draw_qa_badge(grid: &mut Grid, rect: Rect, lx: &Lx, marker_w: u16) {
    let Some(thread) = lx.btw.active_thread() else {
        return;
    };
    let index = lx
        .btw
        .qa
        .iter()
        .position(|summary| summary.qa_id == thread.qa_id)
        .map(|index| index.saturating_add(1));
    let title = lx
        .btw
        .qa
        .iter()
        .find(|summary| summary.qa_id == thread.qa_id)
        .map(|summary| summary.title.as_str())
        .filter(|title| !title.trim().is_empty())
        .unwrap_or(text::LX_BTW_QA_UNTITLED);
    let prefix = match index {
        Some(index) => format!("QA {index} · "),
        None => "QA · ".to_string(),
    };
    let width = |value: &str| {
        u16::try_from(unicode_width::UnicodeWidthStr::width(value)).unwrap_or(u16::MAX)
    };
    let max = rect.width.saturating_sub(12).min(QA_BADGE_MAX_COLS);
    let room = max.saturating_sub(width(prefix.as_str()));
    if room < 4 {
        return;
    }
    let title = text::truncate_to_width(title, usize::from(room));
    if title.is_empty() {
        return;
    }
    let badge = format!(" {prefix}{title} ");
    let badge_w = width(badge.as_str());
    if rect.width <= badge_w.saturating_add(marker_w).saturating_add(2) {
        return;
    }
    let x = rect.right().saturating_sub(badge_w).saturating_sub(1);
    if x <= rect.x.saturating_add(marker_w) {
        return;
    }
    grid.put(x, rect.y, &badge, style::btw_qa_badge());
}
```

同时删除：

```rust
// src/app/state/lx/btw.rs
/// 输入框上沿的目标标签：`To: lx, Turing [explorer]`。
pub fn target_label(&self) -> String { ... }

/// 当前有效回答者集合；为空时回退默认助手。
pub fn effective_targets(&self) -> Vec<BtwTargetView> { ... }
```

```rust
// src/ui/text/btw.rs
/// 输入框左侧目标标签。
pub const LX_BTW_TARGETS_PREFIX: &str = "To: ";
```

保留：

- `BtwThreadState.targets`（提交时仍要用）；
- `BtwTargetView::default_assistant()`（事件层仍在用）；
- `draw_qa_badge` 的窄窗格守卫（标记整体不画，绝不截半截）。

**验收**：宽 80 渲染 BTW 编辑态，顶边框只出现 `QA 1 · <标题>`；任何宽度下都不出现 `To:`。

---

### 3.2 Agents composer 胶囊：去掉 `To:` 前缀

**文件**：`src/ui/main_content/lx/task/agents.rs:273-292`

```rust
/// 右侧目标胶囊：` Turing [executor] `；左侧必须给 `Subagent` 标记留位。
fn draw_agent_badge(grid: &mut Grid, rect: Rect, run: &SubagentRun, marker_w: u16) {
    let name = run
        .nickname
        .as_deref()
        .filter(|nickname| !nickname.is_empty())
        .unwrap_or_else(|| run.role.as_str());
    let label = format!(" {name} [{}] ", run.role.as_str());
    let width = u16::try_from(label.width()).unwrap_or(u16::MAX);
    if rect.width <= width.saturating_add(marker_w).saturating_add(2) {
        return;
    }
    let x = rect.right().saturating_sub(width).saturating_sub(1);
    if x <= rect.x.saturating_add(marker_w) {
        return;
    }
    grid.put(x, rect.y, &label, style::btw_qa_badge());
}
```

**验收**：`agent_composer_renders_target_badge_and_draft` 改为断言 `Turing [executor]` 且不出现 `To:`。

---

### 3.3 BTW 统计行：身份 → 模型显示名

**文件**：`src/event/chat/btw.rs`（`start_recipient`，665-701 行）

现状把 `昵称 [target_kind]` 写进 `new_btw_assistant_message` 的 `model_label`；`BtwRun.label` 也是同一串。

改后：

```rust
// 统计行展示回答者的模型，而不是回答者身份：live 与 reload（view_message_of）
// 使用同一份 `ModelsSnapshot`，同一段对话不会因为加载路径不同而变样。
let model_label = state
    .models
    .model_display_name(&start.model.provider, &start.model.model);
let mut buffer = crate::app::state::LxChat::new();
buffer
    .messages
    .push(crate::app::update::new_btw_assistant_message(
        &start.assistant_message_id,
        &model_label,
    ));
let run = BtwRun {
    // ...
    label: model_label,
};
```

要点：

- `BtwRecipientStart.model: ModelSnapshot { provider, model, variant }` 在回答者启动时已冻结，`model_display_name` 未命中配置时回退 `model_id`，与 reload 的 `view_message_of` 完全同源（`src/app/update/chat/view.rs:42-46`）。
- `BtwRun.label` 字段名与用途不变（只用于 `RecipientStarted`），不扩散改动。
- 父会话主对话本身仍显示模型名，不受影响；子 agent 详情由 `view_message_of` 决定，同样不受影响。
- 不在渲染层加 BTW 特判：避免 live / reload 两条路径再次分叉。

**验收**：BTW 回答完成后的统计行形如 `deepseek-v4.1-flash · 5.4k in · 166 out · 71.1 tok/s · 2.3s`；重启恢复同一线程后该行不变。

---

### 3.4 Agents 追问 composer：开启 `@`

#### 3.4.1 编辑器能力开关

**文件**：`src/app/state/lx.rs`

新增字段：

```rust
/// Agents 追问 composer：只开 `@`（4 Tab、朴素插入），其余编辑浮层保持关闭。
mention_minimal: bool,
```

`build` 默认 `false`；`agent_composer()`：

```rust
/// Agents 追问输入缓冲：文本编辑 + `@` 提及（4 Tab、朴素插入）；
/// `/`、markdown 块面板、路径粘贴参照面板保持关闭。
pub fn agent_composer() -> Self {
    let mut lx = Self::build(false);
    lx.no_shell = true;
    lx.allow_panels = false;
    lx.mention_minimal = true;
    lx
}
```

新增 Tab 集合读取：

```rust
/// `@` 面板可用 Tab：AI Web 仅 Files；Agents 追问不含会话态 Subagents；其余 5 项。
pub fn mention_tabs(&self) -> &'static [crate::app::prompt::MentionTab] {
    use crate::app::prompt::MentionTab;
    if self.entry == LxEntry::AiWeb {
        &[MentionTab::Files]
    } else if self.mention_minimal {
        &MentionTab::PROMPT_ALL
    } else {
        &MentionTab::ALL
    }
}
```

> 说明：`&[MentionTab::Files]` 为常量提升（promoted constant），返回 `&'static [_]` 合法；不改 `MentionTab::ALL/PROMPT_ALL` 的现有语义。

#### 3.4.2 `refresh_panel`：只刷新提及面板

**文件**：`src/app/state/lx/panels.rs:15-71`

```rust
pub fn refresh_panel(&mut self) {
    self.text_selection = None;
    self.preview_selection = None;
    self.preview_drag = None;
    if !self.allow_panels {
        self.panel = None;
        self.slash = None;
        self.paste = None;
        self.text_paste = None;
        self.refer = None;
        if self.mention_minimal {
            // Agents 追问：只重算 `@`，其余浮层保持关闭。
            let masked = crate::app::paste::mask_pasted_text(&self.input).into_owned();
            self.mention.unsuppress();
            self.mention.refresh(&masked, self.cursor);
        } else {
            self.mention.suppress();
        }
        self.scroll_cursor_into_view();
        return;
    }
    // ... 其余逻辑不变
}
```

注意：`mention_minimal` 分支不调用 `sync_mention_subagent_runs()`（该 Tab 不显示，避免跨会话实例列表渗入）。

#### 3.4.3 朴素插入

**文件**：`src/app/state/lx/mention.rs:630-689`、`src/app/state/lx/panels.rs:271-298`

给 `LxMention::confirm` 增加 `plain: bool`：

```rust
/// 确认高亮条目：关闭面板、进入压制，返回替换区间与插入文本。
///
/// `plain = true` 时只写入朴素文本（Agents 追问），不生成 `@[refer-*]` 标记；
/// 标记语法在子 agent 侧没有消费方，写 marker 只会把丑文本转嫁给模型。
pub fn confirm(&mut self, text_len: usize, plain: bool) -> Option<(Range<usize>, String)> {
    if self.tab != MentionTab::Files {
        let trigger = self.trigger.as_ref()?;
        if trigger.from > trigger.to || trigger.to > text_len {
            return None;
        }
        let range = trigger.from..trigger.to;
        let ref_text = match self.tab {
            MentionTab::RefDir => {
                let folder = *self.filtered_ref_folders().get(self.ref_active)?;
                if plain {
                    format!("@{}", folder.path.display())
                } else {
                    crate::app::markdown::format_reference(
                        crate::app::markdown::ReferKind::Folder,
                        &folder.path,
                    )
                    .unwrap_or_else(|| format!("@[refer-folder]({})", folder.path.display()))
                }
            }
            MentionTab::Skills => {
                let skill = *self.filtered_skills().get(self.ref_active)?;
                if plain {
                    format!("@{}", skill.name)
                } else {
                    crate::app::markdown::format_reference(
                        crate::app::markdown::ReferKind::Skill,
                        &skill.path,
                    )
                    .unwrap_or_else(|| format!("@[refer-skill]({})", skill.path.display()))
                }
            }
            MentionTab::Mcp => {
                let server = *self.filtered_mcp_servers().get(self.ref_active)?;
                if plain {
                    format!("@{}", server.name)
                } else {
                    crate::app::markdown::format_reference(
                        crate::app::markdown::ReferKind::Mcp,
                        Path::new(&server.name),
                    )
                    .unwrap_or_else(|| format!("@[refer-mcp]({})", server.name))
                }
            }
            MentionTab::Subagents => {
                let item = *self.filtered_subagents().get(self.ref_active)?;
                let target = item.reference_target();
                if plain {
                    format!("@{target}")
                } else {
                    crate::app::markdown::format_reference(
                        crate::app::markdown::ReferKind::Subagent,
                        Path::new(&target),
                    )
                    .unwrap_or_else(|| format!("@[refer-subagent]({target})"))
                }
            }
            MentionTab::Files => unreachable!(),
        };
        let insertion = format!("{ref_text} ");
        self.reset_session();
        self.suppressed = true;
        return Some((range, insertion));
    }
    let panel = self.panel.as_ref()?;
    // Files 分支现有插入已经是 `@path ` / `@dir/ `（mention_insertion），
    // plain 与 marker 两种模式共用同一段朴素文本，无需分支。
    // ...（其余逻辑保持现状）
}
```

> 实现注意：`plain` 只影响 RefDir / Skills / MCP / Subagents 四个二级 Tab；Files 本来就是 `@path `。Subagents 在 `mention_minimal` 下不可达，`@<target>` 仅作兜底。

`Lx::mention_confirm`：

```rust
pub fn mention_confirm(&mut self) -> bool {
    if self.entry == super::LxEntry::AiWeb && self.mention.active_is_directory() {
        return self.mention_enter_folder();
    }
    let plain = self.mention_minimal;
    let Some((range, insertion)) = self.mention.confirm(self.input.len(), plain) else {
        return false;
    };
    // ... 其余不变
}
```

#### 3.4.4 Tab 循环按可用集合

**文件**：`src/app/state/lx/mention.rs`

不新增 `cycle_in`：`MentionTab::PROMPT_ALL` 的顺序就是 4 Tab 循环顺序，直接按集合下标取模即可，`LxMention` 只负责「集合内循环」这一件事：

```rust
/// 切换 Tab：在给定 Tab 集合内循环；未处于可见状态返回 false。
pub fn switch_tab(
    &mut self,
    delta: isize,
    text: &str,
    cursor: usize,
    tabs: &[MentionTab],
) -> bool {
    if tabs.len() < 2 || self.view(text, cursor) == LxMentionView::Hidden {
        return false;
    }
    let index = tabs.iter().position(|tab| *tab == self.tab).unwrap_or(0) as isize;
    let len = tabs.len() as isize;
    let next = tabs[(index + delta).rem_euclid(len) as usize];
    if next == self.tab {
        return false;
    }
    self.tab = next;
    self.ref_active = 0;
    self.ref_anchor = 0;
    self.ref_viewport = None;
    true
}

/// 在给定 Tab 集合内直接设置活跃 Tab；不在集合内返回 false。
pub fn set_tab(
    &mut self,
    tab: MentionTab,
    text: &str,
    cursor: usize,
    tabs: &[MentionTab],
) -> bool {
    if !tabs.contains(&tab)
        || self.view(text, cursor) == LxMentionView::Hidden
        || self.tab == tab
    {
        return false;
    }
    self.tab = tab;
    self.ref_active = 0;
    self.ref_anchor = 0;
    self.ref_viewport = None;
    true
}
```

`Lx::mention_switch_tab` / `mention_set_tab` 改为：

```rust
pub fn mention_switch_tab(&mut self, delta: isize) -> bool {
    self.sync_mention_subagent_runs();
    let tabs = self.mention_tabs();
    self.mention.switch_tab(delta, &self.input, self.cursor, tabs)
}

pub fn mention_set_tab(&mut self, tab: crate::app::prompt::MentionTab) -> bool {
    self.sync_mention_subagent_runs();
    let tabs = self.mention_tabs();
    self.mention.set_tab(tab, &self.input, self.cursor, tabs)
}
```

原 `switch_tab_for_entry` / `set_tab_for_entry` 的 `ai_web: bool` 由 `mention_tabs()` 返回的集合取代（AI Web 的集合就是 `[Files]`，`tabs.len() < 2` / `!tabs.contains(&tab)` 自然拒绝切换），旧方法随之删除，不保留双入口。

#### 3.4.5 面板顶栏渲染读 `mention_tabs`

**文件**：`src/ui/main_content/lx/mention.rs:128-158`

```rust
fn tab_label(tab: MentionTab) -> &'static str {
    match tab {
        MentionTab::Files => text::MENTION_TAB_FILES,
        MentionTab::RefDir => text::MENTION_TAB_REF_DIR,
        MentionTab::Skills => text::MENTION_TAB_SKILLS,
        MentionTab::Mcp => text::MENTION_TAB_MCP,
        MentionTab::Subagents => text::MENTION_TAB_SUBAGENTS,
    }
}

fn panel_tabs(lx: &Lx) -> Vec<PanelTab<'static>> {
    lx.mention_tabs()
        .iter()
        .map(|tab| PanelTab {
            label: tab_label(*tab),
            active: lx.mention().tab() == *tab,
        })
        .collect()
}
```

`panel_data` 中原来的 `if lx.entry == LxEntry::AiWeb { ... } else { ... }` 整段替换为 `let tabs = panel_tabs(lx);`。

> 鼠标点击路径不用改：`switch_lx_mention_tab` 用 `MentionTab::from_index(index)`，0..=3 在两个集合里顺序一致；第 5 个 Tab 在 Agents composer 里根本不渲染，`set_tab` 的集合校验也兜底拒绝 `Subagents`。

#### 3.4.6 `@` 上下文同步

**文件**：`src/app/state/lx/panels.rs`

新增：

```rust
/// 把主编辑器的 `@` 上下文复制给当前 Agents 追问 composer。
///
/// 进入编辑态、工作区切换、skills / MCP / 引用文件夹刷新后调用；
/// 没有正在编辑的 composer 时返回 false。
pub fn sync_agent_composer_context(&mut self) -> bool {
    let Some(agent_id) = self.agent_edit_id().map(str::to_string) else {
        return false;
    };
    let root = self.mention.root().map(std::path::Path::to_path_buf);
    let project = self.chat.project_root.clone();
    let folders = self.mention.referenced_folders().to_vec();
    let skills = self.mention_skills();
    let mcp = self.mention_mcp_servers();
    let Some(composer) = self.agents.composer_mut(&agent_id) else {
        return false;
    };
    let root_changed = composer.set_mention_root(root);
    let project_changed = composer.set_project_root(project);
    let folder_changed = composer.sync_referenced_folders(&folders);
    composer.set_mention_skills(skills);
    composer.set_mention_mcp_servers(mcp);
    root_changed || project_changed || folder_changed
}
```

调用点：

1. `enter_task_agent_edit`（`task_page.rs:538-574`）在 `ensure_composer` 之后、置 `agent_edit` 之后调用；进入时顺手 `mention_escape()` 清掉上一次会话残留的候选：
   ```rust
   target.lx.agents.ensure_composer(&agent_id);
   if let Some(page) = target.lx.task.as_mut() {
       page.agent_edit = Some(agent_id.clone());
   }
   if let Some(composer) = target.lx.agents.composer_mut(&agent_id) {
       composer.editor_mut().mention_escape();
   }
   target.lx.sync_agent_composer_context();
   ```

2. `sync_lx_mention_root`（`popups.rs:19-30`）结尾追加：
   ```rust
   let composer = pane.lx.sync_btw_composer_context(root, project);
   let agent_composer = pane.lx.sync_agent_composer_context();
   ref_changed || mention || project_changed || composer || agent_composer
   ```

3. 不把同步挂进 `set_mention_skills` / `set_mention_mcp_servers` / `sync_referenced_folders`：这三个 setter 会由事件层高频调用，挂进去等于每次数据刷新都重算 composer 面板；进入编辑态与根同步两处已经覆盖实际使用路径，避免无谓的横向耦合。

#### 3.4.7 扫描请求与回投归属

**文件**：`src/app/update/pane/popups.rs`

新增：

```rust
/// `@` 面板扫描请求的归属编辑器。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MentionScanOwner {
    /// 主输入框。
    Main,
    /// BTW 线程 composer；只有仍显示该 QA 的线程才接收。
    Btw(String),
    /// Agents 追问 composer；只有仍编辑该 agent 时才接收。
    Agent(String),
}
```

`take_lx_mention_scan_requests`：

```rust
pub fn take_lx_mention_scan_requests(
    state: &mut AppState,
) -> Vec<(PaneId, MentionScanOwner, u64, PathBuf)> {
    let visible = state.active_tab().layout.pane_ids();
    let mut requests = Vec::new();
    for id in visible {
        let Some(pane) = state.pane_mut_anywhere(id) else {
            continue;
        };
        if pane.view != PaneView::Lx {
            continue;
        }
        if let Some((generation, root)) = pane.lx.take_mention_scan_request() {
            requests.push((id, MentionScanOwner::Main, generation, root));
        }
        let active_qa = pane.lx.btw.active.clone();
        if pane.lx.btw_editing()
            && let Some(composer) = pane.lx.btw.active_composer_mut()
            && let Some((generation, root)) = composer.take_mention_scan_request()
            && let Some(qa_id) = active_qa
        {
            requests.push((id, MentionScanOwner::Btw(qa_id), generation, root));
        }
        if pane.lx.agents_editing()
            && let Some(agent_id) = pane.lx.agent_edit_id().map(str::to_string)
            && let Some(composer) = pane.lx.agents.composer_mut(&agent_id)
            && let Some((generation, root)) = composer.take_mention_scan_request()
        {
            requests.push((id, MentionScanOwner::Agent(agent_id), generation, root));
        }
    }
    requests
}
```

`apply_lx_mention_entries`：

```rust
pub fn apply_lx_mention_entries(
    state: &mut AppState,
    id: PaneId,
    owner: MentionScanOwner,
    generation: u64,
    root: &std::path::Path,
    entries: Vec<crate::app::markdown::MentionEntry>,
) {
    let Some(pane) = state.pane_mut_anywhere(id) else {
        return;
    };
    match owner {
        MentionScanOwner::Main => {
            pane.lx.apply_mention_entries(generation, root, entries);
        }
        MentionScanOwner::Btw(qa_id) => {
            if pane.lx.btw.active.as_deref() != Some(qa_id.as_str()) {
                return;
            }
            if let Some(composer) = pane.lx.btw.active_composer_mut() {
                composer.apply_mention_entries(generation, root, entries);
            }
        }
        MentionScanOwner::Agent(agent_id) => {
            if pane.lx.agent_edit_id() != Some(agent_id.as_str()) {
                return;
            }
            if let Some(composer) = pane.lx.agents.composer_mut(&agent_id) {
                composer.apply_mention_entries(generation, root, entries);
            }
        }
    }
}
```

**文件**：`src/event/mod.rs`

```rust
LxMentionScanned {
    pane: PaneId,
    owner: update::MentionScanOwner,
    generation: u64,
    root: PathBuf,
    entries: Vec<MentionEntry>,
},
```

`pump_lx_mention_scan` 与 dispatch 跟随改名；`AppEvent` 是内部事件，无外部兼容负担。

#### 3.4.8 编辑态键位

**文件**：`src/app/update/pane/task_page.rs`

```rust
fn apply_agent_edit_input(state: &mut AppState, pane: PaneId, command: LxInput) -> bool {
    // `@` 面板优先：Enter 先采纳候选，不会把半截输入发给子 agent。
    if apply_agent_mention_input(state, pane, command) {
        return true;
    }
    match command {
        LxInput::Escape => leave_agent_edit(state, pane),
        LxInput::Enter => submit_agent_edit(state, pane),
        LxInput::StopProcess => {
            chat::stop_selected_subagent(state, pane);
            true
        }
        LxInput::PageUp | LxInput::PageDown => {
            let delta = if command == LxInput::PageUp { -10 } else { 10 };
            chat::scroll_agents_detail(state, pane, delta);
            true
        }
        LxInput::Tab | LxInput::PrevStep => true,
        _ => edit_agent_composer(state, pane, command),
    }
}

/// `@` 面板按键：镜像 BTW composer（`chat/btw/input.rs:263-303`）。
fn apply_agent_mention_input(state: &mut AppState, pane: PaneId, command: LxInput) -> bool {
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    let Some(agent_id) = target.lx.agent_edit_id().map(str::to_string) else {
        return false;
    };
    let Some(composer) = target.lx.agents.composer_mut(&agent_id) else {
        return false;
    };
    let editor = composer.editor_mut();
    if editor.mention_visible() {
        match command {
            LxInput::Left => {
                editor.mention_switch_tab(-1);
                return true;
            }
            LxInput::Right => {
                editor.mention_switch_tab(1);
                return true;
            }
            LxInput::Escape => {
                editor.mention_escape();
                return true;
            }
            LxInput::EnterFolder => {
                editor.mention_enter_folder();
                return true;
            }
            _ => {}
        }
    }
    if editor.mention_open() {
        match command {
            LxInput::Up => {
                editor.mention_move(-1);
                return true;
            }
            LxInput::Down => {
                editor.mention_move(1);
                return true;
            }
            LxInput::Tab | LxInput::Enter => {
                editor.mention_confirm();
                return true;
            }
            _ => {}
        }
    }
    if editor.mention_visible() {
        // 面板可见但只有状态行（loading / 无匹配）：按键继续编辑过滤词，
        // Enter 不发送，避免用户以为在选候选却把半截文本发出去。
        super::apply_editor_edit(editor, command);
        return true;
    }
    false
}
```

行为清单：

| 键 | 面板可见 | 面板隐藏 |
| :--- | :--- | :--- |
| `Enter` | 有候选：采纳；无候选：吞掉 | 发送 |
| `Tab` | 有候选：采纳 | 不动（保持现状吞键） |
| `Esc` | 关闭面板（压制） | 退出编辑态 |
| `↑/↓` | 移动候选 | 光标移动 |
| `←/→` | 切换 Tab | 光标移动 |
| `Shift+Enter` | Files 目录项进入目录 | 换行 |

#### 3.4.9 进入编辑态即同步

`enter_task_agent_edit` 中 `ensure_composer` 之后调用 `sync_agent_composer_context()`；同时 `sync_lx_mention_root` 会在工作区/根变化时补同步，保证文件扫描根不为空。

---

## 4. 数据流

```text
用户敲 `@`（Agents composer）
  → Lx::insert_char('@') → refresh_panel()
    → !allow_panels && mention_minimal
      → mention.refresh(masked, cursor) → 触发 trigger
  → 需要扫描：take_lx_mention_scan_requests
      → (pane, MentionScanOwner::Agent(agent_id), generation, root)
  → event::pump_lx_mention_scan → files::scan(root)
  → AppEvent::LxMentionScanned(owner=Agent(agent_id))
  → apply_lx_mention_entries
      → 仍编辑该 agent？→ composer.apply_mention_entries
  → 面板渲染：ui::main_content::lx::mention::render(active_editor)
  → Enter / Tab
      → Lx::mention_confirm（plain = mention_minimal）
      → 写入 `@src/main.rs `
  → Enter（面板已关）→ submit_agent_edit
      → Effect::SendSubagentUserInput { text: "@src/main.rs ..." }
```

---

## 5. 边界与失败模式

| 场景 | 预期行为 |
| :--- | :--- |
| 切到别的 agent 后迟到的扫描结果 | `MentionScanOwner::Agent(agent_id)` 校验失败 → 丢弃。 |
| 切 tab / 关面板后迟到的 BTW 扫描结果 | 沿用既有 QA 校验。 |
| `@` 面板打开时按 Enter | 不发送，采纳候选或吞键。 |
| 面板只有 loading / 无匹配时按 Enter | 不发送。 |
| `Esc` | 第一下关面板（保留 `@` 文本），第二下退出编辑态。 |
| 子 agent 已取消 | 沿用 `submit_agent_edit` 的 closed 检查与 toast。 |
| 模型名未命中配置 | `model_display_name` 回退 `model_id`，live 与 reload 一致。 |
| 窄窗格 | BTW / Agents 胶囊整体不画，绝不截半截（沿用现有守卫）。 |
| AI Web 主输入框 | `mention_tabs()` 仍返回 `[Files]`，行为不变。 |

---

## 6. 测试计划

### 6.1 单元/渲染测试

1. `src/ui/main_content/lx/task/tests/btw_list.rs`（或邻近文件）
   - BTW 编辑态宽 80：顶边框含 `QA 1 · <标题>`，不含 `To:`。
   - 窄宽度：胶囊整体不绘制，无越界。
2. `src/ui/main_content/lx/task/tests/agent_list.rs:842-870`
   - 胶囊含 `Turing [executor]`，不含 `To:`。
   - `agent_composer_marker_and_badge_stay_inside_narrow_rect` 保持通过。
3. `src/app/state/lx/mention/tests.rs`
   - `mention_minimal` 的编辑器：`mention_tabs()` = 4 项，`switch_tab_in` 从 `Mcp` 向后回到 `Files`，永远不会停在 `Subagents`。
   - 朴素插入：Files → `@dir/file.rs `；RefDir → `@<abs path> `；Skills → `@skill-name `；MCP → `@server-name `。
   - 主输入框仍插入 marker（回归）。
4. `src/app/update/chat/tests/task_page.rs`
   - `@` 面板打开时 Enter 不发送、不产生 `Effect::SendSubagentUserInput`；确认后草稿为 `@src/main.rs `。
   - Esc 先关面板，再 Esc 才退回列表。
5. `src/app/update/tests.rs`（扫描路由）
   - Agents composer 产生 `MentionScanOwner::Agent(agent_id)` 请求；
   - 回投时 agent 已切换 → 结果被丢弃。
6. `src/app/update/chat/btw/tests.rs`
   - 删除 `default_target_sentinel_renders_as_lx_not_an_agent_id`（随 `target_label` 一起下线）。
7. `src/event/chat/btw.rs` / `src/app/update/chat/view/tests.rs`
   - 验证 `model_display_name` 在缺失配置时回退 `model_id`（复用 `ModelsSnapshot::default()`）。

### 6.2 精准验证命令

```bash
cargo fmt
cargo clippy --all-targets -- -D warnings
cargo test --lib ui::main_content::lx::task
cargo test --lib app::state::lx
cargo test --lib app::update::pane
cargo test --lib app::update::chat::btw
cargo test --lib event::chat::tests
```

> 交互观感（Tab 切换、Enter 采纳、Esc 两级退出、胶囊观感）由用户运行 `cargo run` 复核。

---

## 7. 影响面与回滚

- **无 DB 迁移**：不改 schema、不改任何持久化字段语义。
- **无外部协议变更**：`AppEvent::LxMentionScanned` 只在本进程内使用。
- **回滚路径**：四处改动（BTW 胶囊 / Agents 胶囊 / BTW 统计标签 / Agents composer `@`）互相独立，可单独回退；`mention_minimal` 默认 `false`，关闭该字段即回到纯文本 composer。
- **性能**：Agents composer 的扫描请求只在编辑态发出；文件扫描与现状主输入框同源，无新增轮询。

---

## 8. 未决 / 风险

1. **`BtwRecipientStart.model` 是否总是冻结了回答者模型**：已核验 `src/db/chat/btw.rs:128-141` 携带 `model: ModelSnapshot`；若历史线程目标模型缺失，仍会回退默认模型，展示名随之回退，可接受。
2. **`src/event/mod.rs` 与 `src/event/chat/btw.rs` 已超 1000 行红线**：本次不新增行数，只在既有结构内替换；不在本任务内做拆分。
3. **`@` 面板在 Agents composer 的根目录来源**：依赖 `sync_lx_mention_root` 已注入 root；若用户在从未选中过工作区的空会话里打开 Agents 详情，面板会显示空态而不是崩溃（`view()` 已有 root 判定）。

---

## 9. 代码审查修复（2026-11-14）

完整报告见 [`review.md`](review.md)。本 feature 范围内的修复：

- **迟到扫描清理**：`LxMention::abandon_scan` / `Lx::abandon_mention_scan` 在 owner 切换、结果被丢弃时清掉原 composer 的在途标记；否则 Files Tab 会永久停在 Loading。
- **Shift+Enter 回落**：`mention_enter_folder()` 未消费时回落为换行，Agents 与 BTW composer 同步。
- **上下文同步短路**：`set_skills` / `set_mcp_servers` / `set_disabled_mcp_servers` 内容相同直接返回，避免每帧对 20k 条扫描结果重复过滤。
- **禁用 MCP 继承**：Agents composer 同步当前窗格的 `disabled_mcp_servers`。
- **死字段清理**：`BtwTargetView` 删除已无读取方的 `key` / `label`；`resolve_target_views` 不再为展示标签查 `list_subagents`。
- **测试补强**：迟到扫描恢复、Shift+Enter 回落、`/` 真实路由、满队列保留草稿、BTW 窄窗格胶囊。
