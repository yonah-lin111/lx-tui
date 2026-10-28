# lx 图片附件与用户消息交互设计

> 状态：待用户审核。本文仅是设计，不授权修改功能代码、数据库或 Git 状态。
> 基线：2026-10-28，主 checkout `SCHEMA_VERSION = 17`；实施前必须再次核对迁移版本。

## 1. 目标与验收边界

在 lx 的 Native Agent 输入框中接收剪贴板图片数据、剪贴板图片文件和粘贴的图片路径；发送时把图片作为真实多模态内容交给模型，而不是只发送路径字符串。正文中的有效图片 `@` 引用也发送图片。发送后的图片由 lx 保存，原始文件删除后，重开会话仍可显示附件条目并在后续请求中回放图片。

用户消息历史增加两种展示能力：正文按**渲染后的视觉行**最多显示五行，超出时出现可点击的 `more lines`，展开后可收起；有效的 `@` 文件、文件夹、图片引用及直接附加的图片在正文下方独立成行，非末项以 `├─`、末项以 `└─` 连接。连接符使用终端默认前景色，引用内容沿用对应类型的语义色。引用行不铺用户消息底色，也不响应打开文件或预览图片的点击。

### 已确认的决策

| 决策 | 具体口径 |
| --- | --- |
| 图片入口 | 对齐 Codex：系统剪贴板中的图片数据和图片文件；粘贴本机图片路径也可附加。 |
| 协议范围 | `openai`、`openai-compatible`、`openai-responses`、`anthropic`、`google` 全部编码图片；不支持图片的远端明确报错，不静默降级为纯文本。 |
| 持久化 | lx 管理独立副本；源文件删除后历史仍有附件，后续上下文仍能回放。 |
| 引用识别 | 仅展示可解析为实际文件/文件夹/图片的 `@` 引用；普通 `@文字` 留在正文。 |
| 折叠范围 | 仅正文受五行限制；引用行始终完整展示。 |
| 引用样式 | `├─`/`└─` 默认颜色，引用内容按 file/folder/image 类型高亮；引用行无用户消息深色背景。 |
| 引用点击 | 只读展示；本次仅 `more lines`/收起是新增的历史点击行为。 |
| 图片引用 | 有效图片 `@` 引用必须附带实际图片数据；同张图片与直接粘贴附件去重；缺图拒绝提交并保留草稿。 |
| 资源预算 | 原图单张至多 20 MiB、每条消息最多四张、原图合计至多 40 MiB；发送前最长边缩放至 2048 像素。 |

“查看历史图片”指历史中可见稳定的图片附件条目及其信息；终端内绘制位图预览不在本次范围。文字发送、shell 模式、prompt 右栏、工具结果图片、远程 URL 图片和设置页均不扩展为图片入口。

## 2. 代码事实与参考项目

检索已使用 codegraph 与 codebase-memory，三个项目均已建索引。本项目的以下文件在 codebase-memory 覆盖检查中无已记录缺口；设计以实际源码为准，不依赖目录说明中的旧版本号。

| 位置 | 当前事实 | 本次影响 |
| --- | --- | --- |
| `src/event/paste.rs::handle_lx_paste` | `TerminalEvent::Paste(String)` 先识别本机路径；图片已由 `candidates_from_paths` 和 `platform::sniff_image_file` 分类，但目前只会打开“引用/路径”文本面板。 | 图片路径改为可附加；非图片路径与 shell 语义保持原有路由。 |
| `src/app/state/lx/mention.rs::LxMention::confirm` | 文件候选插入 `@相对路径 `；目录带尾 `/`。 | 发送前解析有效提及，冻结类型与展示名。 |
| `src/app/markdown/refer.rs` | 有 `@[refer-file/folder/image](path)` 的无损解析，跳过代码围栏。 | 复用同一解析器确定长引用，不从 UI 文本猜类型。 |
| `src/app/update/chat/mod.rs::try_send` | 从输入框取纯文本，进入 `RequestState::Committing`，投递 `Effect::BeginChatTurn`；DB ack 后 `on_turn_committed` 清空草稿。 | 在 DB commit 之前增加图片准备与失败回滚；图片消息允许空正文。 |
| `src/db/chat/mod.rs::Database::begin_turn` | 单事务写 user text part、streaming assistant 和 session revision；空文本被拒绝。 | 同事务写图片副本与引用元数据；仅文字和附件都为空时拒绝。 |
| `src/chat/context/mod.rs::assemble` | 从消息构建 `turns` 与 `ModelItem::UserText`，按完整轮次裁剪；当前未计图片。 | 保持纯文本形状不变，为带图 user item 计预算并按所选轮次取图片。 |
| `src/chat/transport/mod.rs::build_request` | 五个 transport 名称由四个构造器实现，OpenAI 与 compatible 共用编码器。 | 四处 user content 编码分支需覆盖多模态。 |
| `src/ui/main_content/lx/history/{snapshot,paint,cache}.rs` | `Builder::build_step` 产生 `UserText` 视觉行；快照行数同时服务绘制、滚动与命中。 | 五行裁剪、引用行和命中必须同一快照计算，不在 draw 时额外增行。 |
| `src/ui/main_content/lx/history_hit.rs` | 已有卡片命中面，状态转换在 `app/update/chat/viewport.rs`。 | 添加按消息稳定 ID 的用户正文折叠目标。 |

Codex 的 `codex-rs/tui/src/clipboard_paste.rs::paste_image_as_png` 优先取剪贴板文件列表，取不到再读图片像素，并统一编码 PNG；`paste_image_to_temp_png` 再写临时文件。其 composer 的 `AttachmentState::attach_image` 插入可见占位并保留图片路径；`fixed.paste_image` 绑定 `Ctrl+V` 和 `Ctrl+Alt+V`。Codex 的模型前图片处理有最长边缩放与 patch 限制；lx 采用相同的用户入口，但因为需要源文件删除后仍可回放，最终副本放在 lx 数据库中。

OpenCode 的 `packages/tui/src/component/prompt/index.tsx::pasteAttachment` 同样把可见占位与文件元数据绑定到独立附件对象，而非把路径文本当作图片本身。其逻辑仅作为交叉参考，不照搬 TS 状态模型。

## 3. 数据与状态边界

### 3.1 草稿、已提交消息与图片副本

`Lx` 持有纯状态 `draft_images`：图片的稳定草稿 ID、来源、展示名、已准备好的像素/编码字节及插入顺序。它不是消息历史，也不包含数据库句柄。剪贴板读取与路径文件读取在 `event/` 发起的后台任务执行；OS 剪贴板访问只放在 `platform/`，结果作为纯数据回投。草稿图片支持多张；粘贴失败不修改既有文字/附件。Backspace 或选择删除附件需有明确命中和单步撤销语义，沿用 lx 编辑器已有的 undo/redo，不让孤立占位留在文字中。

发送时冻结正文、草稿附件和输入修订号，进入准备态。准备态解析 `@` 引用并按冻结的工作区根解析相对路径，读取图片、验证格式、尺寸和资源预算，再把图片规范化为可供所有协议发送的 PNG 或 JPEG。路径引用与剪贴板附件若指向相同规范路径或经字节比较为同一图片，只保留一份图片数据；历史仍保留各处可见引用的顺序。非图片引用只冻结类型、原标记位置和展示路径，不读取文件正文。`@` 短引用只在能唯一映射到本条草稿中已有完整引用时识别；不能唯一解析时留在正文，不猜路径。

与 Codex 一样允许**只有图片、没有文字**的消息。空草稿判定为 `text.trim().is_empty() && images.is_empty()`。图片消息的会话标题/预览使用 `ui/text.rs` 中稳定的图片文案，不把本地绝对路径或二进制写进预览。

已提交消息在内存只保留 `UserAttachmentMeta`，包括稳定附件 ID、序号、类型、展示名、来源标记的字节范围、图片 MIME 与尺寸；**不把历史图片 BLOB 放入 `AppState` 或历史块缓存**。数据库按需加载图片字节。本轮的冻结草稿附件在事件层的 RequestKey 生命周期中保留到请求终态，供远端拒绝时恢复；终态成功后释放，不长期复制到 UI 状态。原始正文仍保留在 text part 中，UI 只对展示副本隐藏可解析的引用标记；模型请求的文本原文不因视觉整理而丢信息。

关键类型建议如下，最终命名可随现有模块风格微调：

```rust
enum UserAttachmentKind { File, Folder, Image }

struct UserAttachmentMeta {
    id: String,
    order: i64,
    kind: UserAttachmentKind,
    label: String,
    marker_range: Option<(usize, usize)>, // 剪贴板独立附件为 None
    mime: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
}

enum ModelItem {
    UserText(String), // 无图旧路径保留，确保请求字节形状不变
    UserContent { text: String, images: Vec<ModelImage> },
    // 其余 assistant/tool 变体沿用现有定义
}
```

`ModelImage` 的运行时字节应以可廉价克隆的共享缓冲承载；持久化记录只保存 MIME、尺寸和 BLOB。传给 transport 前必须完成 BLOB 装载，缺失或损坏即报错，绝不把 `UserContent` 悄悄退化为 `UserText`。

### 3.2 图片格式与资源限额

图片识别以文件头和解码结果为准，不以扩展名作为安全判断。第一帧静态化接受现有签名识别的 PNG/JPEG/GIF/WebP/BMP/TIFF；不支持的格式明确提示。读取原图前用元数据限制单文件 20 MiB；读取期间仍以有界读取阻止 TOCTOU 增长。剪贴板 RGBA 按 `width × height × 4` 受同等原始数据预算控制。解码器另设像素上限，阻止小压缩文件展开为巨量内存；最长边超过 2048 时等比缩放，不放大较小图片。最终编码仅用各协议共同可发送的 PNG/JPEG，MIME 由真实编码结果确定；最终编码若超过同条消息预算，也失败并保留草稿。

单条消息四张、原图总和 40 MiB 的检查在去重后执行；输入的每一个文件仍先受单张 20 MiB 限制。不能在已成功写库后才发现某张图超限。历史回放只读取本次上下文保留的图片；压缩检查点以前的图片留在历史库中，但不进入压缩后的模型请求。按尺寸为图片预留保守 token 预算，轮次裁剪仍以**整轮**为单位，当前图片消息不被静默截断。

## 4. 数据库与迁移

实施前检查 `src/db/mod.rs::SCHEMA_VERSION` 及其它并行分支新增迁移。以当前 v17 为基线，候选版本为 **v18**；若 v18 已被占用，顺延到首个未用版本并保留逐版升级路径。只新增表，不重建旧表；旧会话没有附件记录时按空集合读取，不改原文。

建议将可见引用和图片副本统一存为一张 message 子表，使顺序、删除和图片数据同事务：

```sql
CREATE TABLE agent_message_attachment (
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
    image_bytes BLOB,
    UNIQUE(message_id, position),
    CHECK ((marker_start IS NULL AND marker_end IS NULL) OR
           (marker_start >= 0 AND marker_end > marker_start)),
    CHECK (kind <> 'image' OR
           (mime IN ('image/png', 'image/jpeg') AND image_bytes IS NOT NULL
            AND width > 0 AND height > 0))
);
CREATE INDEX idx_agent_message_attachment_message
    ON agent_message_attachment(message_id, position);
```

`Database::begin_turn_with_attachments` 在现有 `begin_turn` 事务中，写 user text part 之后批量写附件，assistant 占位与 session revision 提交在同一事务；任一附件失败整轮回滚。原 `begin_turn` 可作为空附件委托，避免无关调用点大面积改签名。`load_chat_session` 新增一次批量**仅元数据**查询（不选 `image_bytes`），按 message ID 挂回消息；`load_image_payloads(session_id, attachment_ids)` 用会话归属和 ID 集合读取本轮真正入选的 BLOB，防止跨会话读取。`delete_chat_session`、撤销删除消息和级联删除子会话均由外键清理图片，不留下磁盘孤儿。迁移及读写加入旧库升级、重复打开、外键、空旧会话、图片 BLOB 不随历史元数据加载等测试。

数据库文件的开发/生产/worktree 隔离继续沿用现有 `db` 根，不复制隔离库。迁移只加表，无整表重建；实施时仍需用库副本验证 `user_version`、数据量和 `PRAGMA integrity_check`，再处理真实开发库。设计审核不执行迁移。

## 5. 输入到发送的精确流程

```text
Ctrl+V / Ctrl+Alt+V（剪贴板图片）或图片路径粘贴
  -> event 调用 platform 读取；后台解码/验证，按 pane + generation + 输入修订号回投
  -> app 只接收纯数据附件并更新草稿；失败走 pane.lx.toast
Enter
  -> try_send 冻结 text + draft_images + revision，进入 Preparing
  -> 后台解析有效 @ 引用、读取图片、去重、缩放、执行预算检查
  -> 成功回投后 BeginChatTurn 同事务提交 text + 附件 + assistant 占位
  -> DB ack 后清除匹配的草稿与附件，消息列表只加附件元数据
  -> context::assemble 选出整轮及附件 ID
  -> event 按需装载所选 BLOB，再启动 agent loop / transport
  -> 远端看到 text + 真正的 image 内容
```

剪贴板图片优先文件列表，再读像素数据；与 Codex 一致。`Ctrl+V` 和 `Ctrl+Alt+V` 在 lx Native 输入框中尝试图片；无图片时保留终端原有文本粘贴行为，不插入控制字符。终端已经发出的 `Paste(text)` 仍可处理文本和路径：仅图片路径候选直接形成附件；混合文件/文件夹/图片时保留原有引用/路径面板，选择图片引用后在发送前解析为图片。shell 模式的路径仍是命令字面量，不自动附图。其它窗格与 prompt 右栏不受此快捷键影响。

准备态必须携带 `pane + generation + revision`。用户切换窗格、会话、工作区或继续编辑后，迟到的剪贴板/解码/DB 回执不得注入新草稿；重复 Enter 不发第二轮。已有本地命令运行时的聊天排队若含图片，队列须携带完整冻结附件或明确保持待发草稿；不得只排队文本并丢图。取消准备态时保留未发送的草稿和附件。

失败分三类：

1. **提交前失败**：剪贴板不可用、文件消失、非图片、解码/预算超限、数据库写入失败。保持原始草稿与附件，提示具体可操作原因；不产生半轮消息或无图请求。
2. **提交后、本地请求构造前失败**：已入库图片 BLOB 缺失/损坏或协议编码错误。标记本轮失败并恢复未改动输入框中的冻结草稿/附件，绝不发送缺图内容；已有用户在准备期间输入的新内容不得被覆盖。
3. **远端拒绝图片**：对无法从本地配置可靠判断视觉能力的模型，保留服务器错误并标记本轮失败；在同 pane 且输入未变化时恢复文字和附件供重试。服务端收到请求后失败与本地预检不同，不能承诺数据库中完全没有失败轮次；不允许为了“成功”而重发纯文本。

图片路径和 MIME 不写入日志正文；异常记录只含稳定错误码、附件数量及非敏感尺寸。二进制永不出现在 `AppState.toast`、`tracing` 或 `tool` 回执里。模型可见的结构化说明如需新增一律用英文，用户界面反馈统一从 `ui/text.rs` 取词，并写入 lx 页的 pane toast。

## 6. 模型请求与上下文

`chat::context::assemble` 对无图轮次继续产出 `ModelItem::UserText`，保留既有纯文本请求快照。带图轮次产出单个 `UserContent`，正文先于图片，图片按附件位置顺序；独立附件不伪装成用户键入的路径文本。`HistoryBlock` 增加 user 多模态分支，不拆散到多个轮次，避免工具调用/结果配对和上下文裁剪发生重排。`ChatRequest::turns` 仍供现有纯文本兼容测试，`items` 是带图请求的唯一 wire 来源。

| transport | user content 的目标 wire 形状 |
| --- | --- |
| `openai` / `openai-compatible` | `content: [{"type":"text","text":"..."}, {"type":"image_url","image_url":{"url":"data:image/png;base64,..."}}]` |
| `openai-responses` | `content: [{"type":"input_text","text":"..."}, {"type":"input_image","image_url":"data:image/png;base64,..."}]` |
| `anthropic` | `content: [{"type":"text","text":"..."}, {"type":"image","source":{"type":"base64","media_type":"image/png","data":"..."}}]` |
| `google` | `parts: [{"text":"..."}, {"inlineData":{"mimeType":"image/png","data":"..."}}]` |

图片单独成消息时省略空 text block，至少保留一个 image block。各编码器仅接受已准备好的 PNG/JPEG，使用现有 `base64` 依赖；不能靠 `format!` 拼 JSON 转义。摘要请求与工具调用仍使用现有编码路径，不把图片插进 system/tool result。五种协议的测试分别断言 MIME、base64 解码后的字节、文本顺序、图片单独发送、无图旧请求字节等价；模型明确拒绝图片时测试“报错且不退化为纯文本”。

由于模型视觉能力无法从所有兼容端点的配置准确推断，本地只校验格式与 wire 契约；远端不支持时以真实 API 错误反馈。不会根据模型名称猜“支持图片”。

## 7. 用户消息历史、折叠与命中

原始 user text part 永不因展示而改写。发送前冻结的 `UserAttachmentMeta.marker_range` 仅用于绘制副本：按字节区间移除有效引用标记，并清理标记旁多余空白；无效 `@文字` 原样保留。历史旧消息没有附件元数据时，纯解析器仍能从 `@[refer-*]` 的显式类型和路径生成只读引用行；旧 `@相对路径` 只在会话载入阶段能唯一解析且文件仍存在时生成展示行，不在 `ui/` 执行 IO，也不伪称旧消息保存了图片字节。无法解析的旧标记保留在正文。短引用只有在发送时唯一解析成功才登记；直贴图片附件的行用稳定 `Image N`/原文件名展示，不暴露临时文件路径。

正文按当前历史内容宽度软换行后计视觉行。折叠态行数是 `min(5, wrapped.len())`，若超出五行再加一行 `more lines`；展开态绘制全部正文并提供 `less lines` 收起行。图片区和其它引用行始终跟随正文显示，不计入五行预算；图片单独消息显示简短用户消息占位，随后图片引用行。示意：

```text
> 请检查下面的结果                 <- 原用户消息背景，仅正文
  more lines                         <- 可点，仍属正文块
  ├─ src/main.rs                      <- 无背景，连接符默认色，路径文件色
  └─ screenshots/error.png            <- 无背景，连接符默认色，路径图片色
```

`history::snapshot::Builder` 生成 user 正文、more/less 行及引用行的**唯一行数**，引用与正文之间不额外造一个不受快照管理的空行；消息之间既有空行规则保持。`BlockPaint::UserText` 扩成带引用 span、折叠提示和点击偏移的用户块，`BlockKey` 签名包含消息 ID、内容、引用元数据、宽度和展开态，防止不同宽度/状态复用旧排版。`history::paint` 只按缓存好的可见行绘制：正文块铺 `style::chat_user_bg`，引用行使用默认背景，`├─`/`└─` 默认前景，label 用现有 `style::markdown_refer_{file,folder,image}` 的色相或专用无下划线派生 token；不要把连接符涂成引用色。选中 rewind 用户消息时只变更正文背景，引用仍无底色。

`CardKind`/`CardTarget` 新增 `UserMessage { message_id }` 折叠命中；只在 more/less 行登记可点击区域，按稳定消息 ID 在 `LxChat` 里切换 `expanded_user_messages`。`history_hit.rs` 使用同一快照返回目标；`app/update/chat/viewport.rs` 更新状态并保持滚动位置可解释（展开时锚定该消息可见首行，收起时钳制到底）。引用行不登记点击面；鼠标选择/复制与既有 rewind 双 Esc 定位仍优先按原规则处理。切会话清空瞬时展开集合，重新打开默认折叠。极窄视口需保证提示可见并可点，中文宽字符不能半格溢出。

## 8. 模块改动清单与依赖方向

| 模块 | 实施职责 |
| --- | --- |
| `Cargo.toml` | 评估并加入图像解码/缩放与剪贴板读取依赖；现有 `base64` 复用。锁文件随依赖更新。 |
| `src/platform/` | OS 剪贴板图片读取，返回字节/尺寸或明确错误；各平台 `cfg` 在此收口。 |
| `src/event/paste.rs`, `src/event/chat/` | 图片粘贴与发送前异步准备、按 pane/generation 过滤、图片 BLOB 按需装载、远端失败恢复草稿。 |
| `src/app/state/lx/`, `src/app/update/chat/`, `src/app/effects.rs` | 纯状态的草稿附件、准备态、输入修订、提交/失败转换与用户正文展开态。 |
| `src/app/markdown/` | 复用长引用解析；补充只识别有效 `@` 路径/唯一短引用的纯解析，不做 IO。 |
| `src/db/migration.rs`, `src/db/chat/`, `src/db/service.rs` | v18 候选迁移、附件事务写入、元数据批量读取、BLOB 按需读取与级联删除。 |
| `src/chat/context/`, `src/chat/events.rs`, `src/chat/transport/` | 带图 user item、完整轮次裁剪、四个编码器对五协议的多模态 JSON。 |
| `src/ui/main_content/lx/history/`, `history_hit.rs`, `src/ui/style.rs`, `src/ui/text.rs` | 五行正文、引用树、默认色连接符、无背景引用、缓存签名与点击命中。 |

不得在 `ui/` 读剪贴板、文件或数据库；不得在 `app/` 执行 IO；OS 分支不得散落到 `event/`。单个生产文件达到 1000 行前按现有模块职责拆分，测试文件按被测模块组织，不按行数机械切分。不要修改 TUI 退出/panic 恢复顺序。

## 9. 验证、风险和实施门槛

自动化重点：旧库升版与 BLOB 级联；提交前失败没有半轮；重开会话只载入元数据；源文件删除后回放；纯图片消息；四张与预算边界；同图去重；@ 长引用与相对路径、代码围栏、无效 `@`；中文/窄宽视觉行；展开后的快照行数、滚动/点击/复制；五协议请求 JSON。依现有测试结构增加受影响模块测试，随后按项目要求运行 `cargo fmt`、`cargo clippy --all-targets -- -D warnings`、`cargo test`，报告实际执行范围与结果。涉及 `src/tui/` 恢复路径时另跑 `tui::tests`，本设计不计划触碰它。

主要风险是多图 BLOB 写入造成事件循环卡顿、兼容端点拒绝图片、图像解码峰值内存及旧会话重载性能。实现时图片准备和大 BLOB 读写放后台，历史列表只带元数据；对端能力只能靠服务器反馈。交互与外观由用户在 `cargo run` 中检查真实终端，Agent 侧以纯布局/Buffer 测试守住行数和颜色。

**审核门槛：用户明确审核确认本设计与同目录 `task.md` 后，才开始任何功能代码、依赖、迁移或测试修改。** 文档评审期间只允许继续修订这两份文档；不创建分支、不暂存、不提交、不改数据库。
