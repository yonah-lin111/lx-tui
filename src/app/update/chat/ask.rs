//! `ask` 面板的按键、鼠标与提交：纯状态迁移，只登记 `Effect`，不碰 IO。
//!
//! 交互对齐 opencode 的提问面板（用户指定）：
//!
//! - **单题**：单选 `Enter` 选中高亮项后**立即提交**；多选 `Space` 勾选、`Enter` 提交。
//! - **多题**：单选 `Enter` 选中并进入下一题；多选 `Space` 勾选、`Enter` 下一题；
//!   末题之后进入 `Confirm` 步，只有全部题目都作答，`Confirm` 上按 `Enter` 才提交。
//! - 当前题还没有答案就按 `Enter`：**窗格内 toast** 提示先作答，不前进、不提交。
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

use super::escape_stop;

/// 回车时当前题还没有答案的 toast 文案。
pub const ASK_HINT_EMPTY: &str = "select an answer first";

/// 预览滚动的单次步长（`PgUp`/`PgDn` 与滚轮共用）。
const PREVIEW_STEP: isize = 3;

/// ask 面板接管 lx 输入；返回是否产生状态变化。
pub fn apply_input(state: &mut AppState, pane: PaneId, command: LxInput) -> bool {
    let Some(ask) = state
        .pane_mut_anywhere(pane)
        .and_then(|target| target.lx.ask.as_mut())
    else {
        return false;
    };
    if ask.editing {
        return apply_edit_input(ask, command);
    }
    match command {
        // ↑↓ / j / k：移动高亮（含「其他」行）。
        LxInput::Up | LxInput::Char('k') => ask.move_highlight(-1),
        LxInput::Down | LxInput::Char('j') => ask.move_highlight(1),
        // Space：勾选/取消（多选）或独占选中（单选）；「其他」行进入编辑态。
        LxInput::Char(' ') => {
            if ask.is_confirm() {
                false
            } else {
                ask.choose(ask.highlight[ask.current])
            }
        }
        // 1–9：直选第 N 项（同 Space 的选择语义；多选再按同一数字取消）。
        LxInput::Char(digit @ '1'..='9') => choose_row(ask, digit),
        // Enter：选中/前进一步/提交流程见 [`enter`]。
        LxInput::Enter => enter(state, pane),
        // Tab/→ 下一步（含 Confirm），← 上一步。
        LxInput::Tab | LxInput::Right => ask.next_step(),
        LxInput::Left => ask.prev_step(),
        LxInput::PageUp => ask.scroll_preview(-PREVIEW_STEP),
        LxInput::PageDown => ask.scroll_preview(PREVIEW_STEP),
        // Esc：沿用双击确认（第一次只待命，第二次取消整轮）。
        LxInput::Escape => escape_stop(state, pane),
        // 其余键吞掉：绝不让它们漏进被隐藏的草稿。
        _ => false,
    }
}

/// `Enter` 的统一入口：先满足当前题的作答条件，再决定提交还是前进。
fn enter(state: &mut AppState, pane: PaneId) -> bool {
    let Some(ask) = state
        .pane_anywhere(pane)
        .and_then(|target| target.lx.ask.as_ref())
    else {
        return false;
    };
    if ask.is_confirm() {
        return if ask.all_answered() {
            submit_all(state, pane)
        } else {
            empty_hint(state, pane)
        };
    }
    let (answered, single) = {
        let Some(ask) = state
            .pane_mut_anywhere(pane)
            .and_then(|target| target.lx.ask.as_mut())
        else {
            return false;
        };
        let Some(question) = ask.question() else {
            return false;
        };
        let highlight = ask.highlight[ask.current];
        let on_other = highlight == question.options.len();
        let other_committed = ask.custom_on.get(ask.current).copied().unwrap_or(false);
        // 高亮「其他」且尚未提交手写文本：回车进入编辑（再次回车才视为作答）。
        if on_other && !other_committed {
            return ask.begin_edit();
        }
        // 单选：回车即选中高亮项（opencode 语义）；多选靠 Space 勾选。
        if !on_other && !question.multi_select {
            ask.choose(highlight);
        }
        (
            !ask.answers_at(ask.current).is_empty(),
            ask.questions.len() == 1,
        )
    };
    if !answered {
        return empty_hint(state, pane);
    }
    if single {
        return submit_all(state, pane);
    }
    state
        .pane_mut_anywhere(pane)
        .and_then(|target| target.lx.ask.as_mut())
        .is_some_and(AskState::next_step)
}

/// 空答案提示：窗格内 toast（不抢全局槽、不写草稿）。
fn empty_hint(state: &mut AppState, pane: PaneId) -> bool {
    crate::app::update::show_lx_toast(
        state,
        pane,
        Toast::new(
            ToastKind::Warning,
            ASK_HINT_EMPTY,
            None,
            std::time::Instant::now(),
        ),
    );
    true
}

/// 编辑态按键：编辑子集转发给独立缓冲，`Enter` 提交文本，`Esc` 只退出编辑。
fn apply_edit_input(ask: &mut AskState, command: LxInput) -> bool {
    match command {
        // Enter：提交手写文本为本题答案并退出编辑（不提交整题）。
        LxInput::Enter => ask.commit_edit(),
        // Shift+Enter / Ctrl+Enter：换行。
        LxInput::Newline | LxInput::EnterFolder => ask.editor_mut().is_some_and(|editor| {
            editor.newline();
            true
        }),
        // Esc：退出编辑，文本保留，不取消整轮。
        LxInput::Escape => ask.cancel_edit(),
        // Tab：编辑态吞掉（不切题，避免误操作）。
        LxInput::Tab => false,
        command => ask
            .editor_mut()
            .is_some_and(|editor| super::super::pane::apply_editor_edit(editor, command)),
    }
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
    let limit = crate::chat::tools::ask::MAX_ASK_CUSTOM_BYTES;
    if editor.text().len().saturating_add(text.len()) > limit {
        return false;
    }
    editor.insert_str(text);
    true
}

/// 面板内鼠标点击（几何由渲染层 `layout()` 提供，事件层不重算）。
pub fn apply_ask_click(state: &mut AppState, pane: PaneId, hit: AskHit) -> bool {
    if hit == AskHit::Submit {
        // Submit / Confirm：单题直接提交；多题的 Confirm 只有全部作答才放行。
        let (confirm, on_other, ready) = {
            let Some(ask) = state
                .pane_anywhere(pane)
                .and_then(|target| target.lx.ask.as_ref())
            else {
                return false;
            };
            let on_other = ask
                .question()
                .is_some_and(|question| ask.highlight[ask.current] == question.options.len())
                && !ask.custom_on.get(ask.current).copied().unwrap_or(false);
            (ask.is_confirm(), on_other, ask.all_answered())
        };
        if !confirm && on_other {
            return state
                .pane_mut_anywhere(pane)
                .and_then(|target| target.lx.ask.as_mut())
                .is_some_and(AskState::begin_edit);
        }
        return if ready {
            submit_all(state, pane)
        } else {
            empty_hint(state, pane)
        };
    }
    let Some(ask) = state
        .pane_mut_anywhere(pane)
        .and_then(|target| target.lx.ask.as_mut())
    else {
        return false;
    };
    match hit {
        // 选项行：高亮该项（切换预览）并选中；**绝不提交**（Enter/Submit 才是提交入口）。
        //
        // 点击列表即离开编辑态：用户已经从手写框点回列表，不再算「正在输入」。
        AskHit::Option(index) => {
            let moved = ask.set_highlight(index);
            let left_edit = std::mem::replace(&mut ask.editing, false);
            moved | left_edit | ask.click_option(index)
        }
        // 「其他」行：进入编辑态，光标落到文本末尾。
        AskHit::Other => ask.begin_edit(),
        // 题目标签（含 Confirm）：切 step，状态保留。
        AskHit::Tab(index) => ask.goto_tab(index),
        AskHit::Submit => false,
    }
}

/// 面板内的滚轮：预览区只滚预览，选项区移动高亮。
pub fn apply_ask_wheel(
    state: &mut AppState,
    pane: PaneId,
    zone: AskWheelZone,
    direction: isize,
) -> bool {
    let Some(ask) = state
        .pane_mut_anywhere(pane)
        .and_then(|target| target.lx.ask.as_mut())
    else {
        return false;
    };
    match zone {
        AskWheelZone::Preview => ask.scroll_preview(direction.saturating_mul(PREVIEW_STEP)),
        AskWheelZone::Options => ask.move_highlight(direction),
    }
}

/// 唯一提交入口：全部题目都有答案才组装 `AskAnswers`、清面板并推效应。
pub fn submit_all(state: &mut AppState, pane: PaneId) -> bool {
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    let Some(ask) = target.lx.ask.as_mut() else {
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
    target.lx.ask = None;
    state.effects.push(Effect::AnswerAsk {
        key,
        call_id,
        answers,
    });
    true
}

/// 数字键直选：`1`–`9` 映射到第 N 行；越界吞掉。
fn choose_row(ask: &mut AskState, digit: char) -> bool {
    let index = usize::from(digit as u8 - b'1');
    if index > ask.other_row() {
        return false;
    }
    ask.choose(index)
}
