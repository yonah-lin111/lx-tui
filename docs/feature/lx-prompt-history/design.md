# lx 输入框历史上下文切换：设计方案

> 状态：**用户已逐项确认边界**（见 §2），待实施。
>
> 任务范围：lx 页输入框的「历史 prompt 切换」。参考实现为 `opencode-dev` 的
> `packages/tui/src/mini/prompt.shared.ts`（`createPromptHistory` / `pushPromptHistory` /
> `movePromptHistory`）与 `packages/tui/src/mini/footer.prompt.tsx`（`move` / `historyCommand`）。
> 只移植用户行为，不引入 opencode 的 TypeScript 运行时、`RunPrompt` 结构或附件模型。

## 1. 背景与目标

当前 lx 输入框（`src/app/state/lx.rs::Lx`）只有多行编辑能力：`↑`/`↓` 在无面板时走
`Lx::move_up` / `Lx::move_down`（按视觉行移光标），发送成功后 `on_turn_committed` 直接
`lx.clear()` 清空草稿，没有任何「翻回上一条提问」的能力。用户想重复或微调上一次的提问时
必须重新输入。

目标：在 lx 输入框里用 `↑`/`↓` 切换历史 prompt，行为对齐 opencode；历史**全局共享**
（跨窗格、跨工作区、跨会话）、**持久化**（重启后仍在）、默认保留 **100** 条。

成功标准：

1. 输入框为空时按 `↑` 立即取得最近一条历史；连按 `↑` 逐条向前，到头钳住不循环。
2. 在历史中途按 `↓` 逐条向后，越过最新一条后**恢复进入浏览前的草稿**。
3. 光标不在文首（`↑`）或不在文末（`↓`）时，`↑`/`↓` 仍是原有的光标移动，不被历史抢占。
4. 任一浮层面板（粘贴参照 / `@` 提及 / `/` 命令 / md 块）打开时，`↑`/`↓` 归面板，历史不接管。
5. 浏览历史期间继续编辑，立即退出浏览态，输入框内容成为新草稿；不会静默回滚。
6. 发送成功的 prompt 写入历史表（去重置顶、上限 100），重启后仍可翻阅；历史写库失败不影响聊天。
7. 全路径 `ui/` 只读状态，DB 只在 `db/`，历史切换是纯状态更新，无 IO。

## 2. 已确认边界

用户逐题确认的决策（问题 → 结论）：

| # | 决策点 | 结论 |
| --- | --- | --- |
| 1 | 数据来源范围 | **全局共享**：跨窗格、跨工作区、跨会话同一份历史；且**持久化**，默认 100 条。 |
| 2 | `↑`/`↓` 接管条件 | 严格照抄 opencode：`↑` 仅当光标字节偏移 `== 0`；`↓` 仅当光标在正文末尾（`cursor == input.len()`）；否则回落原有光标移动。 |
| 3 | 存储位置 | lx-tui SQLite 新增全局单表 `prompt_history`（v10 迁移）；不用配置文件。DB 不可用时降级为纯内存历史。 |
| 4 | 写入时机 | `on_turn_committed`（DB ack 成功）后压独立效应 `RecordPromptHistory`；best-effort，失败只 `tracing::warn`。 |
| 5 | 浏览态中的编辑 | 任何编辑操作立即退出浏览态（`index = null`，丢弃暂存草稿），输入框内容即新草稿。配套：恢复历史压一次 undo 快照；`Esc` 结束浏览但**不改文本**。 |
| 6 | 状态归属 | 条目列表（最近 100 条）放 `AppState` 一份全局；浏览游标 `index` 与"进入浏览前草稿"放各自 `Lx`。 |
| 7 | 重复内容 | **全局去重 + 最近使用置顶**（已存在则更新时间为当前）；这是相对 opencode 的有意偏离，因其有 100 条硬上限。 |
| 8 | 面板优先级 | 面板绝对优先：粘贴参照、`@` 提及（含 `mention_visible`）、`/` 命令、md 块面板任一打开时历史不接管；全部关闭才「历史优先于光标移动」。 |
| 9 | 存储内容 | 存**输入框原文**（`trim` 后的 `Lx.input` 逐字节），`@` 提及/`/` 命令/md 块按原样保留；恢复时光标置于末尾并吸回可见区。单条上限 32 KiB，超限不入历史（仍正常发送）。 |
| 10 | `Esc` 与流中 | `Esc` 仅在浏览态被消费（结束浏览、文本不变）；流中允许浏览但 `Enter` 仍被吞；恢复文本与当前草稿完全相同时不压 undo 快照。 |

## 3. 参考实现的关键行为（已核实）

`opencode-dev/packages/tui/src/mini/prompt.shared.ts`：

- `HISTORY_LIMIT = 200`；`createPromptHistory` 过滤「文本为空且无附件」的条目、折叠**相邻**重复、
  `slice(-HISTORY_LIMIT)`，并置 `index: null, draft: ""`。
- `pushPromptHistory`：空内容不入；与末条完全相同时只重置 `index/draft`，不追加。
- `movePromptHistory(state, dir, text, cursor)`：
  - 无条目 → 不接管；
  - `dir === -1 && cursor !== 0` → 不接管；
  - `dir === 1 && cursor !== width(text)` → 不接管；
  - `index === null`：`dir === 1` 不接管；`dir === -1` 记 `draft = text`、跳到末条、光标置 0；
  - `index + dir` 越界（`< 0`）→ 不接管（钳住）；`>= len` → 恢复 `draft`、`index = null`；
  - 否则移动到该条，`dir === -1` 光标置 0，`dir === 1` 光标置末尾。

`opencode-dev/packages/tui/src/mini/footer.prompt.tsx::historyCommand`：先试 `move`（历史），
不接管再回落「视觉行光标移动」——与 lx 现有 `Lx::move_up/move_down` 的分支语义一致
（首视觉行 → 钳到行首；末视觉行 → 钳到行尾）。

lx 的有意差异（均已在 §2 记录理由）：

| 项 | opencode | lx |
| --- | --- | --- |
| 重复折叠 | 仅相邻重复 | 全局去重 + 置顶（100 条上限下更省槽位） |
| 持久化 | 无（仅当前会话 turns 推导） | SQLite 全局表，重启保留 |
| 浏览态编辑 | 直接覆盖文本，暂存草稿被静默丢弃 | 编辑即退出浏览态，草稿不被静默抹除 |
| 条目结构 | `RunPrompt { text, parts, mode, command }` | 仅 `text`（lx 输入框无附件结构） |

## 4. 数据结构

### 4.1 全局历史（`AppState`）

```rust
/// 全局 prompt 历史（跨窗格/工作区/会话共享，最近使用在末尾）。
pub struct PromptHistory {
    /// 条目正文（输入框原文），最多 `PROMPT_HISTORY_LIMIT` 条。
    entries: Vec<String>,
}
```

对外只暴露只读访问与两个变更入口：

```rust
impl PromptHistory {
    pub fn entries(&self) -> &[String];
    pub fn is_empty(&self) -> bool;
    pub fn len(&self) -> usize;
    /// 取第 `index` 条（越界 None）。
    pub fn get(&self, index: usize) -> Option<&str>;
    /// 记录一条已发送 prompt：去重后置顶，裁剪到上限；空内容忽略。
    pub fn record(&mut self, text: &str) -> bool;
    /// 启动时用 DB 载入的条目整体替换（保持传入顺序，尾部为最新）。
    pub fn replace_all(&mut self, entries: Vec<String>);
}
```

`AppState` 新增字段 `pub prompt_history: PromptHistory`。它不属于任何窗格，也不随工作区切换重置。

### 4.2 每窗格浏览态（`Lx`）

```rust
/// 历史浏览状态：游标 + 进入浏览前的本地草稿。
#[derive(Debug, Default)]
struct HistoryBrowse {
    /// 当前浏览位置（`None` 表示未在浏览）。
    index: Option<usize>,
    /// 进入浏览时暂存的草稿（越过最新一条时恢复）。
    draft: String,
}
```

`Lx` 新增私有字段 `history: HistoryBrowse`。**不放进 `LxChat`**：它属于输入框编辑态，
与聊天请求状态机无关。

### 4.3 提示词历史条目上限与单条上限

```rust
/// 全局保留的历史条数上限。
pub const PROMPT_HISTORY_LIMIT: usize = 100;
/// 单条历史文本字节上限；超限不写入历史（仍正常发送）。
pub const PROMPT_HISTORY_MAX_BYTES: usize = 32 * 1024;
```

## 5. 交互流程

### 5.1 `apply_lx_input` 中的接管点

在 `src/app/update/pane.rs::apply_lx_input` 的现有面板链之后、`match command` 之前插入：

```text
if 无任何面板（paste / mention_open / mention_visible / slash / md panel 全无） {
    match command {
        LxInput::Up   => if try_history(state, pane_id, -1) { return; }
        LxInput::Down => if try_history(state, pane_id,  1) { return; }
        _ => {}
    }
}
```

`try_history` 返回 `false` 时继续走原有 `match`，即回落 `lx.move_up()` / `lx.move_down()`。

`try_history(direction)`：

1. 若 `state.prompt_history` 为空 → `false`。
2. `↑` 且 `lx.cursor != 0` → `false`；`↓` 且 `lx.cursor != lx.input.len()` → `false`。
3. `index == None` 且 `↓` → `false`。
4. `index == None` 且 `↑` → 暂存 `draft = input.clone()`，`index = last`。
5. `index + direction` 越界：
   - `< 0` → `false`（钳住，不循环）；
   - `>= len` → 恢复 `draft`，`index = None`，文本 = `draft`。
6. 否则 `index` 移动，文本 = `entries[index]`。
7. 应用文本到 `Lx`：`apply_history_text(text)`（见 5.2），返回 `true`。

### 5.2 文本应用与光标

```rust
/// 把历史文本应用到输入框：压一次 undo 快照（同文本不压）、光标置末尾、
/// 清空选区、光标跟随滚动、重算面板。
fn apply_history_text(&mut self, text: &str)
```

- 文本与当前 `input` 完全相同时**不压快照、不重算面板**，仅移动光标到末尾（幂等）。
- 否则 `snapshot()` 后整体替换 `input`，`cursor = input.len()`，`text_anchor = None`，
  `scroll_cursor_into_view()`，`refresh_panel()`。
- 不调用 `clear()`：`clear()` 有「空则直接 return」与独立快照语义，不适合此处。

### 5.2.1 光标落点（`cursor_at_start`）

对齐 opencode：`↑` 恢复到某条历史后**光标置于文首**，`↓` 恢复后**光标置于文末**。
这不是细节而是接管条件成立的前提——接管要求「`↑` 时 cursor == 0、`↓` 时 cursor == 文末」，
所以光标落点必须与方向一致，否则连按同方向键第二次就会因光标不在边界而失效。

由此产生一个**必须知晓的交互后果**：按 `↓` 从「较旧条目」回到「较新条目」时，
恢复后光标在文末，因此**继续按 `↓` 能接着前进**；但从未浏览态直接按 `↑`
（光标被置于文首）后想改按 `↓` 回退，需要先把光标移到文末才生效
（单行文本按一次 `↓` 即钳到行尾；多行文本需按到底）。这与 opencode 的
`historyCommand` 完全一致：它先试历史，不接管再走「视觉行光标移动」，
所以「先移动到边界、再切换历史」是正确的两段式行为，不是缺陷。

### 5.3 浏览态退出

- 任何编辑命令（`Char`/`Backspace`/`Delete`/`Newline`/`EnterFolder` 回落换行/`Clear`/
  `Delete*`/`Indent`/`Outdent`/`Undo`/`Redo`）→ 在 `apply_lx_input` 分派前调用
  `lx.end_history_browse()`（清 `index` 与 `draft`，不改文本）。
- `LxInput::Escape` 且处于浏览态 → `end_history_browse()` 并 `return`（消费该键）。
- 面板打开时的 `↑`/`↓` 不进入历史，也不退出浏览态（面板操作不影响浏览游标）。

### 5.4 发送后的清理

`on_turn_committed` 现有 `target.lx.clear()` 保留；额外调用 `target.lx.end_history_browse()`，
避免「发送后仍停在旧浏览游标」，否则下一次 `↑` 会从旧位置继续。

## 6. 持久化

### 6.1 v10 迁移

`SCHEMA_VERSION: i64 = 10`。`src/db/migration.rs` 增加：

```sql
CREATE TABLE IF NOT EXISTS prompt_history (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    text          TEXT NOT NULL UNIQUE,
    updated_at_ms INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_prompt_history_updated
    ON prompt_history(updated_at_ms DESC);
```

迁移分支：`if version < 10 { transaction.execute_batch(SCHEMA_V10)?; }`。
新表无历史数据需要回填；老库升级只新增表，既有表结构不变，迁移可重复打开。

### 6.2 DB API（`src/db/history.rs`）

| 函数 | 语义 |
| --- | --- |
| `load_prompt_history(limit) -> Result<Vec<String>>` | 按 `updated_at_ms DESC, id DESC` 取最多 `limit` 条，返回**旧→新**顺序（尾部最新）。 |
| `record_prompt_history(text, limit) -> Result<()>` | 单事务：`INSERT ... ON CONFLICT(text) DO UPDATE SET updated_at_ms = excluded.updated_at_ms`；再删除超出 `limit` 的最旧行（`id NOT IN (SELECT id ... ORDER BY updated_at_ms DESC LIMIT limit)`）。 |

`text` 的超长/空校验由调用方与 SQL 层共同保证：空串与超 32 KiB 在 app 层就过滤，DB 层再断言空串返回 `DbError::InvalidDraft`。

### 6.3 写回路径

- `Effect::RecordPromptHistory { text: String }` 加入 `src/app/effects.rs`。
- `try_send` 成功后（`BeginChatTurn` 压入处）**不**直接压该效应：正文要等 ack。
- `on_turn_committed` 内压 `Effect::RecordPromptHistory { text: frozen_draft }`，并把
  同一条同步 `state.prompt_history.record(&frozen_draft)` 应用到内存，保证 `↑` 立刻可用，
  不等 DB 回执。
- `src/db/service.rs` 处理该效应：`self.with_db(|db| db.record_prompt_history(&text, LIMIT))`，
  失败仅 `tracing::warn!(%error, "recording prompt history failed")`，返回 `false`（不置脏也可，
  但内存状态已变，返回 `true` 以触发重绘无需）。实现选择返回 `false`：内存变更已在 app 层完成，
  DB 只是持久化副产物。

### 6.4 启动载入

`Persistence::start()` 打开库后（`recover_interrupted_*` 之后）读取最近
`PROMPT_HISTORY_LIMIT` 条，经 `AppState` 初始化路径写入 `prompt_history`。
`AppState` 的构造点在事件层启动流程中（见 `src/event/mod.rs` / `src/main.rs`），实现时把
`Persistence::load_prompt_history()` 的结果在首次构造 `AppState` 后立即 `replace_all`。
无 DB（降级模式）时不载入，历史从空开始，仅内存累积。

## 7. 边界与失败矩阵

| 场景 | 结果 |
| --- | --- |
| 历史为空按 `↑`/`↓` | 不接管，回落光标移动（`↑` 到行首 / `↓` 到行尾）。 |
| 光标在中间按 `↑`/`↓` | 不接管，回落光标移动。 |
| 已到最旧一条继续 `↑` | 钳住不循环，不接管。 |
| 越过最新一条 `↓` | 恢复进入浏览前的草稿。 |
| 浏览中编辑 | 立即退出浏览态，文本为新草稿。 |
| 浏览中 `Esc` | 退出浏览态，文本保持该条历史。 |
| 面板打开时 `↑`/`↓` | 面板消费，历史不接管。 |
| 历史写库失败 | 聊天正常，`tracing::warn`；内存历史仍含该条，重启后可能缺失。 |
| DB 不可用 | 纯内存历史，重启丢失。 |
| 单条超 32 KiB | 不入历史（仍正常发送）。 |
| 重复发送同一句 | 内存去重置顶；DB `ON CONFLICT` 更新时间。 |
| 流中浏览后 `Enter` | `Enter` 走 `try_send`，`request.can_send()` 为 false → 不发送、不报错。 |
| 多窗格并发浏览 | 游标各自独立；列表共享，新发送的条目在所有窗格可见。 |

## 8. 文件归属

| 文件 | 改动 | 边界 |
| --- | --- | --- |
| `src/app/state/prompt_history.rs`（新） | `PromptHistory` 与两个上限常量 | 纯数据，无 IO、不依赖 ui/db |
| `src/app/state/lx/history.rs`（新） | `HistoryBrowse` + `decide` / `HistoryMove` | 纯状态机 |
| `src/app/state/lx.rs` | 新增 `history` 字段与转发方法 | 保持编辑器职责，不掺 DB |
| `src/app/state.rs` | `AppState.prompt_history` 字段 | 唯一状态定义处 |
| `src/app/update/pane.rs` | `lx_history_route` 前置路由 + `edits_text` 判定 | 保持 `ui/` 不参与 |
| `src/app/update/chat/mod.rs` | `on_turn_committed` 记录历史 + 清浏览态 | 不新增 IO |
| `src/app/effects.rs` | `RecordPromptHistory` 效应 | 纯数据 |
| `src/db/history.rs`（新） | 两个 DB API + 计数 | 唯一 rusqlite 使用点之一 |
| `src/db/mod.rs`、`src/db/migration.rs` | `SCHEMA_VERSION = 10` + v10 表 | 事务迁移，可重复打开 |
| `src/db/service.rs` | 效应出箱 + `load_prompt_history` | 不绕过事件循环 |
| `src/main.rs` | 启动后载入历史 | 库不可用时静默降级 |
| `src/input/mod.rs` | 无改动（`Up`/`Down` 已映射） | — |
| `src/ui/` | **不改渲染**：历史切换不产生新视觉元素 | 只读状态 |

## 9. 验证与交付

1. 单元测试（`app`）：
   - `PromptHistory::record` 的去重、置顶、上限裁剪、空/超长过滤、`replace_all` 顺序。
   - `Lx` 历史游标：首按 `↑`、连续 `↑` 钳制、`↓` 越过末尾恢复草稿、
     光标非边界时不接管、浏览中编辑退出、`Esc` 退出不改文本、
     同文本不压 undo 快照、恢复后可 `Ctrl+Z` 回到上一份文本。
2. `apply_lx_input` 集成测试：`@`/`/` 面板打开时 `↑`/`↓` 归面板；
   无面板时 `↑`/`↓` 走历史或光标移动；多窗格游标独立而条目共享。
3. DB 测试：v9 → v10 升级、重复打开、`record_prompt_history` 去重置顶与裁剪、
   `load_prompt_history` 顺序、空串/超长拒绝。
4. 服务/事件测试：`RecordPromptHistory` 效应落盘、启动载入、降级模式、
   `on_turn_committed` 记录历史与退出浏览态、`on_turn_failed` 不记录。
5. `cargo fmt`、`cargo clippy --all-targets -- -D warnings`、`cargo test` 全绿。
6. 用户运行 `cargo run` 人工验收交互与观感。

## 10. 实施记录

已按本设计实施，与设计一致的要点不再重复；以下是实施中发现的、需要说明的差异与决定：

1. **光标落点补充为显式字段**：设计初稿只在 `apply_history_text` 里把光标置末尾，
   这会让「连续按 `↑`」在第二次失效（接管条件要求 cursor == 0）。实施中把
   「光标落在文首/文末」变成 `HistoryMove::Entry { cursor_at_start }` 显式携带，
   由 `decide` 按方向决定：`↑` 落文首、`↓` 落文末。这与 opencode `movePromptHistory`
   的 `cursor: dir === -1 ? 0 : stringWidth(text)` 完全一致，设计 §5.2.1 已补齐说明。

2. **DB 去重改为「先删后插」而非 `ON CONFLICT DO UPDATE`**：`updated_at_ms` 只有毫秒
   精度，快速连续记录会撞同一时刻，导致「最近使用置顶」在载入时顺序错误。
   改为在同一事务内 `DELETE WHERE text = ?` + `INSERT`，让单调递增的 `id` 成为
   同毫秒内的排序决胜键（载入与裁剪均按 `updated_at_ms DESC, id DESC`）。

3. **浏览态退出只由「改写文本」的命令触发**：`Left`/`Right`/`Home`/`End`/词移动、
   `Tab`、`Enter` 都不结束浏览态；只有 `Char`/`Backspace`/`Delete*`/`Newline`/
   `EnterFolder`/`Undo`/`Redo`/`Clear` 触发（`edits_text`）。这样「先移动光标再翻页」
   仍然连贯，符合 §5.2.1 的两段式行为。

4. **`Esc` 只在浏览态被消费**：`lx_history_route` 在 `lx.history_browsing()` 为真时
   返回 `EndBrowse` 并 `return`，其余情况下 `Esc` 完全走原有 `escape_stop` 语义，
   未修改任何既有停止/待命行为。

5. **面板占用判定集中在 `lx_history_route`**：与 `apply_lx_input` 内部的面板链使用
   同一组判定（paste / `mention_open` / `mention_visible` / `slash` / `panel`），
   在函数入口处一次性拒绝，避免面板抢键失败后被历史接走。

6. **未改 `src/ui/` 与 `src/input/`**：`Up`/`Down` 早已映射为 `LxInput::Up/Down`，
   历史切换不产生任何新视觉元素，因此渲染层零改动。
