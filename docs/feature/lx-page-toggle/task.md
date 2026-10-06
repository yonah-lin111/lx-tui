# lx 页与终端双视图切换 — 执行任务

- 分支：`feat/lx-page-toggle`；worktree：`.worktrees/lx-page-toggle`；基线：dev@71f0024
- 规则：全部命令在 worktree 根目录执行；生产代码禁止 `unwrap()/expect()`；用户可见文案只进 `ui/text.rs`；颜色只从 `ui/style.rs` 取。

## 执行顺序

### 0. 状态与行为（app 层）

- [ ] `src/app/state.rs`：新增 `PaneView`、`Pane.view`（默认 `Lx`）、`AppState.lx_phase` / `lx_last_tick`、`AppState::lx_visible()`
- [ ] `src/app/update.rs`：`toggle_pane_view`、`tick_lx_animation`、`tick` / `next_deadline` 接入
- [ ] 测试：`src/app/state/tests.rs`、`src/app/update/tests.rs`

### 1. 输入与事件（input / event 层）

- [ ] `src/input/mod.rs`：`route` 增加 `pane_lx` 参数并吞键（`Ctrl+Q` 仍优先）
- [ ] `src/input/tests.rs`：补吞键用例
- [ ] `src/event/mod.rs`：
  - [ ] 按键/粘贴：活动窗格 `Lx` 时吞掉
  - [ ] 左键：按钮命中优先（`toggle_button_at`，过滤 `PaneKind::Terminal`），点击不改变焦点
  - [ ] 内容区：`Lx` 视图仅聚焦 + 清选区，不选区/不转发/无滚动条；滚轮忽略
  - [ ] `forward_pane_mouse_event`、`handle_pane_wheel` 增加视图门控
  - [ ] 滚动条几何改调 `ui::main_content::scrollbar(inner, pane)`
- [ ] `src/event/tests.rs`：按钮切换、吞键、鼠标禁用、鼠标上报模式不转发

### 2. UI：重命名与分派（main content）

- [ ] `git mv src/ui/terminal.rs src/ui/main_content.rs`；`git mv src/ui/terminal src/ui/main_content`
- [ ] `src/ui/main_content.rs`：`render(area, buf, pane, focused, lx_phase)` 分派 / `scrollbar(inner, pane)` / `toggle_button` 几何 / `toggle_button_at` / `toggle_label` / `draw_toggle_button`
- [ ] `src/ui/mod.rs`：模块声明改 `main_content`；`render_panes` 标题按视图选择、按钮在 Block 后覆盖绘制
- [ ] 迁移并补测试：`src/ui/main_content/tests.rs`

### 3. UI：lx 页与像素狐狸

- [ ] `src/ui/main_content/lx.rs`（私有子模块）：
  - [ ] 像素艺术数据（base + tail/ears/eyes 覆盖层，含紧凑狐狸头）
  - [ ] `compose()` 纯函数与相位→帧选择
  - [ ] 半块渲染（`▀/▄/█`，仅写艺术区域）
  - [ ] 布局与降级（全尺寸 → 紧凑头 → 纯文本 `lx`）、水平/垂直居中
- [ ] `src/ui/style.rs`：mascot 调色板语义函数（Indexed，仅 lx 页使用）
- [ ] `src/ui/text.rs`：`LX_TITLE` / `LX_TOGGLE_TERMINAL` / `LX_TOGGLE_LX` / `lx_hint()`
- [ ] 测试：`src/ui/main_content/lx/tests.rs`、`src/ui/style/tests.rs`、`src/ui/text/tests.rs`
- [ ] `src/ui/tests.rs`：默认标题 ` lx `、按钮标签随视图切换、窄窗格隐藏、切回终端标题恢复

### 4. 规范文档同步

- [ ] `docs/standards/tui-design-requirements.md`：main content 双视图交互、标题规则、mascot 调色板例外、动画例外
- [ ] `docs/standards/project-directory-structure.md`：`ui/terminal.rs` → `ui/main_content.rs` 条目

### 5. 全量验证

```bash
cargo fmt
cargo clippy --all-targets -- -D warnings
cargo test
```

- [ ] 全绿（改默认视图后，既有交互测试需显式 `PaneView::Terminal`，逐条修复而不是放宽断言）
- [ ] 用户运行 `cargo run` 目视确认：狐狸动画、按钮切换、标题、终端行为回归

### 6. 提交

- [ ] 提交信息（仓库风格）：`feat(main-content): 窗格双视图（lx 页/终端）与顶栏切换按钮`
- [ ] 提交内容包含：代码 + 测试 + 本目录 design/task 文档 + 规范更新
- [ ] 提交后询问用户是否合并回 `dev`（未确认前不合并、不删除 worktree）

## 布局修订（第二轮，Claude Code 式）

- [x] lx 页改为「顶部品牌区（完整狐狸 18×7 + 字标）→ 白色占位面板 → 底部输入框（纯视觉占位）」垂直结构
- [x] 狐狸按完整尺寸显示；宽度不足以画完整狐狸时省略品牌区，不画残缺狐狸
- [x] 定位为纯展示欢迎页；未来 agent 消息内容切换为不含狐狸的内容视图
- [x] 输入框 `>` + `Ask anything…`；占位面板白色边框 + 居中 `placeholder`
- [x] 降级：提示 → 面板 → 品牌区自下而上省略，最小退化为居中 `lx`
- [x] 测试：艺术一致性、相位、输入框几何、四档布局规模、动画帧差异

## 测试矩阵

| 层 | 用例 |
|----|------|
| state | 新窗格默认 `Lx`；`lx_visible`（全终端为 false） |
| update | `toggle_pane_view` 只翻目标窗格；切 Lx 清理该窗格选区/滚动拖拽；`tick` 可见时推进、不可见冻结；`next_deadline` 可见时含帧期限 |
| input | `pane_lx=true` 吞普通键；`Ctrl+Q` 仍退出；prompt 聚焦优先 |
| event | 点按钮翻转且焦点不变；lx 视图按键/粘贴不写 PTY；lx 内容区点击仅聚焦；鼠标上报模式在 lx 不转发；滚轮忽略；按钮空间不足不命中 |
| ui/main_content | Terminal 路径渲染与游标不变；Lx 路径无游标；`scrollbar` 在 Lx 返回 None；按钮几何/隐藏/标签 |
| ui/lx | `compose` 覆盖层；相位→帧（0/6/10 等）；调色板字符→Indexed；TestBackend 渲染非空且含提示；紧凑/纯文本降级 |
| ui | 标题两态；按钮标签两态；切回终端标题与内容恢复 |

## 风险与回滚

- 默认视图变更面广：所有依赖“默认终端”的测试必须逐条显式设视图；生产路径由 `Palette` 无改动、PTY 会话装配不依赖视图，风险集中在测试面。
- 动画只在 lx 可见时 200ms tick，事件循环空闲仍可唤醒；若发现耗电/闪烁，可把 `LX_FRAME_INTERVAL` 调大或先冻结动画，不影响数据模型。
- 256 色 Indexed 在 16 色终端会由终端降级为近似色；不影响布局与其它区域。
- 回滚：整分支撤回即可，无数据迁移、无外部副作用。
