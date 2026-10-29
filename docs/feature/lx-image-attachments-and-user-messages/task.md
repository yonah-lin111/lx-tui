# lx 图片附件与用户消息交互任务清单

> 状态：已完成。

## 1. 本地图片引用、归档与限额

- [x] 移除系统剪贴板位图直接读取与 `arboard` 依赖，仅支持本地图片文件（粘贴图片路径弹出“引用 / 路径”二级面板转 `@[refer-image](path)`，或 `@` 提及）。
- [x] 实现 `src/platform/clipboard_image.rs`：单张 20 MiB、单条消息 4 张 / 40 MiB、像素上限 32M、最长边 2048px 等比缩放、`<= 2048px` PNG/JPEG 免重编码快速路径与 `IMAGE_CACHE` 内存缓存。
- [x] 为 `Cargo.toml` 中图像编解码依赖配置 `[profile.dev.package.*] opt-level = 3`，消除 debug 模式大图缩放卡顿。
- [x] 实现会话级图片归档 `~/.lx-t[-dev]/images/<session_id>/<hash>-<name>`、启动时 30 天（对齐 Claude Code `cleanupPeriodDays`）未引用图片自动 GC、删除会话时同步清理会话图片目录。

## 2. 引用解析、数据库迁移（v18）与 Rewind 兜底

- [x] 实现 `src/chat/prepare.rs`：解析 `@[refer-file/folder/image](path)`、`@相对路径` 与唯一 `@ref/` 短引用，跳过围栏代码块与行内反引号，按原文位置排序并去重。
- [x] `src/db/migration.rs` 新增 `SCHEMA_V18`（`agent_message_attachment` 无 BLOB 表），推进 `SCHEMA_VERSION = 18`。
- [x] `src/event/chat/rewind/restore.rs` 在 `rewind` 回填草稿时检查外部原图是否存在；若原图已删，自动将草稿中的图片引用路径替换为 `~/.lx-t[-dev]/images/<session_id>/` 归档副本路径并刷新 `mtime`。

## 3. 多模态协议编码、`read` 工具读图与模型模态门控

- [x] 支持 `openai`、`openai-compatible`、`openai-responses`、`anthropic`、`google` 五种协议的 `ModelItem::UserContent` 图像编码。
- [x] 解析模型配置 `modalities.input` 是否含 `"image"`（`supports_image_input`）：不支持图像的模型在输入框发送图片时直接拦截提示。
- [x] 增强 `read` 工具支持读取本地图片并在 `agent_loop` 中注入多模态图像；当模型不支持 `image` 输入时，`read` 工具返回不支持图片识别错误。
- [x] 参考 OpenCode `unsupportedParts`，当会话中途切换到不支持 `image` 输入的模型时，历史轮次中的图片自动降级为文本错误说明。

## 4. 用户历史消息 5 行折叠与引用树展示

- [x] 用户消息展示副本剥离有效引用标记（会话标题、预览与置顶行同步剥离），按内容宽度软换行后超过 5 视觉行时折叠并显示 `… N more lines` / `less lines` 可点击切换。
- [x] 正文下方以 `├─` / `└─` 渲染引用树，连接符默认前景色，文件名按 file / folder / image 语义色高亮，不铺用户消息底色。
- [x] 完成单元测试、Clippy 零警告与格式化校验。
