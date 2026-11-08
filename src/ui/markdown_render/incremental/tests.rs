//! 增量 Markdown 与全量渲染的逐行等价性回归。

use crate::ui::markdown_render::{IncrementalMarkdownState, MdLine, render_wrapped_code_blocks};

/// 逐步喂入每个字节边界，断言 `prefix + tail` 始终等于全量渲染。
fn assert_equivalent(source: &str, width: u16) {
    let mut state = IncrementalMarkdownState::default();
    let mut boundaries: Vec<usize> = source.char_indices().map(|(index, _)| index).collect();
    boundaries.push(source.len());
    for end in boundaries {
        let piece = &source[..end];
        let (prefix, _, tail) = state.update(piece, width);
        let mut got: Vec<MdLine> = (*prefix).clone();
        got.extend(tail);
        assert_eq!(
            got,
            render_wrapped_code_blocks(piece, width),
            "width={width} 前缀在第 {end} 字节处与全量渲染不一致"
        );
    }
}

/// 含全部块级语法的样张。
const COMPLEX: &str = "# 标题\n\
\n\
段落带 **粗体**、*斜体*、`行内代码`、~~删除线~~ 与 [链接](https://example.com)。\n\
\n\
- 顶层一\n\
  - 嵌套二\n\
    - 嵌套三\n\
- [x] 已完成\n\
- [ ] 未完成\n\
\n\
1. 有序一\n\
2. 有序二\n\
\n\
> 引用第一行\n\
> 引用第二行\n\
\n\
![图片](https://example.com/a.png)\n\
\n\
| 名称 | 数量 |\n\
| :--- | ---: |\n\
| 甲 | 1 |\n\
| 乙 | 22 |\n\
\n\
---\n\
\n\
```rust\n\
fn main() {\n\
    // 注释\n\
    let value = 42;\n\
\n\
    println!(\"{value}\");\n\
}\n\
```\n\
\n\
    缩进代码一\n\
    缩进代码二\n\
\n\
收尾段落。\n";

#[test]
fn incremental_matches_full_render_at_widths() {
    for width in [8u16, 20, 45, 96] {
        assert_equivalent(COMPLEX, width);
    }
}

/// 流式中的未闭合围栏、表格与缩进代码块。
#[test]
fn incremental_matches_full_render_while_blocks_are_open() {
    let open = "开头\n\n```rust\nfn a() {\n    let x = 1;\n";
    assert_equivalent(open, 24);
    assert_equivalent("表格前\n\n| a | b |\n| --- | --- |\n| 1 | 2 |\n| 3", 20);
    assert_equivalent("代码前\n\n    第一行\n\n    第二行\n", 30);
}

/// 超长围栏代码块逐行追加：稳定前缀必须持续增长，尾部只留未完成行。
#[test]
fn long_fence_commits_lines_as_they_complete() {
    let mut state = IncrementalMarkdownState::default();
    let mut source = String::from("```rust\n");
    let mut last_prefix_rows = 0usize;
    for line in 0..120 {
        source.push_str(&format!("let value_{line} = {line};\n"));
        let (prefix, _, tail) = state.update(&source, 40);
        assert!(prefix.len() >= last_prefix_rows, "稳定前缀不得回退");
        last_prefix_rows = prefix.len();
        assert!(tail.len() <= 2, "未闭合代码块只应留极短尾部：{tail:?}");
    }
    assert!(last_prefix_rows >= 120, "换行后的代码行应立即固化");
}

/// 内容被整体重写（rewind / 会话切换）时按前缀指纹重置，不沿用旧行。
#[test]
fn rewritten_source_resets_the_prefix() {
    let mut state = IncrementalMarkdownState::default();
    state.update("第一版内容\n第二行\n", 30);
    let (prefix, _, tail) = state.update("完全不同的内容\n", 30);
    let mut got = (*prefix).clone();
    got.extend(tail);
    assert_eq!(got, render_wrapped_code_blocks("完全不同的内容\n", 30));
}

/// 宽度变化必须全量重排（折行依赖宽度）。
#[test]
fn width_change_reflows_everything() {
    let source = "一段足够长的中英文 mixed 文本，用来验证折行宽度变化。\n";
    let mut state = IncrementalMarkdownState::default();
    state.update(source, 60);
    let (prefix, _, tail) = state.update(source, 20);
    let mut got = (*prefix).clone();
    got.extend(tail);
    assert_eq!(got, render_wrapped_code_blocks(source, 20));
}

/// 确定性伪随机块序列：覆盖标题/列表/引用/围栏/表格/缩进代码/空行的交错组合。
#[test]
fn randomized_block_sequences_match_full_render() {
    const PIECES: &[&str] = &[
        "# 标题",
        "## 二级标题",
        "普通段落 with **粗体** and `code`",
        "- 列表项",
        "  - 嵌套项",
        "1. 有序项",
        "- [x] 任务",
        "> 引用",
        "---",
        "```rust",
        "```",
        "    let x = 1;",
        "| a | b |",
        "| --- | --- |",
        "| 1 | 2 |",
        "    缩进代码",
        "",
    ];
    // 线性同余发生器：固定种子保证可复现。
    let mut seed = 0x9e37_79b9_7f4a_7c15u64;
    for case in 0..6 {
        let mut source = String::new();
        for _ in 0..120 {
            seed = seed
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            source.push_str(PIECES[(seed >> 33) as usize % PIECES.len()]);
            source.push('\n');
        }
        let width = if case % 2 == 0 { 40 } else { 17 };
        let mut state = IncrementalMarkdownState::default();
        let mut checked = 0;
        for end in (0..=source.len()).step_by(7) {
            if !source.is_char_boundary(end) {
                continue;
            }
            let piece = &source[..end];
            let (prefix, _, tail) = state.update(piece, width);
            let mut got = (*prefix).clone();
            got.extend(tail);
            assert_eq!(
                got,
                render_wrapped_code_blocks(piece, width),
                "随机块序列 case={case} width={width} 前缀在第 {end} 字节处不一致"
            );
            checked += 1;
        }
        assert!(checked > 10, "必须覆盖足够多的流式前缀");
    }
}
