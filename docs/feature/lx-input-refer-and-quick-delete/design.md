# lx 输入框快捷引用与快捷删除设计

状态：设计已完成（经 Grill-Me 质问决策闭环），待用户审核确认后执行。

## 1. 目标与验收口径

在 lx 主视图的底部多行输入框中，支持参考右侧 Prompt 编辑器的“快捷输入”与“快捷删除”能力：
1. **快捷输入**：用户键入至少 2 个字符（无需先按 `@`），系统自动从当前输入框及**当前会话用户历史消息**中收集已出现过的提及与附件，弹出快捷引用候选面板（ReferPanel）；选中确认后，插入统一格式的 `@[ref-content]/<相对路径> `。Prompt 编辑区的快捷输入同步统一调整为该格式。
2. **快捷删除**：光标紧跟在引用标记尾随空格后（包括补全插入后的默认光标位置）按 Backspace 时，一步删净整段引用标记（含尾随空格），单步可撤销；光标在路径内部或未打空格的末尾按 Backspace 时，执行普通逐字删除。
3. **消息历史呈现与上下文装配**：用户消息发送后，`@[ref-content]/...` 在消息正文中完整保留，**消息底部的附件树（`ref_lines`）中不显示快捷引用**；后台发送给模型时，依然作为有效上下文文件进行装配。

---

## 2. 已确认的产品与技术决策（Grill-Me 闭环）

| 编号 | 决策项 | 结论与具体行为 |
| --- | --- | --- |
| **D1** | **快捷输入候选来源** | 仅收集 **当前输入框文本中已有的引用** + **当前会话中用户历史消息发送过的提及（`@相对路径`）与附件路径**。不收集 AI 工具调用文件，不正则抓取 AI 回复的纯文本。 |
| **D2** | **快捷引用插入格式** | 统一采用 **`@[ref-content]/<末两段路径> `**（默认保留最后两个 `/` 的内容，不足两段用全部段，例如 `@[ref-content]/src/main.rs `，目录带尾斜杠 `@[ref-content]/agent/lx-agent/ `）。**Prompt 编辑器同步统一切换为该格式**。 |
| **D3** | **用户消息展示行为** | 发送后，用户消息正文**完整保留** `@[ref-content]/...` 文本（不被 `strip_references_for_display` 剥离）；**用户消息卡片底部的附件折叠树（`ref_lines`）绝不出现快捷引用的条目**。 |
| **D4** | **模型上下文装配** | 发送消息前，`chat::prepare` 将 `@[ref-content]/<路径>` 识别为有效的文件/目录引用，读取文件内容装配进 Prompt 提示词上下文，保证 AI 能够感知该文件。 |
| **D5** | **快捷删除触发口径** | **严格对齐 Prompt**：光标紧跟在引用标记后的**空格**处按 Backspace 时，一次性删除整块标记（含空格）；若光标在标记内部或末尾无空格，按普通字符退格（支持用户微调路径）。覆盖 `@相对路径 `、`@[refer-*](path) `、`@[ref-content]/<路径> ` 及向后兼容的 `@ref/... `。 |
| **D6** | **面板交互与互斥优先级** | 优先级为：粘贴面板 > `@` 提及面板 > **字母快捷面板 (Refer)** > `/` 斜杠面板 > Markdown 块命令面板。`Up/Down` 循环移动高亮，`Enter/Tab` 确认插入，`Esc` 关闭并压制到下一次移动/编辑，支持鼠标悬停与点击。 |
| **D7** | **浮层几何与对齐** | 快捷面板贴输入框上沿展开，水平方向与输入框左右对齐等宽，高度上限遵循 codex `MAX_POPUP_ROWS`（8 行内容 + 2 行边框），空间不足时向上压缩且不挤压输入框。 |
| **D8** | **语法高亮颜色** | `@[ref-content]/...` 拥有专属高亮色：**柔紫（`Color::Indexed(141)`）加粗**，在编辑器和 lx 输入框中与普通 `@` 提及黄、`refer-file` 绿、`refer-folder` 橙一眼区分。 |

---

## 3. 架构设计与数据流

### 3.1 模块依赖关系

```text
src/app/markdown/
  ├── refer_shortcut.rs    <-- 升级候选收集，生成 @[ref-content]/<相对路径> 格式
  ├── ref_content.rs       <-- [新增] @[ref-content]/ 语法解析、区间识别与快速删除边界
  └── mod.rs               <-- 导出 ref_content_deletion_range 等
src/app/state/lx/
  ├── refer.rs             <-- [新增] Lx 的快捷面板状态维护（触发检测、候选过滤、选区替换）
  ├── mod.rs               <-- 组合 refer 状态，扩展 backspace 快捷删除
  └── chat.rs              <-- 提供 user_historical_reference_paths() 候选提取
src/ui/main_content/lx/
  ├── refer.rs             <-- [新增] lx 快捷面板渲染与命中纯几何
  └── mod.rs               <-- 挂载 render_refer_panel
src/chat/prepare.rs        <-- 支持 @[ref-content]/ 附件解析，且在 strip_references_for_display 中区别处理
src/ui/main_content/lx/history/snapshot.rs <-- 过滤 ref_lines，不在底部展示快捷引用
```

### 3.2 详细流程与状态机

#### 流程 1：快捷输入触发与候选收集 (Refer Panel)

1. 用户在 lx 输入框正常敲击字符（例如输入 `ma`）：
   - `Lx::insert_char` / `splice_text` 触发 `refresh_panel()`；
   - 若当前未处于 `@` 提及、粘贴或斜杠面板状态，调用 `self.refresh_refer_panel()`；
2. `markdown::refer_trigger(&self.input, self.cursor)` 校验：
   - 光标前至少 2 个路径/单词字符，前面是单词边界且非 `@`；不在反引号围栏内；
3. 收集候选源集：
   - 来源 A（当前输入框）：扫描 `self.input` 中已有的 `@` 提及、`@[refer-*]` 以及 `@[ref-content]/`；
   - 来源 B（会话历史）：调用 `self.chat.user_history_reference_candidates()`，提取当前会话历史中所有用户消息的 `attachments` 及正文 `@` 提及；
   - 路径去重（保序），按子序列及前缀匹配用户输入的 `query`（如 `ma` 优先匹配 `src/main.rs`）；
4. 若有匹配候选，构造 `ReferPanel` 并展示；若无候选则保持关闭。

#### 流程 2：采纳插入与撤销 (Confirm)

1. 用户按 `Enter` / `Tab` 或鼠标左键点击候选：
   - 选中的候选转换为：`@[ref-content]/<相对路径>` + 尾随空格（如目录带 `/`，即 `@[ref-content]/src/ `）；
   - 将触发词范围（如 `ma`）替换为上述文本；
   - 光标停在尾随空格之后；
   - 记录快照，作为单步独立撤销步（按 `Ctrl+Z` 可单步恢复回 `ma`）；
   - 压制面板，防止再次自动弹起。

#### 流程 3：快捷删除 (Quick Backspace)

1. 用户在尾随空格处按 `Backspace`：
   - 命中 `ref_content_deletion_range(&self.input, self.cursor)`；
   - （或 `mention_deletion_range` / `reference_deletion_range` / `short_ref_deletion_range`）；
   - 返回 `Range { start, end }`，直接将整段标记（如 `@[ref-content]/src/main.rs `）全部删除；
   - 光标回到标记起点 `start`，记录单步快照，一步撤销。
2. 若光标不在空格处（例如在路径内部）：
   - 上述 range 检测均返回 `None`；
   - 回退至普通单字符退格，删除光标前一个 UTF-8 字符。

#### 流程 4：发送与展示分流 (Send & Render)

1. 用户提交输入框内容到会话：
   - `chat::prepare_user_message`:
     - 解析 `@[ref-content]/<相对路径>`，将其解析为 `MessageAttachmentInput`，`kind` 设为 `File` 或 `Directory`；
     - `marker_start` 与 `marker_end` 记录标记位置；
     - 标记此 attachment 为 `is_inline_refer: true`（或通过专用 AttachmentKind 区分）；
2. 历史时间线渲染：
   - 正文展示：`strip_references_for_display` 跳过 `is_inline_refer` 的引用，使其在正文文字中保持可见（如显示为 `请看 @[ref-content]/src/main.rs`）；
   - 底部附件行：`BlockPaint::UserText` 的 `ref_lines` 过滤掉 `is_inline_refer`，因此底部不会出现树状附件卡片；
3. 后台上下文装配：
   - 无论是否为 inline refer，只要是附件，系统均读取其物理文件内容，组装至发送给 LLM 的 context payload 中。

---

## 4. 关键代码设计与数据结构

### 4.1 语法解析与删除范围 (`src/app/markdown/ref_content.rs`)

```rust
pub const REF_CONTENT_PREFIX: &str = "@[ref-content]/";

/// 检测 @[ref-content]/ 快捷引用的整块删除范围（光标必须紧跟在尾随空格后）
pub fn ref_content_deletion_range(text: &str, cursor: usize) -> Option<Range<usize>> {
    if cursor == 0 || cursor > text.len() || !text.is_char_boundary(cursor) {
        return None;
    }
    let prev = text[..cursor].chars().next_back()?;
    if prev != ' ' && prev != '\t' {
        return None;
    }
    let end = cursor - prev.len_utf8();
    ref_content_ranges(text)
        .into_iter()
        .find(|range| range.end == end)
        .map(|range| range.start..cursor)
}

/// 扫描全文中的全部 @[ref-content]/<path> 范围
pub fn ref_content_ranges(text: &str) -> Vec<Range<usize>> {
    let mut ranges = Vec::new();
    let mut index = 0;
    while let Some(offset) = text[index..].find(REF_CONTENT_PREFIX) {
        let at = index + offset;
        // 必须位于行首或空白后
        let boundary = text[..at]
            .chars()
            .next_back()
            .is_none_or(|ch| ch.is_whitespace());
        if !boundary {
            index = at + REF_CONTENT_PREFIX.len();
            continue;
        }
        let body_start = at + REF_CONTENT_PREFIX.len();
        let mut end = body_start;
        for ch in text[body_start..].chars() {
            if ch.is_whitespace() || ch == '@' {
                break;
            }
            end += ch.len_utf8();
        }
        if end > body_start {
            ranges.push(at..end);
            index = end;
        } else {
            index = body_start;
        }
    }
    ranges
}
```

### 4.2 Lx 输入框退格删除链 (`src/app/state/lx.rs`)

```rust
pub fn backspace(&mut self) {
    if self.input_mode == LxInputMode::Shell && self.cursor == 0 {
        self.snapshot();
        self.input_mode = LxInputMode::Chat;
        self.refresh_panel();
        return;
    }
    if self.cursor == 0 {
        return;
    }
    // 快捷删除：当光标紧跟在有效引用的尾随空格后时，整块一次删净
    if !self.mention.is_open()
        && self.refer.is_none()
        && let Some(range) = crate::app::markdown::ref_content_deletion_range(&self.input, self.cursor)
            .or_else(|| crate::app::markdown::mention_deletion_range(&self.input, self.cursor))
            .or_else(|| crate::app::markdown::reference_deletion_range(&self.input, self.cursor))
            .or_else(|| crate::app::markdown::short_ref_deletion_range(&self.input, self.cursor))
    {
        self.snapshot();
        self.cursor = range.start;
        self.input.replace_range(range, "");
        self.refresh_panel();
        return;
    }
    // 普通单字符退格
    if let Some(prev) = self.input[..self.cursor].chars().next_back() {
        self.snapshot();
        self.cursor -= prev.len_utf8();
        self.input.remove(self.cursor);
        self.refresh_panel();
    }
}
```

### 4.3 会话历史引用提取 (`src/app/state/lx/chat.rs`)

```rust
impl LxChat {
    /// 提取当前会话中用户历史消息发送过的所有文件及引用路径（按最近使用逆序去重）
    pub fn user_history_reference_paths(&self) -> Vec<(String, bool)> {
        let mut seen = std::collections::HashSet::new();
        let mut results = Vec::new();

        for msg in self.messages.iter().rev() {
            if msg.role != "user" {
                continue;
            }
            // 1. 从 attachments 提取
            for att in &msg.attachments {
                if let Some(path) = att.source_path.as_ref().or(Some(&att.label)) {
                    if seen.insert(path.clone()) {
                        results.push((path.clone(), att.kind == MessageAttachmentKind::Directory));
                    }
                }
            }
            // 2. 从正文中提取显式 @ 提及
            for part in &msg.parts {
                if part.kind != "text" {
                    continue;
                }
                for (path, is_dir) in crate::app::markdown::extract_mention_paths(&part.content) {
                    if seen.insert(path.clone()) {
                        results.push((path, is_dir));
                    }
                }
            }
        }
        results
    }
}
```

---

## 5. 边界与异常情况处理

1. **同名文件去重**：历史消息若多次出现同一个路径，严格去重并保留最近一次顺序。
2. **空输入框/无匹配**：未键入满足 2 字符的查询词或无匹配项时，快捷面板绝对不显示，不干扰用户普通输入。
3. **输入框模式切换**：进入 Shell 模式（`!` 开头）时，快捷引用面板被禁用并关闭，保证 Shell 命令不受 Markdown 补全打扰。
4. **与其他面板互斥**：输入框中键入 `@` 时优先触发 `@` 提及面板；如果快捷面板正打开，按下 `@` 将关闭快捷面板转为提及面板。
5. **历史向后兼容**：对旧历史中可能存在的 `@ref/` 格式短引用，快捷删除与模型上下文装配继续保留兼容支持，平滑过渡。

---

## 6. 验证与测试规范

1. **单元测试覆盖**：
   - `ref_content_deletion_range`：覆盖光标在空格后、空格前、路径中、无空格、多余空格等各种边界；
   - `refer_trigger`：校验 2 字符触发门槛与代码围栏隔离；
   - `user_history_reference_paths`：校验只收集用户消息、排除 AI 工具输出、逆序去重；
   - `backspace`：校验撤销快照只生成一次，一步撤销回触发前状态；
   - `prepare_user_message`：校验 `@[ref-content]/` 正确装配为附件，且 `strip_references_for_display` 保留正文。
2. **静态检查**：`cargo clippy --all-targets -- -D warnings` 与 `cargo fmt --check`。
3. **回归测试**：运行 `cargo test` 确保既有 Prompt 树、提及面板及会话模块测试全部通过。
