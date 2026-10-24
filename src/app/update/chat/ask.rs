//! `ask` 面板的按键、鼠标与提交：纯状态迁移，只登记 `Effect`，不碰 IO。
//!
//! 键位契约见 design §7.1（列表态）与 §7.2（编辑态）。两条硬约束：
//!
//! 1. **草稿不变量**：这里任何路径都不读、不写 `Lx.input` / `Lx.cursor` / 撤销栈；
//!    手写文本只进 `AskEditor`。
//! 2. **单一提交入口**：只有 [`submit_current`] 会在末题组装 `AskAnswers` 并清面板；
//!    迟到的重复点击因此天然幂等（`lx.ask` 已经是 `None`）。

use std::collections::BTreeMap;

use crate::app::actions::LxInput;
use crate::app::effects::Effect;
use crate::app::state::lx::{AskHit, AskWheelZone};
use crate::app::state::{AppState, AskState};
use crate::layout::PaneId;

use super::escape_stop;

/// 空答案提示（内联在面板底栏，不占用 toast）。
pub const ASK_HINT_EMPTY: &str = "select at least one option";

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
        // Space：高亮行进入选择；「其他」行进入编辑态。
        LxInput::Char(' ') => ask.choose(ask.highlight[ask.current]),
        // 1–9：直选第 N 项（同 Space 的选择语义；多选再按同一数字取消）。
        LxInput::Char(digit @ '1'..='9') => choose_row(ask, digit),
        // Enter：高亮「其他」进入编辑，否则提交当前题。
        LxInput::Enter => {
            if ask.highlight[ask.current] == ask.other_row() {
                ask.begin_edit()
            } else {
                submit_current(state, pane)
            }
        }
        // Tab/→ 下一题，← 上一题（保留各题状态）。
        LxInput::Tab | LxInput::Right => ask.next_question(),
        LxInput::Left => ask.prev_question(),
        LxInput::PageUp => ask.scroll_preview(-PREVIEW_STEP),
        LxInput::PageDown => ask.scroll_preview(PREVIEW_STEP),
        // Esc：沿用双击确认（第一次只待命，第二次取消整轮）。
        LxInput::Escape => escape_stop(state, pane),
        // 其余键吞掉：绝不让它们漏进被隐藏的草稿。
        _ => false,
    }
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

/// 面板内的鼠标点击（几何由渲染层 `layout()` 提供，事件层不重算）。
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

pub fn apply_ask_click(state: &mut AppState, pane: PaneId, hit: AskHit) -> bool {
    if hit == AskHit::Submit {
        // Submit 等价 Enter：高亮「其他」时进编辑，否则提交当前题。
        let on_other = state
            .pane_anywhere(pane)
            .and_then(|target| target.lx.ask.as_ref())
            .is_some_and(|ask| ask.highlight[ask.current] == ask.other_row());
        return if on_other {
            state
                .pane_mut_anywhere(pane)
                .and_then(|target| target.lx.ask.as_mut())
                .is_some_and(AskState::begin_edit)
        } else {
            submit_current(state, pane)
        };
    }
    let Some(ask) = state
        .pane_mut_anywhere(pane)
        .and_then(|target| target.lx.ask.as_mut())
    else {
        return false;
    };
    match hit {
        // 选项行：高亮该项（切换预览）并选中；**绝不提交**（Submit 是唯一提交入口）。
        //
        // 点击列表即离开编辑态：用户已经从手写框点回列表，不再算「正在输入」，
        // 否则下一次 Submit 会拿空的手写答案判定为空。
        AskHit::Option(index) => {
            let moved = ask.set_highlight(index);
            let left_edit = std::mem::replace(&mut ask.editing, false);
            moved | left_edit | ask.click_option(index)
        }
        // 「其他」行：进入编辑态，光标落到文本末尾。
        AskHit::Other => ask.begin_edit(),
        // 题目标签：切题，状态保留。
        AskHit::Tab(index) => ask.goto_question(index),
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

/// 提交当前题：空答案只设内联提示；末题则组装答案、清面板并推效应。
pub fn submit_current(state: &mut AppState, pane: PaneId) -> bool {
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    let Some(ask) = target.lx.ask.as_mut() else {
        return false;
    };
    let answers = ask.current_answers();
    if answers.is_empty() {
        ask.hint = Some(ASK_HINT_EMPTY.to_string());
        return true;
    }
    let current = ask.current;
    ask.collected[current] = answers;
    ask.hint = None;
    if !ask.is_last_question() {
        return ask.next_question();
    }
    // 末题：逐题汇总。已提交的题沿用 `collected`，从未提交过的题按当前状态兜底。
    let mut collected: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (index, question) in ask.questions.iter().enumerate() {
        let answers = if ask.collected[index].is_empty() {
            ask.answers_at(index)
        } else {
            ask.collected[index].clone()
        };
        collected.insert(question.id.clone(), answers);
    }
    let key = ask.key.clone();
    let call_id = ask.call_id.clone();
    target.lx.ask = None;
    state.effects.push(Effect::AnswerAsk {
        key,
        call_id,
        answers: collected,
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
