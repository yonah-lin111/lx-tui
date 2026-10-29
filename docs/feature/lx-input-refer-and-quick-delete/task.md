# lx 输入框快捷引用与快捷删除开发任务

状态：已完成并全部通过测试验证。
对应设计：`docs/feature/lx-input-refer-and-quick-delete/design.md`。

---

## 阶段 1：语法解析与快捷删除底层（`app/markdown`）

- [x] 1.1 新增 `src/app/markdown/ref_content.rs`
  - [x] 定义 `pub const REF_CONTENT_PREFIX: &str = "@[ref-content]/";`
  - [x] 实现 `ref_content_ranges(text: &str) -> Vec<Range<usize>>` 扫描有效区间
  - [x] 实现 `ref_content_deletion_range(text: &str, cursor: usize) -> Option<Range<usize>>` 识别紧随空格的快捷删除区间
  - [x] 实现 `extract_mention_paths(text: &str) -> Vec<(String, bool)>` 辅助提取文本中的提及
  - [x] 编写独立单元测试覆盖：光标位置边界、行首边界、多空格、非字符边界等用例
- [x] 1.2 改造 `src/app/markdown/refer_shortcut.rs`
  - [x] 将候选生成格式由 `@ref/末两段` 调整为 `@[ref-content]/<相对路径> `（目录带尾斜杠）
  - [x] 更新既有单元测试适配新格式断言
- [x] 1.3 在 `src/app/markdown.rs` 中导出新符号

---

## 阶段 2：Prompt 编辑器同步改造（对齐与向后兼容）

- [x] 2.1 调整 `src/app/prompt/refer.rs`
  - [x] 确认快捷候选插入使用统一的 `@[ref-content]/<相对路径> ` 格式
- [x] 2.2 调整 `src/app/prompt.rs` 的 `backspace()`
  - [x] 在快捷删除链路中首选检查 `ref_content_deletion_range`
  - [x] 保留既有 `reference_deletion_range`、`short_ref_deletion_range` 与 `mention_deletion_range` 向后兼容
- [x] 2.3 更新 `src/app/prompt/tests.rs` 相应测试用例

---

## 阶段 3：会话历史候选提取与 Lx 状态层（`app/state/lx`）

- [x] 3.1 扩展 `src/app/state/lx/chat.rs`
  - [x] 实现 `user_history_reference_paths(&self) -> Vec<(String, bool)>`
  - [x] 严格按要求仅提取用户历史消息的 `attachments` 与正文显式 `@` 提及，逆序去重
  - [x] 编写对应单元测试
- [x] 3.2 新增 `src/app/state/lx/refer.rs`
  - [x] 在 `Lx` 上维护 `refer: Option<ReferPanel>` 快捷面板状态
  - [x] 实现 `refresh_refer_panel()`：融合当前输入框正文引用与会话历史用户引用
  - [x] 实现 `refer_move(delta)`、`refer_confirm()`、`refer_escape()`、`refer_set_active(index)`
  - [x] 确保确认后压制重新弹起，撤销快照记录正确
- [x] 3.3 升级 `src/app/state/lx.rs`
  - [x] 在 `Lx` 结构中加入 `refer` 状态
  - [x] 升级 `backspace()`：在光标处于尾随空格后时，链式调用 `ref_content_deletion_range`、`mention_deletion_range` 等执行整块删除
  - [x] 在 `insert_char`、`move_cursor`、`delete`、模式切换等处正确调度 `refresh_refer_panel()`

---

## 阶段 4：面板渲染与事件交互（`ui` & `update`）

- [x] 4.1 新增 `src/ui/main_content/lx/refer.rs`
  - [x] 实现 `refer_panel_data`、`layout`、`render`、`item_at`
  - [x] 样式对齐现有命令面板规范，贴输入框上沿展开，高度对齐 `MAX_POPUP_ROWS`
- [x] 4.2 在 `src/ui/main_content/lx.rs` 接入
  - [x] 导出并在 `render` 流程中挂载 `render_refer_panel`
  - [x] 保证面板优先级（粘贴 > 提及 > 快捷引用 > 斜杠）
- [x] 4.3 在 `src/app/update/pane.rs` 挂载按键与鼠标事件
  - [x] 当 `lx.refer_panel()` 打开时，优先捕获 `Up` / `Down` / `Enter` / `Tab` / `Esc`
  - [x] 鼠标悬停高亮与点击确认命中

---

## 阶段 5：消息上下文装配与历史时间线展示（`chat/prepare` & `history`）

- [x] 5.1 升级 `src/chat/prepare.rs`
  - [x] 支持解析 `@[ref-content]/<相对路径>` 格式作为附件，加载物理文件供模型使用
  - [x] 改造 `strip_references_for_display`：不剥离 `@[ref-content]/`，在用户消息正文中完整保留
- [x] 5.2 调整 `src/ui/main_content/lx/history/snapshot.rs`
  - [x] 在构建 `ref_lines` 附件树时，排除 `@[ref-content]/` 快捷引用，不在底部展示树状分支
  - [x] 编写时间线渲染快照测试

---

## 阶段 6：全量校验与验收

- [x] 6.1 运行精准单测：
  - `cargo test app::markdown::ref_content`
  - `cargo test app::state::lx`
  - `cargo test ui::main_content::lx`
  - `cargo test chat::prepare`
- [x] 6.2 运行全量静态检查：
  - `cargo fmt --check`
  - `cargo clippy --all-targets -- -D warnings`
- [x] 6.3 运行整体测试套件确保无回归
