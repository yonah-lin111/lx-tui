# 命令面板标题与 @ 面板目录导航 — 设计

- 分支：`feature/prompt-panel-nav`
- 状态：需求边界已确认，实现与测试完成

## 1. 背景与目标

prompt 右栏的两个浮层命令面板（`ui/widgets/command_panel.rs` 渲染）：

- 块命令面板：`#`、`-`、`>` 等块标记触发；
- @ 提及面板：`@` 触发的工作区文件候选。

目标（右栏 prompt 内）：

1. 两个面板顶边左侧都加标题：块命令面板 `Commands`，@ 面板 `Files`。
2. @ 面板支持 Shift+Enter 进入高亮文件夹、Ctrl/Cmd+Z 撤销回退上一级；
   进入后顶边右侧显示当前目录末段名，底边显示两个快捷键。

## 2. 已确认决策（Grill Me 收口）

| # | 决策项 | 结论 |
|---|--------|------|
| 1 | 进入文件夹机制 | 文本驱动：把高亮目录的根相对路径写回 `@` 文本（`@src/main/`），面板按路径前缀限定候选；写回可撤销（`EditKind::Other`） |
| 2 | 进入后列什么 | 空过滤词只列**直接子项**；继续输入在**该子树内递归模糊匹配**；扫描缓存中无后代的目录不可进入（no-op） |
| 3 | 回退语义 | 走编辑器撤销（`Ctrl+Z` / `Cmd+Z`）：进入目录是独立可撤销编辑，撤销一次回到上一级（`@src/ui/` → `@src/` → `@`）；不新增专用回退命令 |
| 4 | 回退物理键 | 无专用键：`Shift+Backspace` 在部分终端不可达，统一用 `Ctrl/Cmd+Z` 撤销回退；`Shift+Delete` 保持普通前删 |
| 5 | 顶左 title | `Commands` / `Files`（英文，与现有 UI 文案一致） |
| 6 | 顶右目录名 | 当前目录末段名（`ui`），回根不显示；不引入文件夹图标 |
| 7 | 底边提示 | `[open ⇧↵] [back ^z]`（`^z` 沿用 `[clear ^c]` 记法），底边左对齐、@ 面板常显、计入面板宽度；块面板无底栏 |
| 8 | 高亮文件时 Shift+Enter | 吞掉 no-op；面板未打开时保持原 `NewlineBelow` |

## 3. 架构与数据流

- `app/markdown.rs`（纯领域）：
  - `split_mention_scope` / `mention_scope`：query 的「目录范围 + 过滤词」拆分；
  - `filter_mentions`：范围前缀限定 + 空过滤词仅直接子项 + 相对路径打分；
  - `MentionPanel::scope_name`：顶边右侧目录末段名。
- `app/prompt/mention.rs`：`MentionState::enter_folder`（返回触发区间与 `@路径/` 写回文本；
  校验目录、缓存后代与区间边界），`refresh` 在目录范围变化时重置高亮。
- `app/prompt.rs`：`mention_enter_folder` 应用文本编辑（独立撤销步）并 `settle`。
- `app/actions.rs` + `input/mod.rs`：新增 `EnterFolder`（Shift+Enter）；回退复用既有 `Undo`。
- `app/update.rs`：面板打开时 `Shift+Enter` 被面板接管（文件/空目录 no-op），面板未打开时回落 `NewlineBelow`。
- `ui/widgets/command_panel.rs`：视图新增 `title` / `right_title` / `footer`，
  顶边左右标题与底边提示参与面板宽度；空间不足由 Block 自然截断。
- `ui/prompt.rs` + `ui/text.rs`：面板视图接线与文案常量；`⇧`/`↵`/`⌫` 纳入单宽非 PUA 兼容测试。

## 4. 边界

- `Shift+Enter` 依赖增强键盘协议；传统终端上报为普通 `Enter` 时自动退化为既有换行行为，无回归。
- 回退即撤销：用户进入目录后若有其他编辑，需连续撤销；这是 `Ctrl/Cmd+Z` 的既有语义。
- 进入目录是文本编辑，撤销后 `settle` 重新计算面板，文本与面板范围始终一致。
