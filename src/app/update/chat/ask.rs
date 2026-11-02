//! `ask` 面板的按键、鼠标与提交：纯状态迁移，只登记 `Effect`，不碰 IO。
//!
//! 交互对齐 `opencode` 的 form 面板（`packages/tui/src/routes/session/form.tsx`）：
//!
//! - **tab**：多题（或单题多选）时 `⇆`（`Tab`/`→`/`Shift+Tab`/`←`）在题目与
//!   `Submit` 汇总间循环；单题单选没有 tab，选中即提交。
//! - **列表态**：`↑`/`↓` 循环移动高亮；`1..9` 直接选中并确认；在 `Other` 行直接
//!   键入会就地开始编辑并写入首字符（`opencode` 的 custom 拦截器）。
//! - **单选**：在选项上按 `Enter` 立即选中并完成本题（单题提交、多题进下一题）。
//! - **多选**：选项只勾选，必须聚焦选项列表底部的 ` ↵ submit `（或 `Submit` tab）
//!   才提交；`Space` 切换勾选。
//! - **提交**：未答完时给窗格内 toast，并跳到第一个未作答的题目。
//!
//! 两条硬约束：
//!
//! 1. **草稿不变量**：这里任何路径都不读、不写 `Lx.input` / `Lx.cursor` / 撤销栈；
//!    手写文本只进 `AskEditor`。
//! 2. **单一提交入口**：只有 [`submit_all`] 会组装 `AskAnswers` 并清面板；迟到的
//!    重复点击因此天然幂等（`lx.ask` 已经是 `None`）。

use std::collections::BTreeMap;

use crate::app::actions::LxInput;
use crate::app::effects::Effect;
use crate::app::state::lx::{AskHit, AskWheelZone};
use crate::app::state::{AppState, AskState};
use crate::app::toast::{Toast, ToastKind};
use crate::layout::PaneId;
use crate::ui::text::agent::ASK_TOAST_ANSWER_REQUIRED;

use super::super::WHEEL_LINES;
use super::escape_stop;

/// 手写答案超过字节上限、写入被拒时的 toast 文案。
pub const ASK_HINT_TOO_LONG: &str = "answer is too long";

/// `Submit` 汇总 tab 里 `↑`/`↓` 的单次滚动步长（一屏内容按行细看）。
const CONFIRM_SCROLL_STEP: isize = 1;

/// 单选题的收尾动作（完成后需要跳出 `ask` 的可变借用）。
enum Follow {
    Submit,
    Next,
    None,
}

/// ask 面板接管 lx 输入；返回是否产生状态变化。
pub fn apply_input(state: &mut AppState, pane: PaneId, command: LxInput) -> bool {
    let Some(ask) = state
        .pane_mut_anywhere(pane)
        .and_then(|target| target.lx.ask.as_mut())
    else {
        return false;
    };
    if ask.editing {
        // 插入类按键先按字节预算判定：超限拒绝写入（绝不截断），并给出窗格内提示。
        let rejected = inserted_bytes(&command)
            .is_some_and(|bytes| ask.editor().is_some_and(|editor| !editor.can_insert(bytes)));
        let changed = apply_edit_input(state, pane, command);
        if rejected {
            hint(state, pane, ASK_HINT_TOO_LONG);
        }
        return changed;
    }
    match command {
        // `Submit` 汇总里 `↑`/`↓` 是滚动，不是移动高亮。
        LxInput::Up if ask.is_confirm() => ask.scroll_window(-CONFIRM_SCROLL_STEP),
        LxInput::Up => ask.move_highlight(-1),
        LxInput::Down if ask.is_confirm() => ask.scroll_window(CONFIRM_SCROLL_STEP),
        LxInput::Down => ask.move_highlight(1),
        // `⇆`：左右与 Tab / Shift+Tab 都在 tab 间循环（含 `Submit` 汇总）。
        LxInput::Left | LxInput::PrevStep => ask.prev_step(),
        LxInput::Right | LxInput::Tab => ask.next_step(),
        // Space：选项勾选、「其他」进入行内编辑、提交等价 Enter。
        LxInput::Char(' ') => space(state, pane),
        // 在 `Other` 行直接键入：就地开始编辑并写入首字符；`1..9` 是快捷选择。
        LxInput::Char(ch) => type_key(state, pane, ch),
        // Enter：选中/前进/提交，规则见 [`enter`]。
        LxInput::Enter => enter(state, pane),
        // 翻页统一滚动整块列表（选项 + 角色画预览）。
        LxInput::PageUp => ask.scroll_page(-1),
        LxInput::PageDown => ask.scroll_page(1),
        // Esc：沿用双击确认（第一次只待命，第二次取消整轮）。
        LxInput::Escape => escape_stop(state, pane),
        // 其余键吞掉：绝不让它们漏进被隐藏的草稿。
        _ => false,
    }
}

/// 列表态字符键：`Other` 行的即输即编 + `1..9` 数字快捷选择。
fn type_key(state: &mut AppState, pane: PaneId, ch: char) -> bool {
    let Some(ask) = state
        .pane_anywhere(pane)
        .and_then(|target| target.lx.ask.as_ref())
    else {
        return false;
    };
    if ask.is_confirm() || ch.is_control() {
        return false;
    }
    let on_other = ask.highlight.get(ask.current).copied().unwrap_or(0) == ask.other_row();
    if on_other && !('1'..='9').contains(&ch) {
        // 「其他」行直接键入即开始编辑：首字符不能丢，也不能先去按 Enter。
        let Some(ask) = state
            .pane_mut_anywhere(pane)
            .and_then(|target| target.lx.ask.as_mut())
        else {
            return false;
        };
        let begun = ask.begin_edit();
        let inserted = ask.editor_mut().is_some_and(|editor| {
            if !editor.can_insert(ch.len_utf8()) {
                return false;
            }
            editor.insert_char(ch);
            true
        });
        if !inserted {
            hint(state, pane, ASK_HINT_TOO_LONG);
        }
        return begun || inserted;
    }
    if !('1'..='9').contains(&ch) {
        return false;
    }
    let index = (ch as usize).saturating_sub('1' as usize);
    if index > ask.other_row() {
        return false;
    }
    if let Some(ask) = state
        .pane_mut_anywhere(pane)
        .and_then(|target| target.lx.ask.as_mut())
    {
        ask.set_highlight(index);
    }
    enter(state, pane)
}

/// `Space`：普通选项勾选，「其他」进入编辑（多选已写文本则切换计入），`Submit` 汇总页等价 Enter。
fn space(state: &mut AppState, pane: PaneId) -> bool {
    let Some(ask) = state
        .pane_anywhere(pane)
        .and_then(|target| target.lx.ask.as_ref())
    else {
        return false;
    };
    if ask.is_confirm() {
        return enter(state, pane);
    }
    let highlight = ask.highlight.get(ask.current).copied().unwrap_or(0);
    let on_other = highlight == ask.other_row();
    let multi = ask.question().is_some_and(|question| question.multi_select);
    let custom = ask.editor().map(|editor| editor.text()).unwrap_or_default();
    let Some(ask) = state
        .pane_mut_anywhere(pane)
        .and_then(|target| target.lx.ask.as_mut())
    else {
        return false;
    };
    if on_other {
        // 多选里「其他」已写文本：Space 只是把这段文本计入/移出答案。
        if multi && !custom.trim().is_empty() {
            let current = ask.current;
            let next = !ask.custom_on.get(current).copied().unwrap_or(false);
            ask.custom_on[current] = next;
            return true;
        }
        return ask.begin_edit();
    }
    ask.choose(highlight)
}

/// `Enter` 的统一入口：`Submit` 汇总 tab > 其他 > 普通选项。
fn enter(state: &mut AppState, pane: PaneId) -> bool {
    let Some(ask) = state
        .pane_anywhere(pane)
        .and_then(|target| target.lx.ask.as_ref())
    else {
        return false;
    };
    // 在 `Submit` 汇总 tab 按 Enter：全部作答才提交，否则提示并跳到第一个未答题。
    if ask.is_confirm() {
        return submit_or_warn(state, pane);
    }
    let highlight = ask.highlight.get(ask.current).copied().unwrap_or(0);
    // 焦点在「其他」：原地进入行内编辑（再次回车才确认文本）。
    if highlight == ask.other_row() {
        return state
            .pane_mut_anywhere(pane)
            .and_then(|target| target.lx.ask.as_mut())
            .is_some_and(AskState::begin_edit);
    }
    let multi = ask.question().is_some_and(|question| question.multi_select);
    let single_only = ask.is_single_select_only();
    let (chosen, follow) = {
        let Some(ask) = state
            .pane_mut_anywhere(pane)
            .and_then(|target| target.lx.ask.as_mut())
        else {
            return false;
        };
        let chosen = ask.choose(highlight);
        let follow = if !chosen || multi {
            Follow::None
        } else if single_only {
            Follow::Submit
        } else {
            Follow::Next
        };
        (chosen, follow)
    };
    match follow {
        Follow::Submit => submit_all(state, pane) || chosen,
        Follow::Next => {
            state
                .pane_mut_anywhere(pane)
                .and_then(|target| target.lx.ask.as_mut())
                .is_some_and(AskState::next_step)
                || chosen
        }
        Follow::None => chosen,
    }
}

/// 提交门：全部作答才提交；否则窗格内提示并跳到第一个未答题。
fn submit_or_warn(state: &mut AppState, pane: PaneId) -> bool {
    let Some(ask) = state
        .pane_anywhere(pane)
        .and_then(|target| target.lx.ask.as_ref())
    else {
        return false;
    };
    if ask.all_answered() {
        return submit_all(state, pane);
    }
    let first_unanswered = (0..ask.questions.len()).find(|index| !ask.answered(*index));
    hint(state, pane, ASK_TOAST_ANSWER_REQUIRED);
    if let Some(index) = first_unanswered
        && let Some(ask) = state
            .pane_mut_anywhere(pane)
            .and_then(|target| target.lx.ask.as_mut())
    {
        ask.goto_tab(index);
    }
    true
}

/// 空答案提示：窗格内 toast（不抢全局槽、不写草稿）。
fn hint(state: &mut AppState, pane: PaneId, message: &'static str) {
    crate::app::update::show_lx_toast(
        state,
        pane,
        Toast::new(ToastKind::Warning, message, None, std::time::Instant::now()),
    );
}

/// 本次按键会写入的字节数（只有插入类命令会增长手写缓冲）。
fn inserted_bytes(command: &LxInput) -> Option<usize> {
    match command {
        LxInput::Char(ch) => Some(ch.len_utf8()),
        LxInput::Newline | LxInput::EnterFolder => Some(1),
        _ => None,
    }
}

/// 编辑态按键：编辑子集转发给独立缓冲，`Enter` 提交文本，`Esc` 只退出编辑。
///
/// `Tab`/`Shift+Tab` 与 `↑` 在首个视觉行都会先离开编辑框，再按 `opencode` 的
/// 行为切 tab / 上移高亮；文本非空即提交（空文本视为放弃）。
fn apply_edit_input(state: &mut AppState, pane: PaneId, command: LxInput) -> bool {
    match command {
        LxInput::Enter => commit_edit(state, pane),
        // Shift+Enter / Ctrl+Enter：换行。
        LxInput::Newline | LxInput::EnterFolder => editor_edit(state, pane, command),
        // Tab / Shift+Tab：提交（或丢弃空文本）并切 tab。
        LxInput::Tab => leave_edit(state, pane) | step(state, pane, true),
        LxInput::PrevStep => leave_edit(state, pane) | step(state, pane, false),
        // ↑ 在输入框首个视觉行：离开编辑框并上移高亮；否则交给编辑器移动光标。
        LxInput::Up => {
            let at_top = state
                .pane_anywhere(pane)
                .and_then(|target| target.lx.ask.as_ref())
                .and_then(AskState::editor)
                .is_some_and(|editor| editor.cursor_visual().0 == 0);
            if !at_top {
                return editor_edit(state, pane, command);
            }
            let left = leave_edit(state, pane);
            let moved = state
                .pane_mut_anywhere(pane)
                .and_then(|target| target.lx.ask.as_mut())
                .is_some_and(|ask| ask.move_highlight(-1));
            left | moved
        }
        // Esc：退出编辑，文本保留，不取消整轮。
        LxInput::Escape => state
            .pane_mut_anywhere(pane)
            .and_then(|target| target.lx.ask.as_mut())
            .is_some_and(AskState::cancel_edit),
        command => editor_edit(state, pane, command),
    }
}

/// 把编辑类命令转发给手写缓冲。
fn editor_edit(state: &mut AppState, pane: PaneId, command: LxInput) -> bool {
    state
        .pane_mut_anywhere(pane)
        .and_then(|target| target.lx.ask.as_mut())
        .is_some_and(|ask| {
            ask.editor_mut()
                .is_some_and(|editor| super::super::pane::apply_editor_edit(editor, command))
        })
}

/// 离开编辑框：非空即提交，空文本视为未作答；返回原本是否在编辑。
fn leave_edit(state: &mut AppState, pane: PaneId) -> bool {
    state
        .pane_mut_anywhere(pane)
        .and_then(|target| target.lx.ask.as_mut())
        .is_some_and(|ask| {
            let editing = ask.editing;
            if ask
                .editor()
                .is_some_and(|editor| !editor.text().trim().is_empty())
            {
                ask.commit_edit();
            } else {
                ask.cancel_edit();
            }
            editing
        })
}

/// 切到下一个/上一个 tab（`Tab` 与 `Shift+Tab` 共用）。
fn step(state: &mut AppState, pane: PaneId, forward: bool) -> bool {
    state
        .pane_mut_anywhere(pane)
        .and_then(|target| target.lx.ask.as_mut())
        .is_some_and(|ask| {
            if forward {
                ask.next_step()
            } else {
                ask.prev_step()
            }
        })
}

/// `Enter` 提交手写文本：多选留在本题，单选按 tab 规则前进或提交。
fn commit_edit(state: &mut AppState, pane: PaneId) -> bool {
    let Some(ask) = state
        .pane_anywhere(pane)
        .and_then(|target| target.lx.ask.as_ref())
    else {
        return false;
    };
    let multi = ask.question().is_some_and(|question| question.multi_select);
    let single_only = ask.is_single_select_only();
    let committed = {
        let Some(ask) = state
            .pane_mut_anywhere(pane)
            .and_then(|target| target.lx.ask.as_mut())
        else {
            return false;
        };
        ask.commit_edit()
    };
    if !committed || multi {
        return committed;
    }
    if single_only {
        return submit_all(state, pane) || committed;
    }
    step(state, pane, true) || committed
}

/// 粘贴：仅编辑态写入手写缓冲；超过单题上限**整次拒绝**（绝不截断造假）。
///
/// 列表态吞掉（返回 false）：用户不可见的草稿一个字节都不能动。
pub fn apply_ask_paste(state: &mut AppState, pane: PaneId, text: &str) -> bool {
    let Some(ask) = state
        .pane_mut_anywhere(pane)
        .and_then(|target| target.lx.ask.as_mut())
    else {
        return false;
    };
    if !ask.editing {
        return false;
    }
    let Some(editor) = ask.editor_mut() else {
        return false;
    };
    // 判定与写入同一口径（净化后长度），超限整次拒绝。
    let inserted = editor.insert_str(text);
    if !inserted {
        // 超限整次拒绝：用户不可见的半截答案比没有答案更糟。
        hint(state, pane, ASK_HINT_TOO_LONG);
    }
    inserted
}

/// 面板内鼠标点击（几何由渲染层 `layout()` 提供，事件层不重算）。
pub fn apply_ask_click(state: &mut AppState, pane: PaneId, hit: AskHit) -> bool {
    match hit {
        AskHit::Tab(index) => {
            let left = leave_edit(state, pane);
            let already_on_confirm = state
                .pane_anywhere(pane)
                .and_then(|target| target.lx.ask.as_ref())
                .is_some_and(|ask| ask.is_confirm() && index >= ask.questions.len());
            if already_on_confirm {
                return left | submit_or_warn(state, pane);
            }
            let switched = state
                .pane_mut_anywhere(pane)
                .and_then(|target| target.lx.ask.as_mut())
                .is_some_and(|ask| ask.goto_tab(index));
            left | switched
        }
        AskHit::Option(index) => click_option(state, pane, index),
        AskHit::Other => {
            let Some(ask) = state
                .pane_mut_anywhere(pane)
                .and_then(|target| target.lx.ask.as_mut())
            else {
                return false;
            };
            let moved = ask.set_highlight(ask.other_row());
            let editing = ask.begin_edit();
            moved | editing
        }
    }
}

/// 鼠标点选项：先确认可能存在的行内编辑，再选中；单选立即前进或提交。
fn click_option(state: &mut AppState, pane: PaneId, index: usize) -> bool {
    let left = leave_edit(state, pane);
    let (changed, follow) = {
        let Some(ask) = state
            .pane_mut_anywhere(pane)
            .and_then(|target| target.lx.ask.as_mut())
        else {
            return left;
        };
        let moved = ask.set_highlight(index);
        let chosen = ask.choose(index);
        let multi = ask.question().is_some_and(|question| question.multi_select);
        let single_only = ask.is_single_select_only();
        let follow = if !chosen || multi {
            Follow::None
        } else if single_only {
            Follow::Submit
        } else {
            Follow::Next
        };
        (moved | chosen, follow)
    };
    match follow {
        Follow::Submit => submit_all(state, pane) || changed,
        Follow::Next => step(state, pane, true) || changed,
        Follow::None => changed,
    }
}

/// 面板内的滚轮：选项与角色画预览共用同一个视口，滚轮只滚它、**绝不动高亮**。
///
/// **选项区滚轮绝不动高亮**（与 `@` 提及面板、`/` 命令面板的 `PanelWindow.viewport`
/// 同一语义）：滚轮是"浏览"，移动高亮是"选择"，两者混在一起会让滚到哪就选到哪。
///
/// `window` 是渲染层给出的 `(窗口起点, 总行数, 可视行数)`，事件层不重算几何；
/// 拿不到几何时吞掉滚轮（返回 false），绝不猜一个基准去挪状态。
pub fn apply_ask_wheel(
    state: &mut AppState,
    pane: PaneId,
    _zone: AskWheelZone,
    direction: isize,
    window: Option<(usize, usize, usize)>,
) -> bool {
    let Some(ask) = state
        .pane_mut_anywhere(pane)
        .and_then(|target| target.lx.ask.as_mut())
    else {
        return false;
    };
    let Some((base, total, visible)) = window else {
        return false;
    };
    ask.scroll_viewport(direction.saturating_mul(WHEEL_LINES), base, total, visible)
}

/// 唯一提交入口：全部题目都有答案才组装 `AskAnswers`、清面板并推效应。
///
/// 关面板前把用户作答等待时长补偿进本轮起点：`ask` 期间计时冻结，恢复后从
/// 提问前的已耗秒数继续，最终 `duration_ms` / `tok/s` 不含思考时间。
pub fn submit_all(state: &mut AppState, pane: PaneId) -> bool {
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    let Some(ask) = target.lx.ask.as_ref() else {
        return false;
    };
    if !ask.all_answered() {
        return false;
    }
    let mut answers: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (index, question) in ask.questions.iter().enumerate() {
        answers.insert(question.id.clone(), ask.answers_at(index));
    }
    let key = ask.key.clone();
    let call_id = ask.call_id.clone();
    let Some(ask) = target.lx.ask.take() else {
        return false;
    };
    ask.compensate_turn_clock(&mut target.lx.chat);
    state.effects.push(Effect::AnswerAsk {
        key,
        call_id,
        answers,
    });
    true
}
