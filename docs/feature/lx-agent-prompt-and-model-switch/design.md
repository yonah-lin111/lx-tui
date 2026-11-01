# lx Agent 系统提示词（AGENTS.md）注入与模型切换逻辑 — 设计文档

> 状态：**已完成并合并入 `dev`**（P0/P1/P2 共 11 项评审问题已全部修复并锁死回归测试）（2026-11-01）
> 参考实现：
> - `/Users/yonah/projects/agent/codex-main`（`codex-rs/codex-home/src/instructions/mod.rs`、`codex-rs/core/src/agents_md.rs`、`codex-rs/core/src/context/user_instructions.rs`）
> - `/Users/yonah/projects/agent/opencode-dev`（`packages/core/src/plugin/identity.ts`、`packages/core/src/session/instruction-state.ts`、`packages/core/src/session/runner/to-llm-message.ts`、`packages/tui/src/util/model.ts`）
> 执行任务清单见 [task.md](task.md)。

---

## 0. 现状核实（写设计前已逐项确认）

| 事实 | 现有代码位置 | 对本设计的影响 |
|---|---|---|
| `src/chat/agent.rs` 目前为单文件（795 行），包含 `AgentRole`、`ToolSurface`、子 Agent 工具入参/汇报视图与 `compose_subagent_prompt` | `src/chat/agent.rs:1-795` | 继续追加系统提示词加载与模型切换逻辑将超过单文件 1000 行红线；必须将 `src/chat/agent.rs` 拆分为 `src/chat/agent/` 目录子模块（`mod.rs`、`instructions.rs`、`model_switch.rs`），测试同步迁入子模块 |
| `PromptParts` 目前仅含 `environment`、`skills`、`mcp`、`compact` 四段，`PROMPT_VERSION = "lx-7"` | `src/chat/prompt.rs:28, 381-434` | 需扩展 `PromptParts` 支持 `model_identity`（`# Your Model`）与 `instructions`（`AGENTS.md` 指令段），并将 `PROMPT_VERSION` 递增为 `"lx-8"` |
| 系统提示发送侧通过 `SystemPromptParts::split` 切分：首段 ≤ 1900 UTF-16 码元进原生 `system` 槽，其余规则拼入首条 `user` 消息的 `<system-instructions>` 包裹 | `src/chat/transport/mod.rs:35-122` | `model_identity` 与 `instructions` 放在 `LX_SYSTEM_PROMPT` 之后的 `tail` 段中，既保证首段 `LX_SYSTEM_PROMPT` 恒定不变（命中上游 Prompt Cache、不触发 9router 2000 码元替换阈值），又能对 XML 元字符自动转义 |
| 数据根目录由 `config::user_data_root()` 唯一推导：生产 `~/.lx-t`，开发 `~/.lx-t-dev` | `src/config/mod.rs:165-172` | 全局 `AGENTS.md` / `agent.md` 存放在 `~/.lx-t[-dev]/` 下；启动或首次读取时若不存在候选文件，自动初始化默认空模板 `~/.lx-t[-dev]/AGENTS.md` |
| 切换模型入口 `finish_model_selection` 目前只更新 `pane.lx.chat.selection` 并入队 `Effect::SetChatSessionModel` 更新 `agent_session` 三列，不在时间线留痕 | `src/app/update/chat/session.rs:637-667`、`src/db/service.rs:1054-1061` | 已有消息的会话在切换模型/思考等级（且发生实质变化）时，需原子写入模型切换时间线记录并生成提示词注入快照 |
| 数据库当前 `SCHEMA_VERSION = 21`，时间线序号 `sequence` 由 `agent_message`、`agent_session_notice`、`agent_compact_checkpoint` 联合取 `MAX(sequence) + 1` | `src/db/mod.rs:27`、`src/db/chat/compact.rs:172-182` | 新增 v22 迁移创建 `agent_model_switch` 表，纳入同一 `sequence` 编号空间，支持 `/resume` 完整恢复与连续切换合并 |
| `compact` 检查点渲染在 `history/compact.rs`，使用 `-- Context compacted --`（`style::accent()`）+ 描述行（`style::muted()`）+ 展开详情，但展开内容尚未带思考块同款 `  └─ ` 树形前缀 | `src/ui/main_content/lx/history/compact.rs:19-94`、`paint.rs:515-556` | 新增 `history/model_switch.rs`，并与 `history/compact.rs` 同步对齐思考块同款的 `  └─ ` 树形连接符 + 压暗 Markdown 渲染（`markdown_render::render_wrapped_literal_rules` + `span_style.patch(style::muted())`） |

---

## 1. 已确认决策汇总（Grill Me 收口）

| 编号 | 决策维度 | 确认结论 |
|---|---|---|
| D1 | `AGENTS.md` / `agent.md` 发现范围与命名 | 双层发现：① 全局用户层 `~/.lx-t[-dev]/`（若不存在任何候选文件则自动创建带英文注释头部的 `~/.lx-t[-dev]/AGENTS.md`）；② 项目层（仅当工作区 `is_trusted` 为真时）从 `project_root`（Git 主仓库根或工作区根）向下到当前窗格 `cwd` 逐层扫描。每个目录内按优先级 `AGENTS.override.md` > `AGENTS.md` > `agent.md`（大小写不敏感匹配 `agents.md` / `agent.md`）取第一个非空普通文件。项目层共享 `32 KiB`（`MAX_PROJECT_DOC_BYTES = 32 * 1024`）字节预算，全局层独立限长 `16 KiB` 不占项目预算。 |
| D2 | `AGENTS.md` 格式化与注入位置 | 严格对齐 Codex：全局层在前、项目层（从 root 到 cwd）在后，两者之间以 `\n\n--- project-doc ---\n\n` 分隔；外层包裹为 `# AGENTS.md instructions for <cwd>\n\n<INSTRUCTIONS>\n...\n</INSTRUCTIONS>`（仅全局指令时首行为 `# AGENTS.md instructions`）。通过 `PromptParts.instructions` 注入系统提示词 `tail` 段。 |
| D3 | 模型切换时间线展示与提示词注入 | 在已有消息的会话中切换模型或思考等级（且与原值不同）时，在消息列表追加持久化切换条目，并在后续轮次对应时间线位置向模型注入 `<system-update>`（含新模型身份声明 `# Your Model`、接管说明及变更后的指令源如 `core/model`、`core/agents_md`）。空白未发消息的会话切模型只改底栏状态，不在时间线产生切换卡片。 |
| D4 | 模型切换与 `compact` 的 TUI 渲染格式 | 对齐 `lx` 的 `compact` 格式并统一采用思考块同款的压暗 Markdown 渲染：<br>- 第 1 行（`style::accent()`）：`-- Model switched --`（连续切换合并为 `-- Model switched xN --`）<br>- 第 2 行（`style::muted()`）：跨模型为 `model: <Display Name> (<variant>) · <HH:MM> · instructions: <sources>`，仅切等级为 `variant: <variant> (<Display Name>) · <HH:MM>`<br>- 点击展开态：使用思考块同款的 `  └─ ` 树形连接符（`style::chat_notice_connector()`，续行缩进 `thinking_body_indent(width)`）+ 压暗 Markdown（`markdown_render::render_wrapped_literal_rules` + `span_style.patch(style::muted())`）渲染切换前后对比与注入的系统提示词内容；`compact` 展开态同步升级为同一套 `  └─ ` + 压暗 Markdown 树形布局。 |
| D5 | 跨模型兼容清洗与子 Agent 继承 | 保持 `AGENT_NAME = "lx"` 与 `LX_SYSTEM_PROMPT` 不可变；每次请求在 `PromptParts` 注入 `# Your Model` 身份段；跨模型切换后历史回放自动剥离非当前模型的 `reasoning_metadata`，若新模型 `supports_image_input == false` 自动将历史图片降级为英文文本占位；子 Agent（`explorer` / `executor`）同步继承 `# Your Model` 身份段、`AGENTS.md` 指令段与父会话最新模型快照。 |

---

## 2. 模块拆分与目录结构

将原 `src/chat/agent.rs`（795 行）重构为 `src/chat/agent/` 目录模块，保持对外 `crate::chat::agent::*` 导出完全兼容，且每个生产文件均低于 1000 行红线（细分文件约 125～440 行，`mod.rs` 收敛在 650 行左右）：

```text
src/
  chat/
    agent/
      mod.rs                子 Agent 角色（AgentRole）、工具面（ToolSurface）、子 Agent 协议结构与 compose_subagent_prompt（~650 行）
      instructions.rs       AGENTS.md / agent.md 双层发现、默认模板初始化、预算截断与 <INSTRUCTIONS> 格式化（~280 行）
      model_switch.rs       # Your Model 身份段生成、模型切换对比计算、指令源指纹对比与 <system-update> 注入构建（~125 行）
      parse.rs              子 agent 控制工具参数白名单解析与校验（~440 行）
      report.rs             子 agent 汇报视图与 <subagent_report> XML 信封序列化（~160 行）
      tests/
        mod.rs              原 agent/tests.rs 用例 + 子模块测试入口
        instructions.rs     AGENTS.md 发现优先级、信任门控、32 KiB 预算截断、自动创建 ~/.lx-t[-dev]/AGENTS.md 测试
        model_switch.rs     模型切换标签、连续切换合并、# Your Model 与 <system-update> 注入测试
    prompt.rs               扩展 PromptParts { model_identity, instructions, ... }，PROMPT_VERSION 升至 "lx-8"
    context/
      mod.rs                AssemblyInput 接入 model_switches 与 instructions，历史回放注入 <system-update> 并执行跨模型清洗
  db/
    migration.rs            新增 v22 迁移：创建 agent_model_switch 表（与 agent_message / notice / compact 共用 sequence）
    chat/
      model_switch.rs       新：ModelSwitchRecord / ModelSwitchCommit 的持久化读写与连续切换合并查询
      mod.rs                联合 sequence 取号纳入 agent_model_switch
    service/
      chat.rs               聊天会话/历史/模型切换/压缩等 Effect 处理；Effect::SetChatSessionModel 用窗格真实 cwd 重新渲染注入提示词
  app/
    state/lx/
      model_switch.rs       新：ModelSwitchNoticeView 与 LxModelSwitch（列表 + expanded 折叠态集合 + is_range_expanded）
    update/chat/
      model_switch.rs       set_switches 回填切换列表、toggle_model_switch 切换展开态
      session.rs            finish_model_selection 识别会话内切换并更新内存/持久化
      viewport.rs           toggle_lx_card 支持 CardKind::ModelSwitch 展开/折叠
  ui/
    text.rs                 新增模型切换相关英文文案常量与格式化函数
    main_content/lx/
      history/
        compact.rs          升级展开态为思考块同款 `  └─ ` 树形前缀 + 5 列缩进 + 压暗 Markdown 渲染
        model_switch.rs     新：`-- Model switched --` 折叠/展开物理行计算与绘制（区间切片 + 树形压暗 Markdown）
        timeline.rs         TimelineEntry 新增 ModelSwitch { first, last, count }，按 sequence 升序合并连续模型切换
        snapshot.rs         SnapEntryKind / BlockSource 接入 ModelSwitch，按区间判定展开态并登记 CardKind::ModelSwitch 点击面
        paint.rs            draw_legacy 分派 BlockSource::ModelSwitch（传入 switches[first..=last] 切片）
```

---

## 3. 核心设计详解与关键代码

### 3.1 `AGENTS.md` / `agent.md` 双层发现与系统提示词注入（`src/chat/agent/instructions.rs`）

#### 3.1.1 候选文件名与默认文件初始化
- 候选文件名优先级常量：
  ```rust
  /// 目录内优先探测的本地覆盖指令文件名（对齐 Codex）。
  pub const LOCAL_AGENTS_MD_FILENAME: &str = "AGENTS.override.md";
  /// 标准指令文件名。
  pub const DEFAULT_AGENTS_MD_FILENAME: &str = "AGENTS.md";
  /// 兼容单数/小写习惯的候选文件名。
  pub const CANDIDATE_AGENT_FILENAMES: [&str; 4] = [
      "AGENTS.override.md",
      "AGENTS.md",
      "agent.md",
      "agents.md",
  ];
  /// 项目级文档共享字节预算上限（32 KiB，对齐 Codex `DEFAULT_PROJECT_DOC_MAX_BYTES`）。
  pub const MAX_PROJECT_DOC_BYTES: usize = 32 * 1024;
  /// 全局用户级文档字节预算上限（16 KiB，独立于项目预算）。
  pub const MAX_GLOBAL_DOC_BYTES: usize = 16 * 1024;
  /// 全局與项目文档之间的分隔符（对齐 Codex `AGENTS_MD_SEPARATOR`）。
  pub const AGENTS_MD_SEPARATOR: &str = "\n\n--- project-doc ---\n\n";
  ```
- **`~/.lx-t[-dev]/AGENTS.md` 自动初始化**：
  - 在 `ensure_global_agents_md(user_root: &Path)` 中，检查 `user_root`（即 `~/.lx-t` 或 `~/.lx-t-dev`）下是否已存在 `CANDIDATE_AGENT_FILENAMES` 中的任意文件（通过大小写不敏感匹配处理 macOS/Linux 文件系统差异）。
  - 若均不存在，原子创建 `user_root.join("AGENTS.md")`（空内容或仅含注释，读取时 `trim()` 后若为空则不注入额外 token，既方便用户随时在 `~/.lx-t[-dev]/AGENTS.md` 填写全局系统提示词，又不在未填写时浪费上下文预算）。

#### 3.1.2 目录链解析（`project_root -> cwd`）与安全门控
- **未信任工作区门控**：若 `workspace_trusted == false`，跳过项目级目录扫描，仅加载全局 `~/.lx-t[-dev]` 指令。
- **目录链构造**：
  - 给定 `project_root: Option<&Path>`（Git 主仓库根或工作区根）与 `cwd: Option<&Path>`：
  - 若 `cwd` 是 `project_root` 的子目录，则构造从 `project_root` 到 `cwd`（含两端）的有序祖先目录列表 `search_dirs`；若无 `project_root` 或 `cwd` 不在其下，则只扫描 `[cwd]`。
  - 每个目录内按 `CANDIDATE_AGENT_FILENAMES` 顺序查找第一个满足 `metadata.is_file()` 且读取后 `trim()` 非空的文件，命中即停止该目录的后续文件名探测。

#### 3.1.3 关键代码实现（`src/chat/agent/instructions.rs`）

```rust
use std::path::{Path, PathBuf};

/// 已加载的 AGENTS.md 指令快照（turn 起点冻结）。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LoadedInstructions {
    /// 全局指令路径（若命中且非空）。
    pub global_source: Option<PathBuf>,
    /// 项目级命中的指令文件路径列表（从 project_root 到 cwd 顺序）。
    pub project_sources: Vec<PathBuf>,
    /// 项目当前工作目录（用于标题行 `# AGENTS.md instructions for <directory>`）。
    pub directory: Option<PathBuf>,
    /// 合并后的内部指令正文（未包裹外层标签）。
    pub body: String,
}

impl LoadedInstructions {
    /// 是否没有任何有效指令。
    pub fn is_empty(&self) -> bool {
        self.body.trim().is_empty()
    }

    /// 简短来源摘要（用于模型切换展开详情与指令指纹对比）。
    pub fn source_labels(&self) -> Vec<String> {
        let mut labels = Vec::new();
        if let Some(path) = &self.global_source {
            labels.push(format!("global ({})", path.display()));
        }
        for path in &self.project_sources {
            labels.push(path.display().to_string());
        }
        labels
    }

    /// 渲染为注入系统提示词的完整英文包裹段落（对齐 Codex `UserInstructions`）。
    pub fn render_prompt_section(&self) -> Option<String> {
        if self.is_empty() {
            return None;
        }
        let header = match &self.directory {
            Some(dir) if !self.project_sources.is_empty() => {
                format!("# AGENTS.md instructions for {}", dir.display())
            }
            _ => "# AGENTS.md instructions".to_string(),
        };
        Some(format!(
            "{header}\n\n<INSTRUCTIONS>\n{}\n</INSTRUCTIONS>",
            self.body.trim()
        ))
    }
}

/// 确保用户数据根（`~/.lx-t` 或 `~/.lx-t-dev`）下存在 `AGENTS.md` 文件。
/// 若已存在 `AGENTS.override.md` / `AGENTS.md` / `agent.md` 则不覆盖。
pub fn ensure_global_agents_md(user_root: &Path) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(user_root)?;
    if let Some(existing) = find_candidate_in_dir(user_root) {
        return Ok(existing);
    }
    let target = user_root.join(DEFAULT_AGENTS_MD_FILENAME);
    if !target.exists() {
        crate::platform::write_file_atomic(&target, b"")?;
    }
    Ok(target)
}

/// 加载并合并全局 `~/.lx-t[-dev]` 与项目级（`project_root -> cwd`）的 `AGENTS.md` 指令。
pub fn load_instructions(
    user_root: Option<&Path>,
    project_root: Option<&Path>,
    cwd: Option<&Path>,
    workspace_trusted: bool,
) -> LoadedInstructions {
    let mut global_source = None;
    let mut global_text = String::new();

    if let Some(root) = user_root {
        let _ = ensure_global_agents_md(root);
        if let Some(path) = find_candidate_in_dir(root)
            && let Some(text) = read_bounded_trimmed(&path, MAX_GLOBAL_DOC_BYTES)
        {
            global_source = Some(path);
            global_text = text;
        }
    }

    let mut project_sources = Vec::new();
    let mut project_texts = Vec::new();
    if workspace_trusted {
        let dirs = collect_search_dirs(project_root, cwd);
        let mut remaining = MAX_PROJECT_DOC_BYTES;
        for dir in dirs {
            if remaining == 0 {
                break;
            }
            if let Some(path) = find_candidate_in_dir(&dir)
                && let Some((text, used)) = read_with_budget(&path, remaining)
            {
                remaining = remaining.saturating_sub(used);
                project_sources.push(path);
                project_texts.push(text);
            }
        }
    }

    let mut body = String::new();
    if !global_text.is_empty() {
        body.push_str(&global_text);
    }
    for (idx, text) in project_texts.iter().enumerate() {
        if !body.is_empty() {
            if idx == 0 && !global_text.is_empty() {
                body.push_str(AGENTS_MD_SEPARATOR);
            } else {
                body.push_str("\n\n");
            }
        }
        body.push_str(text);
    }

    LoadedInstructions {
        global_source,
        project_sources,
        directory: cwd.or(project_root).map(Path::to_path_buf),
        body,
    }
}
```

---

### 3.2 模型身份声明与切换模型提示词注入（`src/chat/agent/model_switch.rs`）

#### 3.2.1 `# Your Model` 身份声明（对齐 OpenCode `IdentityPlugin`）
每次请求（含主会话与子 Agent）在 `PromptParts` 中注入当前冻结的模型身份段，避免跨模型切换后新模型被历史消息中旧模型的自我认知误导：

```rust
/// 当前轮冻结的模型身份描述（注入系统提示词 `# Your Model` 段）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelIdentity {
    pub display_name: String,
    pub provider_id: String,
    pub model_id: String,
    pub variant: Option<String>,
    pub supports_image_input: bool,
}

impl ModelIdentity {
    /// 渲染 `# Your Model` 系统提示词段落（纯英文，符合项目规范）。
    pub fn render_prompt_section(&self) -> String {
        let mut lines = vec![
            "# Your Model".to_string(),
            format!("- Name: {}", self.display_name),
            format!("- Provider ID: {}", self.provider_id),
            format!("- Model ID: {}", self.model_id),
        ];
        if let Some(variant) = self.variant.as_deref().filter(|v| !v.is_empty()) {
            lines.push(format!("- Reasoning Effort / Variant: {variant}"));
        }
        lines.push(format!(
            "- Image Input Support: {}",
            if self.supports_image_input {
                "enabled"
            } else {
                "disabled (historical images are replaced with text placeholders)"
            }
        ));
        lines.join("\n")
    }
}
```

#### 3.2.2 会话中途切换模型的判定与 `<system-update>` 注入（对齐 OpenCode）
1. **何时产生模型切换记录**：
   - 当用户在**已绑定 `session_id` 且已有至少一条消息**的会话中执行 `/model` 完成选择时，对比切换前生效的选择 `previous: ModelSnapshot` 与新选择 `next: ModelSnapshot`。
   - 若 `previous == next`（未发生变化），不产生切换记录。
   - 若 `previous != next`，计算变更的指令源标签列表 `updated_sources`：
     - 跨模型切换（`provider` 或 `model` 变化）：包含 `"core/model"`；
     - 同模型仅切换 `variant`：包含 `"core/reasoning_effort"`；
     - 若当前工作区 `AGENTS.md` 内容与会话已应用快照存在差异或非空：包含 `"core/agents_md"`。
2. **连续切换合并（Coalescing）**：
   - 若用户在发送下一条用户消息之前**连续切换了多次模型**（即当前会话最新一条时间线事件就是尚未被新 `agent_message` 隔开的 `agent_model_switch`）：
   - 数据库保留全部切换记录（每条独立一行、各自分配 `sequence`），界面按连续条目合并规则显示为 `-- Model switched xN --`，描述行与展开内容以**最后一次生效的目标模型**为准，起点 `from` 保留连续切换前的初始模型。
   - 合并卡片以 `switches[first..=last]` 区间整体判定展开态（`LxModelSwitch::is_range_expanded`）：追加新切换时 `first` 不变，已展开的卡片不会被重置关闭；折叠描述行汇总区间内去重保序的全部 `updated_sources`；展开首行按区间渲染完整跃迁链（`switched A -> B -> A`），不会退化为 `switched A -> A`。
3. **向模型上下文注入 `<system-update>`**：
   - 在 `chat::context::assemble` 组装历史轮次时，按 `sequence` 顺序将位于已完成轮次之后的 `ModelSwitchRecord` 转换为一条合成的 `ModelItem::UserText`（置于下一轮用户消息之前，包装在 `<system-update>...</system-update>` 内，与 OpenCode `wrapSystemUpdate` 机制一致）：
   ```rust
   /// 构造注入对话流或展开详情的英文提示词更新文本。
   pub fn render_model_switch_update(
       from_label: Option<&str>,
       to_identity: &ModelIdentity,
       updated_sources: &[String],
       instruction_sources: &[String],
   ) -> String {
       let mut lines = Vec::new();
       let target_label = to_identity.label_with_variant();
       match from_label.filter(|label| !label.trim().is_empty()) {
           Some(prev) => lines.push(format!(
               "Model switched from {prev} to {target_label}. Earlier assistant turns in this conversation were generated by {prev}; continue the conversation seamlessly as `lx` using your current model."
           )),
           None => lines.push(format!("Active model set to {target_label}.")),
       }
       if !updated_sources.is_empty() {
           lines.push(format!(
               "Instructions updated: {}",
               updated_sources.join(", ")
           ));
       }
       if !instruction_sources.is_empty() {
           lines.push(format!(
               "Active instruction files: {}",
               instruction_sources.join(", ")
           ));
       }
       lines.push(String::new());
       lines.push(to_identity.render_prompt_section());
       lines.join("\n")
   }

   /// 包装为发给模型的 `<system-update>` 块（内部转义 `</system-update>` 防止闭合逃逸）。
   pub fn wrap_system_update(content: &str) -> String {
       let safe = content.replace("</system-update>", "&lt;/system-update&gt;");
       format!("<system-update>\n{}\n</system-update>", safe.trim())
   }
   ```

---

### 3.3 `PromptParts` 顺序与 `PROMPT_VERSION` 演进（`src/chat/prompt.rs`）

1. `PROMPT_VERSION` 从 `"lx-7"` 递增为 `"lx-8"`。
2. `PromptParts` 新增 `model_identity: Option<&'a str>` 与 `instructions: Option<&'a str>`，固定拼接顺序为：
   `LX_SYSTEM_PROMPT → model_identity → environment → instructions → skills → mcp → LX_BACKGROUND_NOTE → compact`
   ```rust
   /// 系统提示的可选段落；顺序固定：model_identity → environment → instructions → skills → mcp → compact。
   #[derive(Debug, Clone, Copy, Default)]
   pub struct PromptParts<'a> {
       /// 当前模型身份声明（`# Your Model`）；None 时省略。
       pub model_identity: Option<&'a str>,
       /// 本 turn 冻结的环境快照；None 时现场采集一次（测试与旧调用路径）。
       pub environment: Option<&'a str>,
       /// 全局与项目级 `AGENTS.md` 指令段（`# AGENTS.md instructions ...`）；None 时省略。
       pub instructions: Option<&'a str>,
       /// skill 目录段（`## Skills`）；None 时省略。
       pub skills: Option<&'a str>,
       /// MCP server 说明段（`## MCP servers`）；None 时省略。
       pub mcp: Option<&'a str>,
       /// 当前生效的 compact 摘要；None 时省略。
       pub compact: Option<&'a str>,
   }
   ```
3. `compose_subagent_prompt` 复用 `parts.render_tail_with(false)`，因此子 Agent（`explorer` / `executor`）自动获得 `model_identity`、`environment`、`instructions`（`AGENTS.md`）、`skills` 与 `mcp`。

---

### 3.4 数据库持久化设计（Schema v22，`src/db/migration.rs` & `src/db/chat/model_switch.rs`）

1. **`SCHEMA_VERSION` 推进至 `22`**（当前 `dev` 为 `21`）。
2. **新增表 `agent_model_switch`**：
   ```sql
   CREATE TABLE IF NOT EXISTS agent_model_switch (
       id INTEGER PRIMARY KEY,
       external_id TEXT NOT NULL UNIQUE,
       session_id TEXT NOT NULL REFERENCES agent_session(external_id) ON DELETE CASCADE,
       sequence INTEGER NOT NULL,
       from_provider TEXT,
       from_model TEXT,
       from_variant TEXT,
       to_provider TEXT NOT NULL,
       to_model TEXT NOT NULL,
       to_variant TEXT,
       from_label TEXT,
       to_label TEXT NOT NULL,
       updated_sources_json TEXT NOT NULL DEFAULT '[]',
       injected_prompt TEXT NOT NULL DEFAULT '',
       created_at_ms INTEGER NOT NULL,
       UNIQUE(session_id, sequence)
   );
   CREATE INDEX IF NOT EXISTS idx_agent_model_switch_session_seq
       ON agent_model_switch(session_id, sequence);
   ```
3. **写入请求 `ModelSwitchCommit`**（`src/db/chat/model_switch.rs`）：
   ```rust
   /// 一次模型切换的写入请求。
   #[derive(Debug, Clone, PartialEq, Eq)]
   pub struct ModelSwitchCommit {
       pub session_id: String,
       pub model: ModelSnapshot,
       pub from_label: Option<String>,
       pub to_label: String,
       pub updated_sources: Vec<String>,
       pub injected_prompt: String,
       /// 窗格真实工作目录（`Effect::SetChatSessionModel` 用它探测该目录链上的 `AGENTS.md`）。
       pub cwd: Option<std::path::PathBuf>,
       /// 目标模型身份（服务层重新渲染 `injected_prompt` 时复用）。
       pub to_identity: crate::chat::agent::ModelIdentity,
   }
   ```
   `Effect::SetChatSessionModel` 在 `src/db/service/chat.rs` 中以 `commit.cwd`（回退工作区根）加载 `LoadedInstructions`；若加载到指令，补齐 `core/agents_md` 后调用 `render_model_switch_update(commit.from_label.as_deref(), &commit.to_identity, &commit.updated_sources, &loaded_inst.source_labels())` 重新生成权威 `injected_prompt`，再落库并经 `update::set_model_switches`（`app/update/chat/model_switch.rs::set_switches` 的对外别名）刷新窗格状态。
4. **联合 `sequence` 取号**：
   在 `db/chat/mod.rs`（`begin_turn`）、`db/chat/rewind.rs`（`commit_rewind`）、`db/chat/compact.rs`（`commit_compact`）与新增的 `db/chat/model_switch.rs`（`record_model_switch`）中，提取统一的辅助函数 `next_session_sequence(transaction, session_id)`：
   ```sql
   SELECT COALESCE(MAX(sequence) + 1, 0) FROM (
       SELECT sequence FROM agent_message WHERE session_id = ?1
       UNION ALL
       SELECT sequence FROM agent_session_notice WHERE session_id = ?1
       UNION ALL
       SELECT sequence FROM agent_compact_checkpoint WHERE session_id = ?1
       UNION ALL
       SELECT sequence FROM agent_model_switch WHERE session_id = ?1
   )
   ```
5. **Rewind 联动**：
   当 `/rewind` 删除最后一轮消息且会话内不再剩余任何 `agent_message` 时，`DELETE FROM agent_session WHERE external_id = ?1` 通过 `ON DELETE CASCADE` 自动清理关联的 `agent_model_switch` 记录，不留空壳会话。

---

### 3.5 TUI 历史渲染设计：对齐 `compact` 与思考块压暗 Markdown 格式

#### 3.5.1 视觉结构对比（折叠态 vs 展开态）

- **`compact` 检查点（同步升级展开态为思考块树形压暗 Markdown）**：
  - 折叠态（2 行）：
    ```text
    -- Context compacted --
    model: DeepSeek V4.1 Flash · 07:24
    ```
  - 展开态（点击切换）：
    ```text
    -- Context compacted --
    model: DeepSeek V4.1 Flash · 07:24
      └─ covered sequence 0–5
         <思考块同款压暗 Markdown 渲染的摘要正文，续行对齐列 5>
    ```

- **`model_switch` 模型切换条目（`src/ui/main_content/lx/history/model_switch.rs`）**：
  - 折叠态（默认 2 行）：
    - 第 1 行（`style::accent()`）：`-- Model switched --`（连续 N 次切换合并为 `-- Model switched xN --`）
    - 第 2 行（`style::muted()`）：
      - 跨模型：`model: DeepSeek V4.1 Flash (max) · 07:24 · instructions: core/model, core/agents_md`
      - 仅切思考等级：`variant: max (DeepSeek V4.1 Flash) · 07:24 · instructions: core/reasoning_effort`
  - 展开态（鼠标点击整块或键盘切换）：
    ```text
    -- Model switched --
    model: DeepSeek V4.1 Flash (max) · 07:24 · instructions: core/model, core/agents_md
      └─ switched Claude Sonnet 4 (high) -> DeepSeek V4.1 Flash (max)
         # Your Model
         - Name: DeepSeek V4.1 Flash
         - Provider ID: 9router
         - Model ID: deepseek-v4.1-flash
         - Reasoning Effort / Variant: max
    ```
    其中第 3 行起采用思考块同款的 `  └─ ` 树形连接符（`style::chat_notice_connector()`）与 `tool::muted_markdown_lines`（即 `markdown_render::render_wrapped_literal_rules` + `span_style.patch(style::muted())`），所有续行缩进 `thinking_body_indent(width)`（5 列，窄屏 `<= 5` 列时回退 2 列）。
    连续切换合并为 `xN` 时，描述行汇总区间内去重保序的全部 `updated_sources`，展开首行渲染完整跃迁链（`switched A -> B -> A`，不回退为 `A -> A`），卡片展开态按 `switches[first..=last]` 区间判定，追加新切换不会重置已展开状态。

#### 3.5.2 关键渲染代码（`src/ui/main_content/lx/history/model_switch.rs`）

```rust
//! 模型切换提示（model switch notice）的物理行布局与绘制。
//!
//! 视觉与交互严格对齐 [`super::compact`] 及思考块压暗 Markdown 规范：
//! - 标题行：`-- Model switched --`（连续多条合并为 `-- Model switched xN --`），使用 `style::accent()`；
//! - 描述行：目标模型/等级、时间与更新指令源，使用 `style::muted()`；
//! - 展开态：以思考块同款 `  └─ ` 树形前缀 + 压暗 Markdown（`muted_markdown_lines`）渲染切换详情与注入提示词。

use ratatui::layout::Rect;

use crate::app::state::ModelSwitchNoticeView;
use crate::ui::main_content::lx::Grid;
use crate::ui::{style, text};

use super::visible_row;

/// 展开正文几何：树形前缀占满 [`super::thinking_body_indent`] 列，正文按剩余宽度换行。
fn tree_body_geometry(width: u16) -> (u16, u16) {
    let indent = super::thinking_body_indent(width);
    let body_width = width.saturating_sub(indent).max(1);
    (indent, body_width)
}

/// 展开正文首个物理行前的树形连接符（与思考块同款 `  └─ `）。
fn put_tree_leaf(grid: &mut Grid, content: Rect, y: u16) {
    grid.put_clipped(
        content.x,
        y,
        text::agent::TREE_LEAF,
        style::chat_notice_connector(),
        content.width,
    );
}

/// 合并区间内去重保序的变更指令源（中间切换的 `core/model` 不能丢）。
fn merged_sources(slice: &[ModelSwitchNoticeView]) -> Vec<String> {
    let mut sources: Vec<String> = Vec::new();
    for notice in slice {
        for source in &notice.updated_sources {
            if !sources.iter().any(|seen| seen == source) {
                sources.push(source.clone());
            }
        }
    }
    sources
}

/// 构造展开态的 Markdown 源文本（首行为完整跃迁链，其后为最后一次注入的提示词）。
fn expanded_markdown(slice: &[ModelSwitchNoticeView]) -> String {
    let Some(last) = slice.last() else {
        return String::new();
    };
    let transition = if slice.len() <= 1 {
        last.transition_line(None)
    } else {
        // 连续切换记录完整链：`A -> B -> A`，而不是退化成 `switched A -> A`。
        let mut chain = String::from("switched");
        if let Some(start) = slice[0]
            .from_label
            .as_deref()
            .filter(|label| !label.trim().is_empty())
        {
            chain.push(' ');
            chain.push_str(start);
        }
        for notice in slice {
            chain.push_str(" -> ");
            chain.push_str(&notice.target_display_with_variant());
        }
        chain
    };
    if last.injected_prompt.trim().is_empty() {
        transition
    } else {
        format!("{transition}\n\n{}", last.injected_prompt.trim())
    }
}

/// 提示占用的物理行数（与 [`draw`] 严格同口径）。
pub(super) fn rows(slice: &[ModelSwitchNoticeView], width: u16, expanded: bool) -> usize {
    let mut total = super::wrap(&title(slice.len()), width).len();
    total += super::wrap(&description(slice), width).len();
    if expanded {
        let (_, body_width) = tree_body_geometry(width);
        total += super::tool::muted_markdown_lines(&expanded_markdown(slice), body_width).len();
    }
    total
}

/// 标题（连续多条合并为 `-- Model switched xN --`）。
pub(super) fn title(count: usize) -> String {
    text::model_switch_notice_title(count)
}

/// 头部描述：模型/思考等级、时间与区间内去重保序的全部变更指令源。
pub(super) fn description(slice: &[ModelSwitchNoticeView]) -> String {
    let Some(last) = slice.last() else {
        return String::new();
    };
    let variant_only = slice.len() == 1 && last.is_variant_only();
    text::model_switch_notice_desc(
        variant_only,
        &last.to_label,
        last.to_variant.as_deref(),
        &crate::platform::local_time_label(last.created_at_ms),
        &merged_sources(slice),
    )
}

/// 绘制模型切换提示并推进内容行号。
pub(super) fn draw(
    grid: &mut Grid,
    slice: &[ModelSwitchNoticeView],
    expanded: bool,
    content: Rect,
    scroll: usize,
    row_index: &mut usize,
) {
    if slice.is_empty() {
        return;
    }
    for line in super::wrap(&title(slice.len()), content.width) {
        if let Some(y) = visible_row(content, scroll, *row_index) {
            grid.put_clipped(content.x, y, &line, style::accent(), content.width);
        }
        *row_index += 1;
    }
    for line in super::wrap(&description(slice), content.width) {
        if let Some(y) = visible_row(content, scroll, *row_index) {
            grid.put_clipped(content.x, y, &line, style::muted(), content.width);
        }
        *row_index += 1;
    }
    if !expanded {
        return;
    }
    let (indent, body_width) = tree_body_geometry(content.width);
    let tree = super::thinking_tree(content.width);
    // 只有展开态的第一个物理行画连接符；首行被滚出视口时不再补画。
    let mut first_expanded_row = true;
    for segments in super::tool::muted_markdown_lines(&expanded_markdown(slice), body_width) {
        if let Some(y) = visible_row(content, scroll, *row_index) {
            if first_expanded_row && tree {
                put_tree_leaf(grid, content, y);
            }
            super::tool::put_segments(
                grid,
                content.x.saturating_add(indent),
                y,
                body_width,
                &segments,
            );
        }
        first_expanded_row = false;
        *row_index += 1;
    }
}
```

---

## 4. 不变量与边界条件（Invariants）

1. **身份不可变**：`chat::prompt::AGENT_NAME` 恒为 `"lx"`，系统提示首行恒为 `"You are lx, ..."`，任何 `AGENTS.md` 或模型切换逻辑均不得改写 `LX_SYSTEM_PROMPT` 首段。
2. **单条 System 长度安全线**：原生 `system` 槽仍经 `SystemPromptParts::split` 严格限制在 `<= 1900` UTF-16 码元；`# Your Model` 与 `AGENTS.md` 均位于 `tail` 段并经 XML 转义包裹在 `<system-instructions>` 内，绝不触发上游代理替换。
3. **未信任工作区安全隔离**：未信任工作区绝不读取项目目录内的 `AGENTS.override.md` / `AGENTS.md` / `agent.md`，仅读取用户自己的 `~/.lx-t[-dev]/` 全局指令。
4. **空白会话不刷屏**：未绑定会话或消息列表为空时切换模型，只更新窗格 `selection` 与 `ai.json` 的 `defaultModel`，不产生 `-- Model switched --` 时间线条目。
5. **渲染只读与行号严格一致**：`ui/main_content/lx/history/model_switch.rs` 与 `compact.rs` 的 `rows()` 和 `draw()` 共享完全相同的 `wrap` 与 `muted_markdown_lines` 计算，块间空行仅由 `history::snapshot` 累加。
6. **生产文件行数红线**：拆分后 `src/chat/agent/` 下各文件及新增的 `model_switch` 相关模块均控制在 700 行以内，绝不超过 1000 行上限；单元测试同步按子模块拆分。
