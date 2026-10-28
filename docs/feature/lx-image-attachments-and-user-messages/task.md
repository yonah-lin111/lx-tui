# lx 图片附件与用户消息交互任务清单

> 状态：待用户审核；以下均未执行。只有用户明确审核确认本文件及 `design.md` 后，才修改功能代码。
> 执行规则：不创建/切换分支，不暂存、不提交、不修改 Git 状态；每阶段先核对工作区原有改动，不覆盖用户修改。

## 0. 审核门槛与基线

- [ ] 用户明确确认 `design.md` 和本任务清单。
- [ ] 再读四份项目规范：`project-directory-structure.md`、`code-writing-standards.md`、`tui-design-requirements.md`、`html-assets.md`；核对本次新增约束是否与规范冲突。
- [ ] 用 codegraph 与 codebase-memory 复查即将修改的符号及调用链，逐个文件查索引覆盖；若有新增代码或索引落后，只对受影响文件刷新或读源码确认。
- [ ] `rtk git status --short` 只读记录现有改动；不清理、不覆盖、不触碰无关文件。
- [ ] 核对 `src/db/mod.rs::SCHEMA_VERSION` 和其它分支迁移。当前设计基线为 17；若 v18 已占用，顺延且保留旧库逐级升级。对迁移先做库副本 `user_version`、数据量与 `PRAGMA integrity_check` 验证。

## 1. 图片入口与草稿状态

- [ ] 评估 Codex 已用的 `arboard`、`image` API 与 lx 现有依赖；第三方资料先查 context7，再决定 feature flags。将 OS 剪贴板读取封装进 `src/platform/`，核心状态不出现平台 `cfg`。
- [ ] 为 lx 输入草稿加入独立图片附件状态（稳定草稿 ID、来源、展示名、字节、顺序）和输入修订号。图片占位与附件必须同生同灭，删除/撤销/重做不能留下文字占位或孤立字节。
- [ ] 在 lx Native 输入下接入 `Ctrl+V` / `Ctrl+Alt+V` 的图片读取：剪贴板图片文件优先，像素数据其次；无图不插入控制字符，文字括号粘贴照常工作。shell 模式、prompt 右栏与终端窗格保持原快捷键路由。
- [ ] 图片路径粘贴：仅图片路径候选直接成为附件；文件夹、普通文件及混合候选仍遵循现有引用/路径面板。有效 `@[refer-image]`、文件提及 `@path` 在发送准备阶段解析，不把所有带图片扩展名的普通文字视为附件。
- [ ] 图片准备在后台完成，按 pane、generation 和输入修订号回投；工作区/会话切换或继续编辑后的旧结果丢弃。失败在 `pane.lx.toast` 呈现，草稿保持可修正。

**验收测试**：剪贴板文件优先、像素数据回退、连续粘贴四张、第五张拒绝、空图片剪贴板不污染文字、粘贴路径与 shell 字面量互不串台、过期回执不进别的 pane。需要桌面剪贴板的测试使用注入接口和内存假实现，不依赖测试机 GUI。

## 2. 引用解析、格式及限额

- [ ] 复用 `app::markdown::parse_references` 提取 `@[refer-file/folder/image](path)`，并为 lx 的 `@相对路径` 设计纯解析区间；跳过代码围栏、坏标记与普通 `@文字`。`@ref/` 只有能唯一反解到本条消息已有完整引用时才作为引用。
- [ ] 冻结工作区/窗格路径基准后在 `event/` 后台检查文件，确定 file/folder/image 类型；只对图片读取和解码，普通文件与目录不读正文。路径转义继续用 `app::markdown::refer` 的规则，不能自写分隔符解析。
- [ ] 同图去重：规范化路径相同或图片字节相同的引用与直接附件只生成一份模型图片；显示引用的行序和文本位置仍稳定。缺失的**显式图片引用**拒绝整轮提交，草稿与已粘图片保持原样。
- [ ] 原图读取上限 20 MiB/张、四张/消息、原图总量 40 MiB；读期间二次检查，阻止元数据与内容竞争。解码像素数单独限制，最长边等比缩至 2048；最终 PNG/JPEG 及 MIME 对应真实编码。动态/未知格式给明确错误，不发送部分图片。

**关键代码契约**：准备函数返回 `Result<PreparedUserMessage, ImagePrepareError>`；成功对象同时包含原文、可见引用元数据与完整图片 payload。错误对象给稳定枚举/代码，用户文案在 `ui/text.rs` 映射，不把路径、原图字节或密钥写进错误正文。

**验收测试**：20 MiB 边界、四张/40 MiB 边界、恶意小文件巨大尺寸、文件消失、非图片扩展名但有效签名、假图片扩展名但无效签名、相同图片多入口、含空格/括号/中文路径、代码围栏里的假引用、只有图片没有文字。

## 3. 迁移与原子提交

- [ ] 在 `src/db/migration.rs` 增加下一个未占用版本的 `agent_message_attachment` 表：消息外键 `ON DELETE CASCADE`、`(message_id, position)` 唯一约束、类型/MIME/尺寸/BLOB 约束和索引。准确更新 `SCHEMA_VERSION`，版本号不可复用。
- [ ] `Database::begin_turn_with_attachments` 将 user text、附件行、streaming assistant、session revision 和 rewind scope 放进现有单事务。保留现有纯文本 `begin_turn` 路径的兼容入口；允许图片单独发送，文本与附件同时空才拒绝。
- [ ] `load_chat_session` 增加一次按会话批量查询附件**元数据**，不选择 BLOB。`chat_message_views_to_db` 与 `view_message_of` 往返稳定 ID、序号、标记范围、类型与尺寸；旧消息无附件时行为不变。
- [ ] 增加按 session 与附件 ID 过滤的图片字节读取方法；历史列表、侧栏和纯 UI 渲染不得加载 BLOB。删除会话、删除子会话、撤销删轮次依靠外键级联释放 BLOB，不留下 lx 管理的外部文件。
- [ ] 对大图片写库和读取制定后台执行路径，保证事件循环不因最多 40 MiB 的 BLOB 长时间阻塞；跨连接访问保持现有 session guard/SQLite 事务序列语义。失败不允许半轮落库。

**关键 SQL 验证**：旧 v17 库与空库升级到新版本，重复打开版本不推进；`PRAGMA foreign_key_check` 与 `PRAGMA integrity_check` 均成功；在用户消息写入过程中注入附件失败，确认 user/assistant/session revision 不留下半成品；删除消息/会话后 `agent_message_attachment` 行数归零。

## 4. 发送状态机与失败恢复

- [ ] `try_send` 对图片消息进入 `Preparing`，冻结文字、图片 ID 与输入修订号；重复 Enter/取消不启动第二轮。所有图片校验成功后才产生 `BeginChatTurn`，DB ack 后才清空匹配草稿。
- [ ] 为排队中的普通聊天携带图片附件，不允许已有 user-shell 队列只保留文字；队列分派时仍按生成号和会话 guard 校验。
- [ ] DB ack 后用户消息视图同时携带附件元数据；启动请求前若图片载入/编码失败，标记当前轮失败并恢复未改动的草稿及附件。不能覆盖用户随后输入的新草稿。
- [ ] 事件层按 RequestKey 保留本轮冻结的图片草稿快照至请求终态，供本地编码失败或远端拒绝时恢复；成功、取消或旧 key 失效时释放，不把 BLOB 留在历史 UI 状态。带图片的已发送轮次不能仅向现有纯文本 prompt history 记录文字，避免按 `↑` 召回一条缺图草稿；本批次应扩展召回元数据，或跳过该轮的纯文本历史登记并以 `/resume` 访问完整消息。
- [ ] 对远端“不支持图片”的真实错误保留失败消息与错误码，在可安全定位的同一 pane 恢复原草稿/附件；不退化为纯文本重试，不伪称远端从未收到请求。
- [ ] 发送图片后切模型、切会话、停止流、关闭 pane 的迟到回执都按 RequestKey 丢弃；`pane.lx.toast` 反馈不走全局 toast。

**验收测试**：准备失败零 DB 记录、DB 失败文本与附件保留、图片单独消息、生成中重复 Enter、旧回执不写新草稿、远端 400 失败不形成缺图成功轮、恢复草稿不覆盖用户新输入、shell 队列含图片不丢失。

## 5. 上下文与五协议编码

- [ ] `ModelItem` 和 `HistoryBlock` 增加带图片的单个 user 内容变体；无图片继续沿用 `UserText`，保持现有纯文本 wire 测试字节等价。
- [ ] `context::assemble` 将图片保存在所属完整轮次，使用保守图片 token 估算；旧轮裁剪、compact 检查点与当前轮保留规则不变。只收集选中轮次的图片 ID。
- [ ] `event/chat` 在启动 agent loop 前按需从 DB 装载选中 BLOB；缺失或不匹配 MIME/尺寸时本地报错并停止 HTTP，不能发送只含文字的退化消息。多步采样复用已装载的共享字节缓冲，不反复查库。
- [ ] 四个构造器分别编码五协议：OpenAI Chat/compatible `image_url`、Responses `input_image`、Anthropic `image/source`、Google `inlineData`。图片独立消息省略空 text block；JSON 由 `serde_json` 结构化生成，base64 复用现有依赖。
- [ ] 不向 system、assistant、工具调用或工具结果附图；模型可见新增文本一律英文，不主动改变 lx 身份提示或 `PROMPT_VERSION`，除非实现确实修改系统提示。

**验收测试**：五种 transport 每种都断言文本与图片顺序、MIME、base64 解码后字节一致、只有图片、历史回放、无图旧快照；兼容接口拒绝图片时明确失败。上下文测试覆盖四图预算、压缩前图片仍在历史但不进请求、当前轮超预算不静默丢图。

## 6. 用户历史绘制与点击

- [ ] 消息正文的展示副本只移除有效引用标记；不改 text part、模型正文、prompt 历史、复制所依据的原始消息内容。无附件元数据的旧消息仍纯解析显式 `@[refer-*]`；旧 `@相对路径` 只在载入阶段能唯一解析且文件存在时显示为引用，无法解析时保留原文，绝不在渲染中访问文件系统。
- [ ] 在 `history::snapshot::Builder` 中按实际内容宽度换行，正文折叠显示前五视觉行，溢出才加 `more lines`；展开显示全部正文和 `less lines`。引用行始终显示，不计入五行；消息之间既有空行仍只由 snapshot 决定。
- [ ] `BlockPaint::UserText`、`BlockKey` 签名与 `paint` 行数同源：正文铺 `chat_user_bg`，引用行不铺底色，`├─`/`└─` 用默认前景，引用 label 使用 file/folder/image 色。不能把整块用户底色延伸到引用行。
- [ ] `history::cards::CardKind`、`app::state::CardTarget`、`history_hit.rs` 和 `toggle_lx_card` 增加仅 more/less 行可点的 user 消息目标，按消息 ID 保存瞬时展开集合；引用行不打开文件、不预览图、不注册点击面。
- [ ] 变宽、滚轮、滚动条、置顶用户消息、rewind 选中、鼠标文本选择/复制与极窄/极矮视口下，行数与命中均从同一快照取。切会话清空展开集合，默认折叠。

**验收测试**：5 行不显示提示，6 行显示提示；自然换行使 6 行也能折叠；中文双宽字符与零宽边界；两条引用树枝末项正确；引用行始终可见且背景为默认；连接符默认色与 file/folder/image 三种颜色；点击提示展开/收起，点击引用无动作；滚动与点击坐标不偏移；旧消息无引用行为不变。

## 7. 精确校验与用户验收

- [ ] 在每一层修改后先运行对应的定向测试：`platform` 图片读取/解码、`app::markdown` 引用、`db::chat` 迁移/事务、`chat::context`、四个 transport、`ui::main_content::lx::history` 和 `event::chat`。按项目规范最后执行 `cargo fmt`、`cargo clippy --all-targets -- -D warnings`、`cargo test`；不为了省时间跳过与本次广泛数据流相关的门槛。
- [ ] `rtk git diff --check` 只读检查空白错误；用 codegraph/codebase-memory 再核对新调用路径和索引覆盖。记录实际测试命令、通过数量和未运行项，不声称执行了人工 UI 验证。
- [ ] 用户运行 `cargo run` 人工检查：截图粘贴、复制图片文件粘贴、图片路径、图片单独发送、文件删除后重进会话、不同模型、五行展开/收起、引用三色及默认色连接符、引用行无深色背景、窄窗格滚动。外观或交互问题按反馈修订，不修改 Git 状态。

## 8. 实施完成判据

功能代码、迁移、测试和文档与设计一致；所有五种协议发送真实图片，提交失败不丢草稿，源文件删除不丢历史图，用户消息正文折叠与引用行布局精确；受影响测试和项目验证通过，已报告人工 TUI 验收仍需用户执行。不得以“仅显示图片路径”“仅支持一个协议”“只改前端占位”视为完成。
