# lx 输入框快捷引用与快捷删除功能代码审查 (Code Review)

本次 Review 覆盖分支 `feat/lx-input-refer-and-quick-delete` 引入的全部改动（包括底层语法解析、Prompt 适配、Lx 状态机、渲染层、上下文装配及鼠标按键交互）。

审查发现的问题按照严重级别从高到低（P0 - P2）归类如下，并明确记录关键点、错误原因与修复策略。

---

## 缺陷清单与优先级排序

### [P0] 快捷引用保留末两段路径后导致 `prepare_user_message` 无法解析深层物理文件，附件静默丢失

- **关键点**：
  - 用户消息发送前，`chat::prepare::prepare_user_message` 负责将用户消息正文中的 `@[ref-content]/...` 解析为数据库与模型上下文的 `MessageAttachmentInput`。
  - 此前逻辑中直接使用 `resolve_reference_path(workspace_root, trimmed_token)`（即直接以 `workspace_root.join(trimmed_token)` 检查 `metadata`）。
- **错误原因**：
  - 当快捷引用根据新需求默认保留末两段路径时（例如原文件为 `src/ui/main_content/lx/refer.rs`，插入格式为 `@[ref-content]/lx/refer.rs`），`trimmed_token` 仅为 `lx/refer.rs`。
  - 在工作区根目录下直接通过 `workspace_root.join("lx/refer.rs")` 访问必然返回 `NotFound`。
  - 这导致该快捷引用被直接跳过，`attachments` 中未被记录，模型上下文完全无法加载该物理文件。且即使直接命中，`source_path` 也会被错误记录为 `lx/refer.rs` 而非真实相对路径。
- **修复方案**：
  - 增强 `prepare_user_message` 中的路径解析：
    1. 首先尝试直接解析 `workspace_root.join(trimmed_token)`（覆盖单段或根目录文件）；
    2. 若直接不存在，优先在当前消息已解析出的 `attachments` 中比对是否已有完整相对路径其末尾匹配 `/{trimmed_token}` 或等于 `trimmed_token`；
    3. 若仍未找到，调用 `crate::files::scan(workspace_root)`，在工作区候选路径中搜索后缀匹配 `/{trimmed_token}` 或等于 `trimmed_token` 的条目；
    4. 找到真实相对路径后，将其绝对路径传入 `metadata` 和图片嗅探，并将附件的 `source_path` 设置为真实的完整相对路径。

---

### [P1] `Lx` 快捷候选融合时未对短路径与完整相对路径进行别名去重与全路径保留

- **关键点**：
  - `src/app/state/lx/refer.rs` 的 `refresh_refer_panel` 将当前输入框中的候选与当前会话用户历史中的 `user_history_reference_paths` 进行合并。
- **错误原因**：
  - 合并去重时仅执行了简单的 `c.path == candidate.path` 字符串完全相等比对。
  - 当当前输入框已存在 `@[ref-content]/lx/refer.rs`（path 为 `lx/refer.rs`），而历史消息中存在完整路径 `@src/ui/main_content/lx/refer.rs` 时，由于字符串不相等，面板会出现两条指向同一个文件的候选。
  - 更严重的是，如果输入框的残缺短路径排在前面，候选记录的 `path` 也会退化为无完整前缀的短路径。
- **修复方案**：
  - 合并候选时引入 `is_duplicate` 语义判断（相同末两段且同类型判定为别名重复）；
  - 若已存在的候选路径较短（缺少前缀层级），而历史路径为完整相对路径，则使用完整路径替换升级已有候选，确保候选携带最完整的路径信息。

---

### [P1] `Lx` 快捷引用面板缺少滚轮滚动（Mouse Wheel）视口状态与事件分派

- **关键点**：
  - 当候选条目较多（超过 `MAX_POPUP_ROWS` 8 条）时，在弹出的快捷面板上滑动鼠标滚轮，用户期望滚动候选视口查看更多项。
- **错误原因**：
  - `ReferPanel` 结构体缺少 `viewport: Option<usize>` 字段与 `scroll_viewport` 方法；
  - `src/ui/main_content/lx/refer.rs` 中的 `layout` 函数被标记为 `#[cfg(test)]`，未接入 `src/event/mouse/wheel.rs`；
  - `wheel.rs` 中在处理 lx 窗格时，只检查了 `mention_layout` 和 `slash_layout`，漏掉了 `refer_layout`，导致面板区域的滚轮事件漏到底层输入框或历史视口。
- **修复方案**：
  - 在 `ReferPanel` 中增加 `viewport: Option<usize>` 与 `scroll_viewport` 支持；
  - 在 `ui::main_content::lx::refer` 中导出 `layout`，将 `data.viewport` 传给 `CommandPanelView::window_start`；
  - 在 `app::update::pane` 中实现 `scroll_lx_refer(state, pane, delta, base)`；
  - 在 `src/event/mouse/wheel.rs` 中增加对 `lx::refer_layout` 的命中判断与滚轮滚动处理。

---

### [P1] `LxMention` 未暴露扫描条目缓存导致未带尾斜杠的目录在快捷候选生成时被误判为文件

- **关键点**：
  - `src/app/state/lx/refer.rs` 调用 `refer_candidates` 时，第四个参数传了空切片 `&[]`。
- **错误原因**：
  - `refer_candidates` 依赖传入的 `MentionEntry` 列表来判断 `@dir`（正文中未显式加尾斜杠的目录提及）是否是目录。
  - 由于传入 `&[]`，此类提及在生成候选时会被识别为普通 `File` 而非 `Folder`，确认插入时不会补尾斜杠。
- **修复方案**：
  - 在 `LxMention` 上新增 `pub fn entries(&self) -> &[MentionEntry]`，直接返回内部的 `cache` 切片；
  - 在 `Lx::refresh_refer_panel` 中将 `self.mention.entries()` 传给 `refer_candidates`。

---

### [P2] `Lx` 删除类文本编辑操作未复位 `refer_suppressed` 标记

- **关键点**：
  - 用户按 `Esc` 关闭快捷面板后，`refer_suppressed` 置为 `true` 以防止原处重复弹窗。
- **错误原因**：
  - 只有在 `insert_char` 和光标移动（`move_left` / `move_right` 等）时将 `refer_suppressed` 重置为 `false`。
  - 在用户按退格键 `backspace()`（删除字符）、`delete()`、`delete_word_backward()` 等删除编辑时，遗漏了重置 `refer_suppressed = false;`。
  - 导致用户按 `Esc` 关闭后，按退格键修改触发词时，面板依然被压制无法响应。
- **修复方案**：
  - 在 `backspace()`、`delete()`、`delete_to_line_start()`、`delete_to_line_end()`、`delete_word_backward()`、`delete_word_forward()` 等编辑方法中统一重置 `self.refer_suppressed = false;`。
