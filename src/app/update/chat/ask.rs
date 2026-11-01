//! `ask` 面板的按键、鼠标与提交：纯状态迁移，只登记 `Effect`，不碰 IO。
//!
//! 交互对齐 opencode 的提问面板（design §4.3）：
//!
//! - **列表态只认 `↑`/`↓`**：切高亮（含「其他」与底部提交按钮），没有 `j`/`k`、
//!   `Tab`、`←`/`→`、`1..9`——多题切题只靠鼠标点 tab 或单选答完自动前进。
//! - **单选**：在选项上按 `Enter` 立即选中并完成本题（单题提交、多题进下一题、
//!   末题把焦点移到提交按钮）。
//! - **多选 / 多题末题**：选项只勾选，必须聚焦底部 ` ↵ submit ` 按 `Enter`
//!   （或鼠标点击按钮）才提交。
//! - **提交按钮**：未答完时给窗格内 toast，并跳到第一个未作答的题目。
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

/// 预览滚动的单次步长（`PgUp`/`PgDn` 与滚轮共用）。
const PREVIEW_STEP: isize = WHEEL_LINES;

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
        // 列表导航只剩 ↑↓：不再有 j/k、Tab、←/→、1..9。
        LxInput::Up => ask.move_highlight(-1),
        LxInput::Down => ask.move_highlight(1),
        // Space：选项勾选、「其他」进入行内编辑、提交行等价 Enter。
        LxInput::Char(' ') => space(state, pane),
        // Enter：选中/前进/提交，规则见 [`enter`]。
        LxInput::Enter => enter(state, pane),
        LxInput::PageUp => ask.scroll_preview(-PREVIEW_STEP),
        LxInput::PageDown => ask.scroll_preview(PREVIEW_STEP),
        // Esc：沿用双击确认（第一次只待命，第二次取消整轮）。
        LxInput::Escape => escape_stop(state, pane),
        // 其余键吞掉：绝不让它们漏进被隐藏的草稿。
        _ => false,
    }
}

/// `Space`：普通选项勾选/独占选择，「其他」进入编辑，提交行等价 Enter。
fn space(state: &mut AppState, pane: PaneId) -> bool {
    let Some(ask) = state
        .pane_anywhere(pane)
        .and_then(|target| target.lx.ask.as_ref())
    else {
        return false;
    };
    let highlight = ask.highlight.get(ask.current).copied().unwrap_or(0);
    if Some(highlight) == ask.submit_row() {
        return enter(state, pane);
    }
    let on_other = highlight == ask.other_row();
    let Some(ask) = state
        .pane_mut_anywhere(pane)
        .and_then(|target| target.lx.ask.as_mut())
    else {
        return false;
    };
    if on_other {
        return ask.begin_edit();
    }
    ask.choose(highlight)
}

/// `Enter` 的统一入口：提交按钮 > 其他 > 普通选项。
fn enter(state: &mut AppState, pane: PaneId) -> bool {
    let Some(ask) = state
        .pane_anywhere(pane)
        .and_then(|target| target.lx.ask.as_ref())
    else {
        return false;
    };
    let highlight = ask.highlight.get(ask.current).copied().unwrap_or(0);
    // 焦点在提交按钮：全部作答才提交，否则提示并跳到第一个未答题。
    if Some(highlight) == ask.submit_row() {
        let ready = ask.all_answered();
        let first_unanswered = (0..ask.questions.len()).find(|index| !ask.answered(*index));
        if ready {
            return submit_all(state, pane);
        }
        hint(state, pane, ASK_TOAST_ANSWER_REQUIRED);
        if let Some(index) = first_unanswered
            && let Some(ask) = state
                .pane_mut_anywhere(pane)
                .and_then(|target| target.lx.ask.as_mut())
        {
            ask.goto_tab(index);
        }
        return true;
    }
    // 焦点在「其他」：原地进入行内编辑（再次回车才确认文本）。
    if highlight == ask.other_row() {
        return state
            .pane_mut_anywhere(pane)
            .and_then(|target| target.lx.ask.as_mut())
            .is_some_and(AskState::begin_edit);
    }
    let multi = ask.question().is_some_and(|question| question.multi_select);
    let single = ask.questions.len() == 1;
    let has_submit = ask.has_submit_row();
    let Some(ask) = state
        .pane_mut_anywhere(pane)
        .and_then(|target| target.lx.ask.as_mut())
    else {
        return false;
    };
    // 单选：回车即选中高亮项；多选：只切换勾选，绝不自动提交。
    if !ask.choose(highlight) {
        return false;
    }
    if multi {
        return true;
    }
    if has_submit {
        ask.focus_submit();
        return true;
    }
    if !single {
        return ask.next_step();
    }
    submit_all(state, pane)
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
fn apply_edit_input(state: &mut AppState, pane: PaneId, command: LxInput) -> bool {
    match command {
        LxInput::Enter => commit_edit(state, pane),
        // Shift+Enter / Ctrl+Enter：换行。
        LxInput::Newline | LxInput::EnterFolder => state
            .pane_mut_anywhere(pane)
            .and_then(|target| target.lx.ask.as_mut())
            .is_some_and(|ask| {
                ask.editor_mut().is_some_and(|editor| {
                    editor.newline();
                    true
                })
            }),
        // Esc：退出编辑，文本保留，不取消整轮。
        LxInput::Escape => state
            .pane_mut_anywhere(pane)
            .and_then(|target| target.lx.ask.as_mut())
            .is_some_and(AskState::cancel_edit),
        // Tab：编辑态吞掉（不切题，避免误操作）。
        LxInput::Tab => false,
        command => state
            .pane_mut_anywhere(pane)
            .and_then(|target| target.lx.ask.as_mut())
            .is_some_and(|ask| {
                ask.editor_mut()
                    .is_some_and(|editor| super::super::pane::apply_editor_edit(editor, command))
            }),
    }
}

/// `Enter` 提交手写文本：非空且单选时完成本题（提交或前进），多选/末题退出编辑并勾选。
fn commit_edit(state: &mut AppState, pane: PaneId) -> bool {
    let Some(ask) = state
        .pane_anywhere(pane)
        .and_then(|target| target.lx.ask.as_ref())
    else {
        return false;
    };
    let multi = ask.question().is_some_and(|question| question.multi_select);
    let single = ask.questions.len() == 1;
    let has_submit = ask.has_submit_row();
    let mut submit = false;
    let changed = {
        let Some(ask) = state
            .pane_mut_anywhere(pane)
            .and_then(|target| target.lx.ask.as_mut())
        else {
            return false;
        };
        let committed = ask.commit_edit();
        if committed && !multi {
            if has_submit {
                ask.focus_submit();
            } else if single {
                submit = true;
            } else {
                ask.next_step();
            }
        }
        committed
    };
    if submit {
        return submit_all(state, pane);
    }
    changed
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
            let Some(ask) = state
                .pane_mut_anywhere(pane)
                .and_then(|target| target.lx.ask.as_mut())
            else {
                return false;
            };
            let left_edit = ask.editing && ask.commit_edit();
            let switched = ask.goto_tab(index);
            left_edit | switched
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
        AskHit::Submit => submit_hit(state, pane),
    }
}

/// 鼠标点选项：先确认可能存在的行内编辑，再选中；单选无提交按钮时立即收尾。
fn click_option(state: &mut AppState, pane: PaneId, index: usize) -> bool {
    let (changed, follow) = {
        let Some(ask) = state
            .pane_mut_anywhere(pane)
            .and_then(|target| target.lx.ask.as_mut())
        else {
            return false;
        };
        let left_edit = ask.editing && ask.commit_edit();
        let moved = ask.set_highlight(index);
        let chosen = ask.choose(index);
        let multi = ask.question().is_some_and(|question| question.multi_select);
        let has_submit = ask.has_submit_row();
        let single = ask.questions.len() == 1;
        let follow = if chosen && !multi && !has_submit {
            if single { Follow::Submit } else { Follow::Next }
        } else {
            Follow::None
        };
        (left_edit | moved | chosen, follow)
    };
    match follow {
        Follow::Submit => submit_all(state, pane),
        Follow::Next => {
            let advanced = state
                .pane_mut_anywhere(pane)
                .and_then(|target| target.lx.ask.as_mut())
                .is_some_and(AskState::next_step);
            changed | advanced
        }
        Follow::None => changed,
    }
}

/// 鼠标点提交按钮：先确认行内编辑，再按「全部作答」决定提交或提示跳题。
fn submit_hit(state: &mut AppState, pane: PaneId) -> bool {
    let committed = state
        .pane_mut_anywhere(pane)
        .and_then(|target| target.lx.ask.as_mut())
        .is_some_and(|ask| ask.editing && ask.commit_edit());
    let Some(ask) = state
        .pane_anywhere(pane)
        .and_then(|target| target.lx.ask.as_ref())
    else {
        return committed;
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

/// 面板内的滚轮：预览区只滚预览，选项区只滚列表视口。
///
/// **选项区滚轮绝不动高亮**（与 `@` 提及面板、`/` 命令面板的 `PanelWindow.viewport`
/// 同一语义）：滚轮是"浏览"，移动高亮是"选择"，两者混在一起会让滚到哪就选到哪。
///
/// `window` 是渲染层给出的 `(窗口起点, 总行数, 可视行数)`，事件层不重算几何；
/// 拿不到几何时吞掉滚轮（返回 false），绝不猜一个基准去挪状态。
pub fn apply_ask_wheel(
    state: &mut AppState,
    pane: PaneId,
    zone: AskWheelZone,
    direction: isize,
    window: Option<(usize, usize, usize)>,
) -> bool {
    let Some(ask) = state
        .pane_mut_anywhere(pane)
        .and_then(|target| target.lx.ask.as_mut())
    else {
        return false;
    };
    match zone {
        AskWheelZone::Preview => ask.scroll_preview(direction.saturating_mul(WHEEL_LINES)),
        AskWheelZone::Options => {
            let Some((base, total, visible)) = window else {
                return false;
            };
            ask.scroll_viewport(direction.saturating_mul(WHEEL_LINES), base, total, visible)
        }
    }
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
