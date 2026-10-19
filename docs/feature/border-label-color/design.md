# 边框标签值颜色：聚焦不再变蓝 — 设计方案

> 状态：已按 grill-me 逐条确认边界后实施。
>
> 任务范围：`src/ui/` 渲染层的边框标签取色。不改布局、不改命中、不改前缀。

## 1. 背景与问题

lx 页有三类"工作区身份"标签出现在边框上：

| 位置 | 标签 | 现状 |
| --- | --- | --- |
| lx 窗格顶边右侧（切换按钮左隔 2 列） | ` ws:末两段` | 前缀 `accent()`；值 `border_value(focused)` |
| 终端窗格顶边右侧（同上位置） | ` agent:kimi` | 前缀 `accent()`；值 `border_value(focused)` |
| prompt 底边左侧 | ` b:分支`、` wt:工作区名` | 前缀 `accent()`；值 `muted()` |
| prompt 底边右侧 | `ws:路径末段名` | 前缀 `accent()`；值 `muted()` |

用户报告：这些标签的**值**在聚焦时会变成蓝色，要求移除这层聚焦着色，保留原本的默认颜色。

两条路径的成因不同：

1. 窗格顶边：`render_border_right_label` 先 `cell.reset()` 再写入，值样式完全由 `style::border_value(focused)` 决定 —— 聚焦时 `accent().add_modifier(DIM)` = `Cyan + BOLD + DIM`，这就是"变蓝"。
2. prompt 底边：值用 `style::muted()` 写入，而 `muted()` 只设 `add_modifier(DIM)`、**不设前景**。该文字直接叠在边框单元格上，于是把边框的 `Cyan + BOLD` 一并继承 —— prompt 面板聚焦时底边框是 `accent()`，值就跟着变蓝。

## 2. 已确认边界（grill-me）

| 决策 | 口径 |
| --- | --- |
| 窗格 `ws:` 值 | 聚焦与失焦**完全一致**：恒为默认灰。焦点仍只由边框框线颜色区分。 |
| 窗格 `agent:` 值 | 一并改（用户追加确认）。理由同上：值不随焦点变色。 |
| 前缀 | `ws:` / `agent:` / `b:` / `wt:` **保持 `accent()` 不变**。失焦时前缀随边框继承 DIM 的现有行为也保持不变。 |
| 值的外观 | 保留 dim 一档：显式 `Color::Reset` 前景 + `DIM`，并显式去掉继承的 `BOLD`。 |
| 范围 | 只这四处值。prompt 顶边框标题、浮层边框标题、侧栏标记等一律不动。 |
| 文档 | 同步更新 `tui-design-requirements.md` 与 `workspace-agent-status/design.md` 中"聚焦时值改用强调色高亮"的描述。 |

## 3. 实现

### 3.1 新增样式 token

`style::border_value(focused)` 只有窗格 `ws:` 与 `agent:` 两个调用方，且两者都要改成"恒定默认灰"，因此该函数没有保留价值 —— 删除它，改为无参、语义明确的 token。

```rust
/// 边框上的标签值（窗格 `ws:`/`agent:`、prompt `b:`/`wt:`/`ws:` 的值）：
/// 显式默认前景 + dim，并显式移除边框继承来的 BOLD，不随面板焦点变色。
pub fn border_label_value() -> Style {
    Style::default()
        .fg(Color::Reset)
        .add_modifier(Modifier::DIM)
        .remove_modifier(Modifier::BOLD)
}
```

为什么必须显式 `fg(Color::Reset)` 与 `remove_modifier(BOLD)`：边框上的文字是叠加写入的，`set_style` 只覆盖样式里**给出**的字段（见 `ratatui_core::buffer::Cell::set_style`）。只用 `muted()` 会继承聚焦边框的青色与加粗。

### 3.2 调用点

- `ui/panes.rs`：`render_pane_workspace_label` 与 `render_pane_agent_label` 的值 span 改用 `border_label_value()`；两处 `focused` 参数不再参与值取色，随之删除该临时变量（`render_panes` 里的 `focused` 仍继续用于边框与标题）。
- `ui/prompt_frame.rs`：`render_workspace_label` 与 `branch_status_spans` 的值改用 `border_label_value()`。

## 4. 不变量

1. 值的样式与 `focused` / `state.prompt_focused` 完全无关。
2. 前缀样式统一为 `border_label_prefix()`（`Cyan + DIM`，显式去 BOLD）。
3. 文本内容、宽度计算、截断与隐藏规则不变：本次只换样式，不改布局，因此 `truncate_to_width` / `ellipsize` / 可见宽度判定都不受影响。

## 4.1 第二轮：前缀与页名也不再随焦点变化

第一轮只处理了四条**值**，前缀仍是 `accent()`；实测（`TestBackend` 单元格样式）暴露了同一类问题的其余两处：

| 单元格 | 第一轮后失焦 | 第一轮后聚焦 | 期望 |
| --- | --- | --- | --- |
| prompt `b:` 前缀 | `Cyan + BOLD + DIM` | `Cyan + BOLD` | `Cyan + DIM` |
| 窗格 `ws:` / `agent:` 前缀 | `Cyan + BOLD` | `Cyan + BOLD` | `Cyan + DIM` |
| prompt 页名 `· Page 1` | `Reset + DIM` | `Reset + BOLD + DIM` | `Reset + DIM` |

根因仍是"样式只写部分字段 → 其余从边框继承"：前缀用 `accent()` 写死了 BOLD，页名用 `muted_on_border()` 没去 BOLD 而继承了聚焦边框的粗体。

本轮决策（grill-me 逐条确认）：

| 决策 | 口径 |
| --- | --- |
| 前缀 | 五处（prompt `b:`/`wt:`/`ws:`、窗格 `ws:`/`agent:`）共用新 token `border_label_prefix()` = `Cyan + DIM` + 显式去 BOLD。 |
| 页名 | `muted_on_border()` 增加 `.remove_modifier(BOLD)`，一处修好全部 5 个调用点（页名、`· ` 重命名分隔符、引用菜单/搜索底注、`[UI]` 查看器底注）。 |
| 亮度 | 取方案甲：前缀两个状态都是这档淡青，聚焦不再变亮变粗。 |

### 4.2 第二轮实现要点

- `style::border_label_prefix()`：前缀统一 token；与 `border_title()` 同色系（都是 `Cyan + DIM`、都不加粗）。
- `style::muted_on_border()`：补 `.remove_modifier(BOLD)`。该 token 的设计意图本就是"叠加在强调色边框上的灰字不被染色"，漏了 BOLD 一项；5 个调用点全画在 `accent()` 边框上，同病同治。
- `ui/panes.rs`、`ui/prompt_frame.rs`：五处前缀从 `style::accent()` 改为 `style::border_label_prefix()`。
- 页名本身不改绘制代码，由 token 修复即可生效。


## 5. 验证

`src/ui/panes.rs` 与 `src/ui/prompt_frame.rs` 无独立测试文件，渲染断言集中在 `src/ui/tests.rs`。本次在 `src/ui/tests.rs` 覆盖：

1. lx 窗格 `ws:` 值：聚焦与失焦渲染结果逐字节相同（`Reset + DIM`，无 `BOLD`，非 `Cyan`）。
2. 终端窗格 `agent:` 值：同上，聚焦/失焦一致。
3. prompt 底边 `b:main`、`wt:work`、`ws:末段` 的值：面板聚焦（底边框为 `accent()`）时仍是 `Reset + DIM`，且不带 `BOLD`。
4. 前缀回归：`b:` / `wt:` / `ws:` / `agent:` 首字符仍是 `Cyan`。
5. `style::border_label_value()` 的 token 单测，锁定"显式 Reset 前景 + DIM + 无 BOLD"。

原有断言旧行为的 3 个测试同步更新断言，不降低覆盖面。

### 5.1 第二轮新增/更新

- 新增 `ui::tests::prompt_bottom_prefix_ignores_focus`：prompt 底边框 `b:` 前缀在失焦/聚焦都必须是 `Cyan + DIM` 且逐字段一致。
- 新增 `ui::tests::prompt_page_name_ignores_focus`：页名在失焦/聚焦都必须是 `Reset + DIM`（不带 BOLD）。
- 扩展 `lx_pane_ws_value_ignores_focus` / `pane_agent_value_ignores_focus`：追加前缀断言，并断言聚焦/失焦前缀样式一致。
- 新增 `ui::style::tests::border_label_prefix_is_soft_cyan_without_bold`、`muted_on_border_strips_inherited_bold`：token 级断言"叠加到 `accent()` 单元格后不带 BOLD"。
- 更新既有 2 处前缀断言（`prompt_title_moves_path_name_to_bottom_right`、`pane_border_shows_agent_label_separated_from_toggle_button`）。

