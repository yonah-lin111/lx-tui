# lx 图片附件与用户消息交互设计

> 状态：已实现并完成验证。
> 基线：主 checkout `SCHEMA_VERSION = 17` 升级至 `SCHEMA_VERSION = 18`（新增 `agent_message_attachment` 表）。

## 1. 目标与验收边界

在 lx 的 Native Agent 中支持本地图片引用与多模态模型交互：
1. **粘贴本地图片路径**：弹出“引用 / 路径”二级面板，选择“引用”后转为 `@[refer-image](/path/to/image.png)`，选择“路径”则保留普通路径文本。
2. **`@` 提及与短引用**：支持 `@相对路径` 与 `@ref/` 短引用指向本地图片、文件或文件夹。
3. **会话级图片归档与 Rewind 兜底**：参考 Claude Code，发送时将验证/缩放后的图片归档至 `~/.lx-t[-dev]/images/<session_id>/`，数据库仅记录元数据与归档路径（不存图片 BLOB）；应用启动时按 30 天保留期（对齐 Claude Code `cleanupPeriodDays = 30`）清理未引用的过期图片，删除会话时立即清理对应 `<session_id>` 图片目录；执行 `rewind` 回填草稿时若桌面等外部原图已被删除，自动将引用标记改写为受管归档路径。
4. **`read` 工具支持图片与模态门控**：当模型对本地图片路径调用 `read` 工具时，若当前模型支持 `image` 输入模态（设置页 `modalities.input` 勾选 `"image"`），自动解码缩放并在下一采样轮次作为多模态图像注入模型上下文；若当前模型不支持 `image` 输入模态，工具返回明确错误提示，且输入框直接发送图片时同步拦截提示；中途 `/model` 切换到不支持图像的模型时，历史轮次中的图片自动降级为文本错误说明（参考 OpenCode `unsupportedParts`）。
5. **用户消息历史展示**：正文按软换行后的视觉行最多显示 5 行，超出时显示可点击的 `… N more lines` / `less lines`；有效引用在正文下方以 `├─` / `└─` 树形分支独立展示（`├─ File: xxxx` / `├─ Folder: xxxx` / `└─ Image: xxxx`，连接符与类型标签默认前景，`:` 后的引用名使用 prompt 三个 refer 语义色，无用户消息底色）。

### 已确认的决策

| 决策 | 具体口径 |
| --- | --- |
| 图片入口 | 仅使用本地图片文件（不读取系统剪贴板位图）：粘贴本地图片路径弹出“引用 / 路径”二级面板，或通过 `@` 提及引用本地图片，或由模型调用 `read` 工具读取图片路径。 |
| 协议范围 | `openai`、`openai-compatible`、`openai-responses`、`anthropic`、`google` 五种协议全部支持多模态图像编码。 |
| 模态门控与切换 | 严格按模型配置 `modalities.input` 是否包含 `"image"` 判定：不支持时输入框发送图片直接拦截、`read` 工具读图返回不支持错误、会话中途切到纯文本模型时历史图片自动替换为错误说明文本。 |
| 持久化与归档 | 数据库 `agent_message_attachment` 表不存 BLOB，图片归档至 `~/.lx-t[-dev]/images/<session_id>/<hash>-<name>`；桌面原图被删后历史回放与 `rewind` 均从归档副本恢复。 |
| 自动清理 | 启动时自动清理未被任何会话引用且超过 30 天（`IMAGE_RETENTION_SECS = 30 * 24 * 3600`）的归档图片；主动删除会话时立即删除其归档子目录。 |
| 折叠与引用树 | 仅用户正文受 5 视觉行折叠限制；引用行以 `├─`/`└─` 始终完整展示且不铺用户消息深色背景。 |
| 资源预算与性能 | 单张原图至多 20 MiB、每条消息最多 4 张、原图合计至多 40 MiB；发送前最长边等比缩放至 2048px（`<= 2048px` 的合法 PNG/JPEG 直接复用原字节免重编码，配合进程内 `IMAGE_CACHE` 消除重复编解码）。 |

## 2. 数据库与迁移（Schema v18）

`src/db/mod.rs::SCHEMA_VERSION` 推进至 `18`，新增 `agent_message_attachment` 表（无 `image_bytes` BLOB 列）：

```sql
CREATE TABLE IF NOT EXISTS agent_message_attachment (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    external_id TEXT NOT NULL UNIQUE,
    message_id TEXT NOT NULL REFERENCES agent_message(external_id) ON DELETE CASCADE,
    position INTEGER NOT NULL CHECK (position >= 0),
    kind TEXT NOT NULL CHECK (kind IN ('file', 'folder', 'image')),
    label TEXT NOT NULL,
    marker_start INTEGER,
    marker_end INTEGER,
    source_path TEXT,
    mime TEXT,
    width INTEGER,
    height INTEGER,
    UNIQUE(message_id, position),
    CHECK ((marker_start IS NULL AND marker_end IS NULL) OR
           (marker_start >= 0 AND marker_end > marker_start)),
    CHECK (kind <> 'image' OR
           (mime IN ('image/png', 'image/jpeg') AND source_path IS NOT NULL
            AND width > 0 AND height > 0))
);
CREATE INDEX IF NOT EXISTS idx_agent_message_attachment_message
    ON agent_message_attachment(message_id, position);
```

- `insert_attachments` 在 `begin_turn_with_attachments` 事务内将图片归档至 `~/.lx-t[-dev]/images/<session_id>/` 并把归档路径写入 `source_path`。
- `load_chat_session` 批量加载附件元数据；`load_image_payloads` 按会话与附件 ID 从磁盘归档路径加载图片字节。
- `delete_chat_session` 在事务提交后调用 `platform::remove_session_images(session_id)` 清理对应图片目录。

## 3. 核心数据流

### 3.1 输入、发送准备与归档

```text
粘贴图片路径 -> paste_open 弹出“引用 / 路径”面板 -> 确认“引用”插入 @[refer-image](path)
Enter 发送 -> try_send 校验模型 supports_image_input
  -> prepare_user_message 解析 @[refer-*] / @path / @ref/（跳过代码围栏与行内反引号）
  -> 校验图片签名、尺寸、4 张 / 20 MiB / 40 MiB 预算并去重
  -> BeginChatTurn 落库并将图片归档至 ~/.lx-t[-dev]/images/<session_id>/
  -> start_stream 装载去重后的图片字节，构造 ModelItem::UserContent 发往各协议
```

### 3.2 Rewind 原图缺失回退

`src/event/chat/rewind/restore.rs` 在执行 `rewind` 删除轮次前读取该轮附件元数据：
- 刷新受管归档图片的 `mtime`；
- 若检测到原引用路径（如桌面截图）已被用户删除，自动按 `marker_start..marker_end` 将回填草稿中的引用改写为 `@[refer-image](~/.lx-t[-dev]/images/<session_id>/...)`，确保回填后可直接再次发送。

### 3.3 `read` 工具读图与模型切换兼容

- **工具层**：`src/chat/tools/read.rs` 在读取普通文件前通过 `sniff_image_file` 检测图片签名。若 `ctx.supports_image_input` 为 `false`，返回 `ERROR: Cannot read "<path>" (this model does not support image input). Inform the user.`；若为 `true`，返回 `ReadKind::Image` 并在 `agent_loop` 中紧随 `ToolResult` 注入 `ModelItem::UserContent`。
- **会话中途切换模型**：在 `start_stream` 装配历史消息时，若当前模型 `supports_image_input == false`，将历史轮次的 `ModelItem::UserContent` 自动降级为 `ModelItem::UserText` 并附带不可读说明文本，避免纯文本模型报错。
