# lx 页与终端双视图切换 — 设计

- 分支：`feat/lx-page-toggle`（worktree：`.worktrees/lx-page-toggle`，基线 dev@71f0024）
- 状态：待用户审核确认后执行

## 1. 背景与目标

现状：每个窗格（Pane）固定渲染真实 PTY 的终端网格，标题为 OSC → cwd → `pane N`；全局 prompt 右栏独立存在。

目标：把窗格内容区升级为 **main content**，容纳两个互斥视图：

- `Lx`：品牌欢迎页（像素狐狸吉祥物 + `lx` 字标 + 切换提示），默认视图；
- `Terminal`：现有终端网格，行为完全不变。

窗格顶边框右端提供纯鼠标切换按钮，PTY 在 lx 视图下继续后台运行，切回后画面、回滚缓冲、选区状态基线不受影响。

## 2. 已确认决策（盘问结论）

| # | 决策 | 结论 |
|---|------|------|
| 1 | 概念重命名 | terminal 概念/代码改名为 **main content**，文件命名遵循 Rust snake_case；不是改用户可见文案 |
| 2 | lx 页内容 | 像素狐狸（动画）+ `lx` 字标 + 一行切换提示；不伪造版本/模型/tips 数据 |
| 3 | 切换作用域 | 每个 Pane 独立持有视图状态；切到 lx 后 PTY 继续运行，切回不丢画面与滚动位置 |
| 4 | 默认视图 | 启动、新建窗格/标签/工作区默认 **Lx** |
| 5 | lx 视图键盘 | 除 `Ctrl+Q` 外全部按键**吞掉**，不写入后台 PTY |
| 6 | 按钮 | 终端视图显示 `[lx]`、lx 视图显示 `[>_]`（显示目的地）；窗格顶边框右端；纯鼠标；空间不足隐藏 |
| 7 | lx 视图鼠标 | 内容区仅保留“点击聚焦”；选区、鼠标转发、滚轮、滚动条全部禁用 |
| 8 | 配色 | 256 色 Indexed 还原 logo（粉/浅蓝/藏青），色板收敛在 `ui/style.rs`，仅 lx 页使用 |
| 9 | 动画 | 摆尾 + 眨眼 + 抖耳；200ms/帧，仅 lx 页可见时推进（不可见时冻结、零重绘） |
| 10 | 规范同步 | 最小增补 `tui-design-requirements.md` 与 `project-directory-structure.md` |

## 3. 数据模型与状态

### 3.1 `src/app/state.rs`

```rust
/// 窗格主内容视图。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaneView {
    Lx,       // lx 欢迎页（默认）
    Terminal, // 终端网格
}
```

- `Pane` 增加字段 `pub view: PaneView`；`Pane::new()` 置 `PaneView::Lx`。
- `AppState` 增加动画状态：
  - `pub lx_phase: u64`：动画相位（tick 计数）；
  - `pub lx_last_tick: Instant`：上次推进时刻；
- 新增方法：

```rust
/// 当前标签是否存在 lx 视图窗格（决定动画是否推进）。
pub fn lx_visible(&self) -> bool
```

### 3.2 动画时序常量（`src/app/update.rs`）

- `LX_FRAME_INTERVAL = 200ms`；
- 周期 24 帧（4.8s）：
  - 摆尾：`(lx_phase / 2) % 2`（400ms 一换向）；
  - 眨眼：`lx_phase % 12 ∈ {6,7}`（2.4s 一次）；
  - 抖耳：`lx_phase % 24 ∈ {10,11}`（4.8s 一次）。

### 3.3 `src/app/update.rs`

```rust
/// 切换窗格主内容视图；返回是否变化。
///
/// 切到 Lx 时清掉该窗格上的终端选区与滚动条拖拽（隐藏终端不接受交互）。
pub fn toggle_pane_view(state: &mut AppState, pane: PaneId) -> bool

/// 推进 lx 动画：仅当 lx_visible()，按 200ms 步进 lx_phase；返回是否变化。
fn tick_lx_animation(state: &mut AppState, now: Instant) -> bool
```

- `tick()` 在 toast / 自动滚动之后追加动画推进，返回值合并；
- `next_deadline()` 在 lx 可见时追加 `lx_last_tick + 200ms`，事件循环据此定时唤醒；lx 不可见时不登记、不唤醒、不重绘。

## 4. 交互规格

### 4.1 切换按钮

- 几何：窗格顶边框行（`rect.y`），右端距右角 1 列，宽 = 标签字符数（`[lx]` / `[>_]` 同为 4）；
- 命中：`main_content::toggle_button_at(rects, col, row) -> Option<PaneId>`，仅 `PaneKind::Terminal` 的窗格参与（geometry.rects 含 prompt 栏与占位窗格，必须过滤）；
- 渲染：在 Block（含标题）之后覆盖绘制，样式 `style::accent()`；
- 隐藏：`rect.width < 标签宽 + 3` 时不渲染、不命中；
- 点击：只切换视图，**不改变焦点**（对齐折叠按钮“控件”语义）；
- 仅鼠标：不增加任何键盘快捷键（lx 视图吞键已确认；悬浮块命令等保持不变）。

### 4.2 标题

窗格标题（Block title，左上）：

- `Lx`：` lx `；
- `Terminal`：` {pane_display_title} `（保持 OSC → cwd → `pane N` 原逻辑）；
- 两种视图下 `pane.exited` 都追加 ` (exited)`（终端已死的事实不能被 lx 页藏掉）；
- 焦点色规则不变（focus 且非 prompt 聚焦时 accent，否则 muted）。

### 4.3 键盘与粘贴

- `input::route` 增加 `pane_lx: bool` 参数，判定顺序：Release → `Ctrl+Q` → 浮层 → prompt 聚焦 → **pane_lx 吞掉** → 编码进窗格；
- 事件循环在非 prompt 聚焦、无浮层、活动窗格为 Lx 时把按键交给 `route`（由 route 统一吞掉，不在事件层散落键位判断）；
- `Paste` 事件：prompt 聚焦优先；否则活动窗格为 Lx 时直接吞掉，不写 PTY。

### 4.4 鼠标

- 按钮命中优先于内容区；
- lx 视图窗格内容区：左键按下仅 `focus_pane` + 清 prompt 选区；**不**启动终端选区、**不**转发鼠标上报、**不**显示/命中滚动条；滚轮忽略；
- `forward_pane_mouse_event` 增加视图门控（该窗格 view 为 Lx 时直接返回 false），一次覆盖 Down/Drag/Up/Moved 转发路径；
- `handle_pane_wheel` 的窗格分支增加 view 门控；
- 终端滚动条几何（`main_content::scrollbar`）在 Lx 视图返回 None，覆盖拖拽与命中；
- 硬件光标：Lx 视图不调用终端渲染、不返回光标位置，终端原生光标不显示。

## 5. 渲染规格

### 5.1 模块结构（重命名 + 新增）

```text
src/ui/
  main_content.rs          # 由 ui/terminal.rs 重命名：分派 Terminal/Lx + 终端网格渲染 + 按钮几何/绘制
  main_content/
    tests.rs               # 原 ui/terminal/tests.rs 迁移 + 分派/按钮测试
    lx.rs                  # 私有子模块：lx 页渲染、像素画、调色板映射、动画帧
    lx/tests.rs            # lx 页测试
```

- `ui/mod.rs`：`pub mod terminal;` → `pub mod main_content;`；`render_panes` 改为：

```rust
match pane.kind {
    PaneKind::Terminal => {
        // 标题按 view 选择；随后
        main_content::render(inner, frame.buffer_mut(), pane, focused, state.lx_phase)
        main_content::scrollbar(inner, pane)
        main_content::draw_toggle_button(...)  // 仅 kind == Terminal 且几何允许
    }
    PaneKind::Placeholder => {}
}
```

- `event/mod.rs`：两处 `ui::terminal::scrollbar(...)` 改为 `ui::main_content::scrollbar(inner, pane)`。

### 5.2 半块像素画算法

- 艺术数据为“像素矩阵 + 调色板字符”，每字符 1 像素；
- 每 2 个纵向像素渲染进 1 个终端单元格：
  - 上像素有、下像素无：`▀`（fg=上色，默认 bg）；
  - 上无、下有：`▄`（fg=下色，默认 bg）；
  - 上下同色：`█`（fg=色）；
  - 不同色：`▀`（fg=上色，bg=下色）；
  - 全透明：不写单元格；
- 只在艺术区域写单元格，不铺整屏背景，符合“不铺自绘背景”的既有规范精神。

### 5.3 调色板（`src/ui/style.rs` 新增语义函数，仅 lx 页使用）

| 语义函数 | 字符 | Indexed | 近似色 | 用途 |
|----------|------|---------|--------|------|
| `mascot_outline()` | `k` | 17 | `#00005f` | 描边（logo 藏青） |
| `mascot_fur()` | `p` | 218 | `#ffafd7` | 主体粉 |
| `mascot_fur_light()` | `P` | 224 | `#ffd7d7` | 高光浅粉 |
| `mascot_shadow()` | `n` | 168 | `#d75f87` | 暗部深粉 |
| `mascot_accent()` | `b` | 117 | `#87d7ff` | 天空蓝（内耳/胸） |
| `mascot_accent_deep()` | `B` | 74 | `#5fafd7` | 深一档蓝 |
| `mascot_highlight()` | `w` | 231 | `#ffffff` | 眼白/高光 |

### 5.4 lx 页布局与降级

从上到下居中堆叠：狐狸艺术 → 空行 → `lx` 字标（`style::strong()`）→ 空行 → 提示（`style::muted()`，`text::lx_hint()`＝“click [>_] to open terminal”）。

降级规则（`inner` 为窗格内容区）：

1. `width ≥ FOX_W + 2` 且 `height ≥ FOX_ROWS + 4`：全尺寸狐狸 + 字标；宽度再满足提示长度 + 4 才画提示；
2. 否则 `width ≥ HEAD_W + 2` 且 `height ≥ HEAD_ROWS + 2`：紧凑狐狸头（简化像素图）+ 字标；
3. 否则：仅居中 `lx` 文本。

尺寸常量在实现时按最终像素图落定（预估全尺寸 16 列 × 14 像素＝16×7 单元格；紧凑头 10×8 像素＝10×4 单元格）。

### 5.5 动画叠加

- 艺术数据分层：`BASE`（耳/头/身/腿）+ `TAIL`（左/右 2 帧覆盖层）+ `EARS`（常态/抖动 2 帧）+ `EYES`（睁/闭 2 帧）；覆盖层同尺寸，`.` 表示保留底层；
- 合成顺序：base → tail → ears → eyes；`compose()` 为纯函数，便于单测；
- 相位 → 帧（周期 24，见 3.2）：
  - tail = `(phase / 2) % 2`；
  - eyes = `matches!(phase % 12, 6 | 7)`；
  - ears = `matches!(phase % 24, 10 | 11)`。

### 5.6 用户可见文案（`src/ui/text.rs`）

```rust
pub const LX_TITLE: &str = "lx";
pub const LX_TOGGLE_TERMINAL: &str = "[>_]"; // lx 视图按钮（去终端）
pub const LX_TOGGLE_LX: &str = "[lx]";       // 终端视图按钮（去 lx）
pub fn lx_hint() -> String                    // "click [>_] to open terminal"
```

## 6. 受影响的现有行为与测试

- 默认视图改为 Lx 后，所有“按键进 PTY / 点击出选区 / 滚轮滚动窗格”的既有测试都需要显式把目标窗格置为 `PaneView::Terminal`（新增测试辅助函数），或在断言前切换；不改生产语义。
- `ui::terminal` 模块路径消失，相关 import 与文档引用同步更新。
- 工作区自动命名仍跟随根窗格 cwd（PTY 继续运行），不受视图影响。

## 7. 规范文档更新（最小增补）

`docs/standards/tui-design-requirements.md`：

- 主区域职责：补充 main content 双视图、默认 lx、切换按钮、标题规则、lx 视图键鼠规则；
- 配色与样式：补充 mascot Indexed 调色板例外（仅 lx 页）；
- “不使用动画”条款：补充“lx 页吉祥物 200ms 帧动画”例外。

`docs/standards/project-directory-structure.md`：

- `ui/terminal.rs` 条目改为 `ui/main_content.rs`（主内容分派：lx 页/终端网格，含 lx 子模块）；模块职责第 3 条同步。

## 8. Non-goals

- 不做键盘切换快捷键；
- 不做 lx 页的可配置开关/主题；
- 不为 Placeholder 窗格渲染按钮或 lx 页（无生产路径）；
- 不引入图片、字体或动画库依赖；
- 不改 prompt 右栏、侧栏、标签栏的既有行为。
