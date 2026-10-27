# TUI 统一输入快捷键与全文本选择实施任务

> 状态：实施中（阶段 0-2 完成；阶段 3-4 首版完成；待用户 `cargo run` 验收）。
> 说明：稳定 `TextSource` 全组件映射尚未落地，当前屏幕选区用“冻结画面 + 内容行匹配”重定位；
> 其余已确认行为（松开不自动复制、选区保留、统一快捷键、异步粘贴/确认剪切）已实现并有测试。

## 阶段 0：锁定现状与测试入口

- [ ] 对照 `design.md` 的 13 条已确认边界，更新 `docs/standards/tui-design-requirements.md` 的选择/复制与按键说明。保持项目规范为唯一最终规则来源。
- [ ] 为现有“松开即复制并清选区”行为增加/改写事件层回归用例，使新要求先有失败测试；保留终端鼠标上报、弹窗遮挡、控件命中等既有保护测试。
- [ ] 复查每个将触及的源文件是否接近生产文件 1000 行上限；现有 `event/mouse/press.rs` 约 684 行，选择逻辑应单独成模块。

## 阶段 1：共用输入编辑模型

- [ ] 在 `src/app/overlay.rs` 的 `TextInput` 增加文本选区、快照撤销/重做、全选/替换/剪切/按词移动与删除。坐标以字符边界为准，转换到字节偏移时保持 UTF-8 安全；复制范围不包含光标外文本。
- [ ] `src/app/state/todo.rs` 继续复用 `TextInput`；`src/app/update/todo.rs` 编辑态 Ctrl+Z/Y 进入文字历史，列表态 Ctrl+Z 保留删除撤销。一次粘贴、剪切、选区替换各是一条撤销记录。
- [ ] `src/app/state/lx/ask.rs` 继续包装 lx 编辑核心，接入全选与剪切、复制后留选区；保留答案字节上限、净化口径与原草稿不变。
- [ ] prompt/lx 多行编辑器补齐共享快捷键缺口，不能破坏面板状态、提及、路径粘贴提案和既有 undo 事务边界。
- [ ] 页重命名、`OverlayKind::{Rename,NewWorkspace,NewNode,ResumeSessionEdit,WorktreeOpen,SaveDraft,ReferencedFolderInput,ReferencedFolderSearch}` 等逐项接入相同的编辑语义和长度限制；非输入型 overlay 不获得编辑命令。

## 阶段 2：键盘与粘贴

- [ ] `src/input/mod.rs` 统一 `Ctrl+Shift+A`/`Cmd+A` 全选、`Ctrl+A/E` 行首/行尾、`Mod+C/X/V/Z/Shift+Z`、`Ctrl+Y`、按词移动/删除与 Home/End 的解析；保留 Ctrl+Q 和终端无选区 Ctrl+C。删除 prompt 全文复制、模板块 Shift+C 的焦点特殊路径；lx 无选区 Ctrl+C 恢复为清空当前输入。显式复制按钮/菜单不受影响。
- [ ] `src/event/input.rs` 在焦点分流前处理非空全局选区复制；无选区时按已确认规则路由。可编辑选区剪切走“原生剪贴板结果确认 → 校验编辑器修订与选区 → 删除”的异步事务；写入失败、状态变化或只有 OSC 52 时不删除。
- [ ] 把 Paste 事件的所有可编辑目标收束到同一入口，补上所有浮层、页重命名和搜索输入。列表态或非编辑态吞掉，不能写隐藏输入/PTY。
- [ ] `src/platform/` 增加 OS 剪贴板读接口与可确认结果的原生写接口；按键形式的 Mod+V 异步执行并回投。记录目标实例与修订号，焦点变化时丢弃结果；按有界短等待窗口合并近邻 Paste 事件，不声称终端协议没有事务 ID 时可绝对去重。
- [ ] 对输入路由、净化与最大长度、todo 撤销模式、剪切失败、异步粘贴竞态分别写精确单元测试。

## 阶段 3：屏幕选择模型

- [ ] 新增 `ui` 侧最终画面 `ScreenText` 取样/源映射：宽字符首格与续格、可见填充空格、边框、遮挡层和屏幕坐标一致。只读取已渲染 Buffer，不在绘制中执行状态更新或 IO。
- [ ] 在 `app/state`、`app/update/selection` 建立单一 `ScreenSelection` 生命周期，逐步替换 `selection`、`terminal_selection`、`lx_selection` 等并行状态。避免迁移中一个动作只清掉部分旧选区。
- [ ] 给终端内容、prompt 文本、lx 历史/输入、todo、ask、弹窗和静态区域提供稳定 `TextSource`。先覆盖当前画面；滚动/缩放后按同源格重定位高亮，源失效则清除。
- [ ] 提取冻结 `SelectedCells` 为纯文本，按屏幕视觉行添加换行，保留边框和行内空格；宽字符只输出一次，剔除不可见续格与控制序列。复制之后不改变选择状态。
- [ ] 给点击计数（同格 400 ms）、Unicode 词/逻辑行、拖动反向扩选、可见行数、跨区域、resize/scroll 与源删除写纯数据测试。

关键实现顺序：先在 `ui::render_cached` 的最终可见 Buffer 上取得只读 `ScreenText`，再把鼠标端点映射为 `TextSource`，最后替换旧的三套选区槽。不要先删除 `terminal::Terminal` 的选区能力：终端绝对行锚点和鼠标模式门控需要先有回归测试。截图/复制的冻结 payload 应在选择范围完成或改变时更新，而不是在每个 loading 帧重新序列化历史。

## 阶段 4：鼠标接线与绘制

- [ ] 在 `src/event/mouse/press.rs`、`drag.rs` 维持原有控件命中顺序；新增最后一层文字命中。Shift+拖动在终端鼠标上报模式下抢占，普通拖动仍转发 PTY。
- [ ] 双击 todo/prompt 行的**文字格**选词；Enter/工具栏继续编辑，非文字区域的原操作保留。三击逻辑行；其他已有双击控件按控件优先规则处理。
- [ ] 鼠标松开只结束选择，不写剪贴板、不清选区；lx 卡片点击只有“未形成文字选择”的点击才可折叠。
- [ ] 在 `src/ui/mod.rs` 的最终可见层完成后叠加反显高亮；弹窗与 toast 的可见文字可选，不显示被遮住底层的高亮。确保选区样式不改变文字和布局。
- [ ] 增加渲染 Buffer 测试：侧栏至主区、弹窗边框、长字符、软换行、隐藏底层、copy 后高亮仍在、滚动后滚回重新高亮。

## 阶段 5：精确验证与用户验收

- [ ] 对修改的 Rust 文件执行 `cargo fmt`；检查格式差异只涉及本任务。
- [ ] 对受影响测试模块运行精确筛选：`input::tests`、`app::selection::tests`、`app::state::lx::tests`、`app::update::todo::tests`、`event::tests` 的新增选区/复制场景、`ui::tests` 与相关 prompt/terminal 用例。若 Cargo 测试过滤不方便，按受影响测试二进制/模块执行。
- [ ] 按项目规范运行 `cargo clippy --all-targets -- -D warnings`；共享输入/全局事件触及面广，最终运行 `cargo test` 作为本任务集成验证，不扩展到无关工具链。
- [ ] 用户运行 `cargo run` 检查实际终端：Cmd 是否被终端拦截、Shift+拖动 PTY、宽字符/跨区域视觉高亮、粘贴去重、鼠标操作优先级。Agent 不声称已交互验证 TUI。
- [ ] 核对文档行为表与代码、测试一致；只报告本任务产生的文件差异，不修改用户已有 Git 暂存或其他工作区状态。

建议的精确命令（按实际测试模块名调整过滤器）：

```sh
cargo fmt --check
cargo test input::tests
cargo test app::update::todo::tests
cargo test app::update::tests::selection
cargo test event::tests::mouse
cargo test ui::tests::selection
cargo clippy --all-targets -- -D warnings
cargo test
```

其中不存在的过滤器不能被误报为“通过”：先用 `cargo test -- --list` 核对新增测试名，再运行对应过滤器。`cargo fmt` 本身会改动文件，因此最终只在实现阶段执行，文档审核阶段不运行格式化。全量 `cargo test` 是项目级规则针对共享输入/渲染修改的最终门槛，不以它替代阶段中的定向验证。

## 审核点

请先审核 `design.md` 的 §1 已确认边界、§3 稳定锚点方案、§5 快捷键表，以及本文件的实施阶段。用户明确确认后才进入阶段 0 的代码与规范修改。

## 实施记录（自动验证）

- `cargo fmt`、`cargo clippy --all-targets -- -D warnings`、`cargo test` 全绿。
- 已覆盖：`TextInput` 选区/撤销/按词编辑；lx/ask 选区替换与剪切；todo 两套撤销；
  `Ctrl+Shift+A` 全选、`Ctrl+A/E` 行首尾与 `Mod+C/X/V` 的可编辑目标路由；lx 无选区 Ctrl+C 清空；终端 Paste 与按键回退的代次去重；
  原生写入确认后才删除的剪切事务；最终 Buffer 屏幕取样、宽字符去重、反显高亮；
  Shift+拖动跨区域选择、双击词/三击行、松开保留选区。
- 复制/剪切反馈统一在屏幕顶部居中并显示字符数；三击整行按点击处最近的左右边框分段，
  竖边框列与纯横边框行不进入选区，边框上的标题/状态文字仍可复制。
- Todo 条目行不启动屏幕双击/三击选词或选行，双击交还 Todo 行内编辑；列表外空白仍可选择。
- 弹窗、todo 编辑框与 ask 手写输入已接入全选/行首/行尾；全选后移动光标会取消选区。
- `Ctrl+A` 恢复行首、`Ctrl+E` 行尾、`Ctrl+Shift+A`（macOS `Cmd+A`）全选；
  lx 页无选区 `Ctrl+C` 清空当前输入，有选区只复制。
- 尚待用户实测：终端对 `Cmd` 的拦截差异、Shift+拖动 PTY、真实终端里的跨区域视觉高亮、
  粘贴去重时序与弹窗内文字拖动选择。
