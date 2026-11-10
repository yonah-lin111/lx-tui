# lx 消息列表与置顶行保留 `@` 引用内容设计 (`design.md`)

状态：已实现并完成代码审查、缺陷修复与合并入 `dev` 主分支（合并提交 `ebd3d8e0`）。

## 1. 目标与验收口径

在 `lx` 输入框中使用 `@` 引用文件、目录、图片、Skill、MCP 或子 Agent 并发送消息后：
1. **消息列表用户消息气泡（`BlockPaint::UserText`）**：完整保留用户发送时的全部 `@` 引用原文（包括 `@<相对路径>`、`@[refer-file](...)`、`@[refer-folder](...)`、`@[refer-image](...)`、`@[refer-skill](...)`、`@[refer-mcp](...)`、`@[refer-subagent](...)`、`@[ref-content]/...` 与 `@ref/...`），不再通过 `strip_references_for_display` 从气泡正文中剥离。
2. **气泡下方附件树（`UserRefLine::from_attachments`）**：保持现有 `├─ / └─` 附件树展示规则不变（常规文件/目录/图片/Skill 引用继续在下方展示，`@[ref-content]/...` 快捷引用不在下方附件树重复展示）。
3. **消息列表顶部悬浮置顶行（`sticky_text`）**：向上滚动历史列表时，固定在顶部的单行用户消息摘要同步保留 `@` 引用原文，与消息气泡保持完全一致。
4. **气泡内文本样式（`UserSpanKind::Plain`）**：消息气泡内的 `@` 引用文本保持用户消息普通文本样式（继承用户消息背景色与默认文本前景色），仅底部附件树保留按类型区分的语义色彩，视觉层次清晰。

---

## 2. 已确认的产品与技术决策（Grill-Me 闭环）

| 编号 | 决策项 | 结论与具体行为 |
| --- | --- | --- |
| **D1** | **保留的 `@` 引用范围** | 消息列表中用户消息气泡统一保留全部类型的 `@` 引用原文（`@<路径>`、`@[refer-*](...)`、`@[ref-content]/...`、`@ref/...`），实现输入框与消息列表“所见即所得”。 |
| **D2** | **底部附件树（`ref_lines`）** | 保持现有行为不变：气泡正文保留 `@` 原文，气泡下方继续渲染 `├─ File: ...` / `└─ Folder: ...` 等附件树（`@[ref-content]/...` 继续排除在附件树外）。 |
| **D3** | **置顶用户消息行（`sticky_text`）** | 置顶悬浮行同步保留 `@` 引用原文，不再调用 `strip_references_for_display` 剥离。 |
| **D4** | **气泡内 `@` 文本视觉样式** | 气泡正文内的 `@` 引用采用普通用户消息文本样式（`UserSpanKind::Plain`），不做额外语法高亮染色，避免与底部彩色附件树产生视觉冲突。 |
| **D5** | **会话标题与摘要边界** | `strip_references_for_display` / `strip_references_from_inputs` 仅保留给新建会话初始标题与异步标题生成提炼纯文本摘要使用，不再介入消息列表与置顶行的 UI 渲染。 |

---

## 3. 架构设计与数据流

### 3.1 模块职责与改动范围

```text
src/ui/main_content/lx/history/
  ├── snapshot.rs          <-- 用户消息正文保留 `@` 原文与首行缩进，仅在全文空白且带附件时回退预览文案
  ├── sticky.rs            <-- 置顶行摘要保留 `@` 原文，遍历首个非空文本段并对齐空正文附件回退
  ├── cache/signature.rs   <-- 移除用户消息附件已失效的 marker_start/marker_end 指纹哈希，统一完整 label 哈希
  └── tests/
      ├── user_folding.rs  <-- 新增多类 `@` 引用、单 `@` 引用及首行代码缩进保留的渲染测试
      └── sticky.rs        <-- 新增置顶行保留 `@` 引用及多文本段/空正文附件回退测试
src/app/update/chat/
  ├── web.rs               <-- AI Web `/resume` 合成云端附件引用标记时对文件名执行 `encode_path` 转义
  └── web/tests.rs         <-- 新增含括号云端文件名在 `/resume` 展示与 `/rewind` 重发闭环测试
src/chat/prepare.rs        <-- 更新 `strip_references_for_display` 文档注释，明确其仅服务于会话标题/摘要提取
```

### 3.2 核心渲染与回退流程

1. **消息气泡正文构造（`snapshot.rs`）**：
   - 检查 `part.content.trim().is_empty()`：
     - 若为空白且 `!message.attachments.is_empty()`，按附件类型回退为 `image_message_preview` / `folder_message_preview` / `file_message_preview`；
     - 若非空白，仅裁剪首尾空行与尾部空白（`part.content.trim_matches(['\r', '\n']).trim_end()`），保留首行行内前导缩进及正文中所有 `@` 引用标记；
   - 经 `wrap_user_projection` 执行折叠粘贴块投影与软折行（超过 5 视觉行时支持展开/收起），下方挂载 `UserRefLine::from_attachments(&message.attachments)`。
2. **置顶悬浮行摘要构造（`sticky.rs`）**：
   - 遍历 `message.text_parts()` 取首个非空文本段执行 `Projection::new`；
   - 若所有文本段均为空且含附件，依次按图片数量预览、首个非空附件名、文件夹/文件数量预览回退；
   - 取前 512 字符将控制字符转为空格并折叠为单行文本。
3. **AI Web 云端历史恢复兼容（`web.rs`）**：
   - `on_ai_web_session_loaded` 为云端历史附件合成 `@[refer-image](...)` / `@[refer-file](...)` 时，对文件名执行 `crate::app::markdown::encode_path` 转义，保证含括号/反斜杠的文件名在气泡原文展示与 `/rewind` 重发解析中完全无损。
