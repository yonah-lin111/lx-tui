//! 历史滚动性能回归（`#[ignore]`，只在 release 下手动跑）。
//!
//! 验收口径（`docs/feature/lx-history-perf/design.md` §0）：1000 条消息量级、约 10 万
//! 渲染行、200×50 视口；滚轮/拖滚动条时单帧 `layout + render` 的 P95 ≤ 16ms。
//!
//! ```
//! cargo test --release -- --ignored perf_lx_history
//! ```

use std::time::{Duration, Instant};

use ratatui::layout::Rect;

use super::*;
use crate::app::state::{
    CardSection, CommandView, DiffHunkView, DiffLineView, FileChangeView, FileDiffDetailView,
    FileDiffView, ProcessStateView, ToolCallView, ToolDetailView, ToolStatus, ToolSummary,
};
use crate::chat::tools::{DiffLineKind, FileChangeKind};

/// 每条正文块的行数；乘以消息数用来逼近 10 万渲染行。
const BODY_LINES: usize = 70;
/// 单帧预算（60fps）。
const FRAME_BUDGET: Duration = Duration::from_millis(16);
/// 采样的帧数。
const FRAMES: usize = 120;
/// 每条 assistant 消息里的工具调用数（`read` + `exec_command` 各一次）。
const TOOLS_PER_MESSAGE: usize = 2;

/// 一段带中英文与标点的正文（逼真的折行与 markdown 解析成本）。
fn body(tag: usize) -> String {
    (0..BODY_LINES)
        .map(|line| {
            format!("第 {tag} 条消息第 {line} 行：mixed 中英文 content，含标点与数字 123。")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// 完成态的 `read` 卡片：两行摘要，走 `BlockPaint::Legacy`（每次绘制重新排版）。
fn read_step(position: i64) -> AssistantStepView {
    AssistantStepView::Tool {
        position,
        call: ToolCallView {
            call_id: format!("call-read-{position}"),
            name: "read".into(),
            position,
            arguments_json: String::new(),
            status: ToolStatus::Completed,
            summary: Some(Box::new(ToolSummary {
                path: Some("src/ui/main_content/lx/history/paint.rs".into()),
                range: Some((12, 40)),
                read_lines: Some(29),
                ..ToolSummary::default()
            })),
            error: None,
            result_json: None,
            detail: None,
        },
    }
}

/// 完成态的 `exec_command` 卡片：参数区 + 折叠输出预览（同样走 Legacy）。
fn command_step(position: i64) -> AssistantStepView {
    let output_tail: String = (0..12)
        .map(|line| format!("test result line {line}: ok"))
        .collect::<Vec<_>>()
        .join("\n");
    AssistantStepView::Tool {
        position,
        call: ToolCallView {
            call_id: format!("call-command-{position}"),
            name: "exec_command".into(),
            position,
            arguments_json: String::new(),
            status: ToolStatus::Completed,
            summary: Some(Box::new(ToolSummary::default())),
            error: None,
            result_json: None,
            detail: Some(Box::new(ToolDetailView::Command(CommandView {
                process_session_id: None,
                command: "cargo test --lib history".into(),
                cwd: "/repo".into(),
                tty: false,
                duration_ms: None,
                state: ProcessStateView::Exited { code: Some(0) },
                output_preview: Vec::new(),
                truncated: false,
                dropped_bytes: 0,
                output_tail,
                started_at_ms: None,
                ended_at_ms: None,
            }))),
        },
    }
}

/// 构造压力会话：用户/助手交替；助手带一段 thinking 与两个工具调用。
///
/// 工具卡片是**唯一没进二级内容缓存**的块（`BlockPaint::Legacy`），
/// 只压满正文与思考量出来的 P95 不能代表真实会话，所以这里必须带上它们。
fn history(messages: usize) -> Lx {
    let mut out = Vec::with_capacity(messages);
    for index in 0..messages {
        let role = if index % 2 == 0 { "user" } else { "assistant" };
        let mut parts = vec![text_part(&body(index))];
        if role == "assistant" {
            parts.push(reasoning_part("先看看现有实现的排版口径"));
        }
        let mut card = message(role, "complete", parts);
        if role == "assistant" {
            for offset in 0..TOOLS_PER_MESSAGE {
                let position = card.next_position();
                card.steps.push(if offset == 0 {
                    read_step(position)
                } else {
                    command_step(position)
                });
            }
        }
        out.push(card);
    }
    lx_with(out)
}

/// 第 `percent` 百分位（采样先排序）。
fn percentile(samples: &mut [Duration], percent: usize) -> Duration {
    samples.sort_unstable();
    let index = samples.len().saturating_sub(1) * percent / 100;
    samples[index]
}

/// 热缓存下的连续滚动：帧内只应做「按视口挑块 + 画视口内的行」。
#[test]
#[ignore = "性能回归：cargo test --release -- --ignored perf_lx_history"]
fn perf_lx_history() {
    let mut lx = history(1400);
    let area = Rect::new(0, 0, 200, 50);
    let mut cache = PaneCache::default();
    let mut grid = Grid::new(area);

    let cold = Instant::now();
    let total = layout(area, &lx, &mut cache).total_rows;
    let cold_elapsed = cold.elapsed();
    assert!(total > 90_000, "压力会话应有约 10 万行，实际 {total}");

    // 预热：先画一帧，让视口周边的块进缓存。
    let _ = render(&mut grid, area, &lx, 0, &mut cache);
    let max_top = total.saturating_sub(usize::from(area.height)).max(1);

    let mut scroll_samples = Vec::with_capacity(FRAMES);
    for frame in 0..FRAMES {
        cache.advance_frame();
        let scroll = (frame * 20) % max_top;
        let started = Instant::now();
        let _ = layout(area, &lx, &mut cache);
        let _ = render(&mut grid, area, &lx, scroll, &mut cache);
        scroll_samples.push(started.elapsed());
    }

    // 流式追加：每帧给最后一条消息补一行，只有该块重排。
    let last = lx.chat.messages.len().saturating_sub(1);
    let mut stream_samples = Vec::with_capacity(FRAMES);
    for _ in 0..FRAMES {
        if let Some(AssistantStepView::Part { part, .. }) =
            lx.chat.messages.get_mut(last).and_then(|message| {
                message
                    .steps
                    .iter_mut()
                    .find(|step| matches!(step, AssistantStepView::Part { .. }))
            })
        {
            part.content.push_str("\n流式追加的一行");
        }
        cache.advance_frame();
        let started = Instant::now();
        let _ = layout(area, &lx, &mut cache);
        let _ = render(&mut grid, area, &lx, max_top, &mut cache);
        stream_samples.push(started.elapsed());
    }

    let scroll_p95 = percentile(&mut scroll_samples, 95);
    let stream_p95 = percentile(&mut stream_samples, 95);
    let bytes = cache.bytes();

    eprintln!(
        "perf_lx_history: total_rows={total} cold={cold_elapsed:?} scroll_p95={scroll_p95:?} \
         stream_p95={stream_p95:?} cache_bytes={bytes}"
    );

    assert!(
        scroll_p95 <= FRAME_BUDGET,
        "热缓存滚动 P95 超预算：{scroll_p95:?} > {FRAME_BUDGET:?}"
    );
    assert!(
        stream_p95 <= FRAME_BUDGET,
        "流式追加 P95 超预算：{stream_p95:?} > {FRAME_BUDGET:?}"
    );
    assert!(
        bytes <= crate::ui::main_content::lx::history::cache::MAX_CACHED_BYTES,
        "二级缓存超上限：{bytes} 字节"
    );
}

/// 展开的大文件差异：2000 行，滚动其内部时的单帧成本。
///
/// 工具卡片仍走 `BlockPaint::Legacy`（行数已缓存、正文每次绘制重建）；命令输出
/// 展开上限 24 行（`DETAIL_MAX_ROWS`），所以最坏场景是展开的差异卡片。这条用例
/// 量的是「工具卡片尚未接入二级内容缓存」还差多少。
#[test]
#[ignore = "性能回归：cargo test --release -- --ignored perf_lx_history"]
fn perf_lx_history_expanded_diff() {
    let hunks: Vec<DiffHunkView> = (0..100)
        .map(|hunk| DiffHunkView {
            lines: (0..20)
                .map(|line| DiffLineView {
                    kind: DiffLineKind::Context,
                    old_line: Some(hunk * 20 + line),
                    new_line: Some(hunk * 20 + line),
                    text: format!("context line {hunk}-{line} with some source code text"),
                    no_final_newline: false,
                })
                .collect(),
        })
        .collect();
    let mut card = message("assistant", "complete", vec![text_part("改一下文件")]);
    let mut call = ToolCallView::new("call-diff", "edit_file", 0);
    call.status = ToolStatus::Completed;
    call.summary = Some(Box::new(ToolSummary::default()));
    call.detail = Some(Box::new(ToolDetailView::File(FileDiffDetailView {
        files: vec![FileChangeView {
            path: "src/big.rs".into(),
            to: None,
            kind: FileChangeKind::Updated,
            added: Some(2000),
            removed: Some(0),
            counts_known: true,
            simplified: false,
            diff: Some(FileDiffView::Complete {
                hunks,
                simplified: false,
            }),
        }],
        files_total: 1,
        added: Some(2000),
        removed: Some(0),
        simplified: false,
        omitted_files: 0,
        single: false,
    })));
    card.steps.push(AssistantStepView::Tool {
        position: card.next_position(),
        call,
    });
    let (id, slot) = (card.id.clone(), "call-diff".to_string());
    let mut lx = lx_with(vec![
        message("user", "complete", vec![text_part("开始")]),
        card,
    ]);
    // 展开文件差异卡（整卡展开，section = Body）。
    lx.chat
        .detail_expanded
        .push((id, slot, CardSection::Body, true));

    let area = Rect::new(0, 0, 200, 50);
    let mut cache = PaneCache::default();
    let mut grid = Grid::new(area);
    let total = layout(area, &lx, &mut cache).total_rows;
    assert!(total > 2000, "展开的差异应占数千行，实际 {total}");

    let mut samples = Vec::with_capacity(FRAMES);
    for frame in 0..FRAMES {
        cache.advance_frame();
        let max_top = total.saturating_sub(usize::from(area.height)).max(1);
        let scroll = (frame * 20) % max_top;
        let started = Instant::now();
        let _ = layout(area, &lx, &mut cache);
        let _ = render(&mut grid, area, &lx, scroll, &mut cache);
        samples.push(started.elapsed());
    }
    let p95 = percentile(&mut samples, 95);
    eprintln!("perf_lx_history_expanded_diff: total_rows={total} p95={p95:?}");
    assert!(
        p95 <= FRAME_BUDGET,
        "展开的大差异滚动 P95 超预算：{p95:?} > {FRAME_BUDGET:?}"
    );
}
