//! Starlark 命令规则（C 批）：`<user_data_root>/rules/*.rules` 的加载、热更新与追加。
//!
//! 语法兼容 Codex `prefix_rule(...)`；`allow` 只在本条命令不含 shell 元字符时生效。
//! 规则文件是用户显式维护的高权限配置：`allow` 会跳过沙箱。

use std::io::{Read, Seek, SeekFrom, Write};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::OnceLock;
use std::time::SystemTime;

use decision::Decision;
use policy::Evaluation;
use policy::Policy;

pub mod decision;
pub mod error;
pub mod parser;
pub mod policy;
pub mod rule;

pub use error::Error;
pub use error::Result;
pub use policy::Evaluation as RuleEvaluation;
pub use rule::PrefixRule as Rule;

/// 规则目录名（位于 `<user_data_root>` 下）。
pub const RULES_DIR_NAME: &str = "rules";
/// 模型/审批追加规则写入的文件名。
pub const DEFAULT_RULE_FILE: &str = "default.rules";
/// 单条命令规则对整条命令的裁决。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuleDecision {
    /// 所有安全分段都命中 `allow`：不套沙箱、不弹审批。
    Allow,
    /// 需要用户审批。
    Prompt(Evaluation),
    /// 直接拒绝。
    Forbidden(Evaluation),
    /// 没有决定性规则。
    None,
}

/// 规则目录 `<user_data_root>/rules`。
pub fn rules_dir() -> std::result::Result<PathBuf, crate::config::ConfigError> {
    Ok(crate::config::user_data_root()?.join(RULES_DIR_NAME))
}

/// 保守分段：命令含 shell 元字符时返回 None，`allow` 因此不生效。
///
/// 只做安全降级，不尝试完整 shell 解析；`Forbidden` / `Prompt` 的整串匹配仍生效。
pub fn split_shell_segments(command: &str) -> Option<Vec<Vec<String>>> {
    if command.chars().any(|ch| {
        matches!(
            ch,
            '&' | '|' | ';' | '<' | '>' | '$' | '`' | '(' | ')' | '{' | '}' | '\n' | '\\'
        )
    }) {
        return None;
    }
    let tokens = shlex::split(command)?;
    if tokens.is_empty() {
        None
    } else {
        Some(vec![tokens])
    }
}

/// 对整条命令应用规则（安全分段全部 allow 才返回 [`RuleDecision::Allow`]）。
pub fn decide(policy: &Policy, command: &str) -> RuleDecision {
    let (segments, allow_allowed) = match split_shell_segments(command) {
        Some(segments) => (segments, true),
        // 含元字符时按引号外控制符拆分子命令分别评估，但禁止 Allow 放行。
        None => (extract_metachar_segments(command), false),
    };

    let mut forbidden: Option<Evaluation> = None;
    let mut prompt: Option<Evaluation> = None;
    let mut all_allow = allow_allowed && !segments.is_empty();
    for segment in &segments {
        match policy.evaluate(segment) {
            Some(eval) if eval.decision == Decision::Forbidden => {
                forbidden = Some(stricter(forbidden, eval));
                all_allow = false;
            }
            Some(eval) if eval.decision.needs_approval() => {
                prompt = Some(stricter(prompt, eval));
                all_allow = false;
            }
            Some(_) => {}
            None => all_allow = false,
        }
    }
    if let Some(eval) = forbidden {
        RuleDecision::Forbidden(eval)
    } else if let Some(eval) = prompt {
        RuleDecision::Prompt(eval)
    } else if all_allow {
        RuleDecision::Allow
    } else {
        RuleDecision::None
    }
}

fn extract_metachar_segments(command: &str) -> Vec<Vec<String>> {
    let mut raw_segments = Vec::new();
    collect_raw_segments(command, &mut raw_segments);
    raw_segments
        .into_iter()
        .filter_map(|segment| {
            let tokens = shlex::split(&segment)?;
            (!tokens.is_empty()).then_some(tokens)
        })
        .collect()
}

fn flush_segment(current: &mut String, out: &mut Vec<String>) {
    let trimmed = current.trim();
    if !trimmed.is_empty() {
        out.push(trimmed.to_string());
    }
    current.clear();
}

fn collect_raw_segments(input: &str, out: &mut Vec<String>) {
    let mut current = String::new();
    let mut i = 0usize;
    let mut in_single = false;
    let mut in_double = false;

    while i < input.len() {
        let rest = &input[i..];
        let Some(ch) = rest.chars().next() else {
            break;
        };

        if in_single {
            current.push(ch);
            i += ch.len_utf8();
            if ch == '\'' {
                in_single = false;
            }
            continue;
        }

        if in_double {
            if ch == '\\' {
                current.push(ch);
                i += 1;
                if let Some(escaped) = input[i..].chars().next() {
                    current.push(escaped);
                    i += escaped.len_utf8();
                }
                continue;
            }
            if rest.starts_with("$(") {
                current.push(' ');
                let (inner, consumed) = extract_paren_command_sub(&input[i + 2..]);
                collect_raw_segments(inner, out);
                i += 2 + consumed;
                continue;
            }
            if ch == '`' {
                current.push(' ');
                let (inner, consumed) = extract_backtick_command_sub(&input[i + 1..]);
                collect_raw_segments(inner, out);
                i += 1 + consumed;
                continue;
            }
            current.push(ch);
            i += ch.len_utf8();
            if ch == '"' {
                in_double = false;
            }
            continue;
        }

        if ch == '\\' {
            current.push(ch);
            i += 1;
            if let Some(escaped) = input[i..].chars().next() {
                current.push(escaped);
                i += escaped.len_utf8();
            }
            continue;
        }
        if ch == '\'' {
            in_single = true;
            current.push(ch);
            i += 1;
            continue;
        }
        if ch == '"' {
            in_double = true;
            current.push(ch);
            i += 1;
            continue;
        }
        if rest.starts_with("$(") {
            flush_segment(&mut current, out);
            let (inner, consumed) = extract_paren_command_sub(&input[i + 2..]);
            collect_raw_segments(inner, out);
            i += 2 + consumed;
            continue;
        }
        if ch == '`' {
            flush_segment(&mut current, out);
            let (inner, consumed) = extract_backtick_command_sub(&input[i + 1..]);
            collect_raw_segments(inner, out);
            i += 1 + consumed;
            continue;
        }
        if rest.starts_with("&&") || rest.starts_with("||") {
            flush_segment(&mut current, out);
            i += 2;
            continue;
        }
        if matches!(
            ch,
            ';' | '|' | '&' | '\n' | '(' | ')' | '{' | '}' | '<' | '>'
        ) {
            flush_segment(&mut current, out);
            i += 1;
            continue;
        }
        current.push(ch);
        i += ch.len_utf8();
    }
    flush_segment(&mut current, out);
}

fn extract_paren_command_sub(input: &str) -> (&str, usize) {
    let mut depth = 1usize;
    let mut i = 0usize;
    let mut in_single = false;
    let mut in_double = false;
    while i < input.len() {
        let Some(ch) = input[i..].chars().next() else {
            break;
        };
        if in_single {
            i += ch.len_utf8();
            if ch == '\'' {
                in_single = false;
            }
            continue;
        }
        if in_double {
            if ch == '\\' {
                i += 1;
                if let Some(escaped) = input[i..].chars().next() {
                    i += escaped.len_utf8();
                }
                continue;
            }
            i += ch.len_utf8();
            if ch == '"' {
                in_double = false;
            }
            continue;
        }
        match ch {
            '\\' => {
                i += 1;
                if let Some(escaped) = input[i..].chars().next() {
                    i += escaped.len_utf8();
                }
            }
            '\'' => {
                in_single = true;
                i += 1;
            }
            '"' => {
                in_double = true;
                i += 1;
            }
            '(' => {
                depth += 1;
                i += 1;
            }
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return (&input[..i], i + 1);
                }
                i += 1;
            }
            _ => i += ch.len_utf8(),
        }
    }
    (input, input.len())
}

fn extract_backtick_command_sub(input: &str) -> (&str, usize) {
    let mut i = 0usize;
    while i < input.len() {
        let Some(ch) = input[i..].chars().next() else {
            break;
        };
        if ch == '\\' {
            i += 1;
            if let Some(escaped) = input[i..].chars().next() {
                i += escaped.len_utf8();
            }
            continue;
        }
        if ch == '`' {
            return (&input[..i], i + 1);
        }
        i += ch.len_utf8();
    }
    (input, input.len())
}

fn stricter(current: Option<Evaluation>, candidate: Evaluation) -> Evaluation {
    match current {
        None => candidate,
        Some(current) => {
            if candidate.decision > current.decision
                || (candidate.decision == current.decision
                    && candidate.matched_prefix.len() > current.matched_prefix.len())
            {
                candidate
            } else {
                current
            }
        }
    }
}

/// 规则文件签名（文件名 + mtime + 长度）。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Signature(Vec<(String, Option<SystemTime>, u64)>);

/// 规则加载状态（设置页只读展示）。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RulesStatus {
    /// 规则目录。
    pub directory: String,
    /// 目录内 `.rules` 文件总数。
    pub total_files: usize,
    /// 成功加载的文件数。
    pub loaded_files: usize,
    /// 解析失败的文件与原因。
    pub errors: Vec<String>,
}

struct Inner {
    policy: Arc<Policy>,
    signature: Signature,
    status: RulesStatus,
}

/// 规则仓库：持有最近一次成功加载的策略，按文件签名热更新。
pub struct PolicyStore {
    dir: PathBuf,
    inner: Mutex<Inner>,
    write_lock: Mutex<()>,
}

impl PolicyStore {
    /// 从目录构建（目录缺失为空策略）。
    pub fn new(dir: PathBuf) -> Self {
        let (policy, signature, status, _) = load_dir(&dir);
        Self {
            dir,
            inner: Mutex::new(Inner {
                policy: Arc::new(policy),
                signature,
                status,
            }),
            write_lock: Mutex::new(()),
        }
    }

    /// 当前策略；文件集合或 mtime 变化时先重载。
    pub fn current(&self) -> Arc<Policy> {
        self.reload_if_changed();
        lock(&self.inner).policy.clone()
    }

    /// 规则目录。
    pub fn directory(&self) -> &std::path::Path {
        &self.dir
    }

    /// 加载状态。
    pub fn status(&self) -> RulesStatus {
        self.reload_if_changed();
        lock(&self.inner).status.clone()
    }

    /// 文件集合或 mtime 变化时重载；返回是否替换了策略。
    pub fn reload_if_changed(&self) -> bool {
        let signature = match signature_of(&self.dir) {
            Ok(signature) => signature,
            Err(error) => {
                tracing::warn!(error = %error, "failed to inspect exec policy directory");
                return false;
            }
        };
        if lock(&self.inner).signature == signature {
            return false;
        }
        self.reload_with(signature)
    }

    /// 无条件重载。
    pub fn force_reload(&self) {
        let signature = match signature_of(&self.dir) {
            Ok(signature) => signature,
            Err(error) => {
                tracing::warn!(error = %error, "failed to inspect exec policy directory");
                return;
            }
        };
        self.reload_with(signature);
    }

    fn reload_with(&self, signature: Signature) -> bool {
        let (policy, loaded_signature, status, failed) = load_dir(&self.dir);
        if failed {
            tracing::warn!("exec policy reload failed; keeping the previous rules");
            return false;
        }
        let changed_again = loaded_signature != signature;
        let mut inner = lock(&self.inner);
        inner.policy = Arc::new(policy);
        // 目录在两次扫描之间发生变化：不缓存签名，下次继续重载。
        inner.signature = if changed_again {
            Signature::default()
        } else {
            loaded_signature
        };
        inner.status = status;
        true
    }

    /// 追加一条 `allow` 前缀规则到 `default.rules` 并立即重载。
    ///
    /// 先走 banned prefix 与元字符校验；同一进程内用互斥锁串行化。
    pub fn append_allow_rule(&self, prefix: &[String], justification: Option<&str>) -> Result<()> {
        if prefix.is_empty() {
            return Err(Error::InvalidPattern("prefix cannot be empty".into()));
        }
        for token in prefix {
            if token.is_empty() || token.chars().any(char::is_control) {
                return Err(Error::InvalidRule(
                    "prefix tokens must be non-empty without control characters".into(),
                ));
            }
            if token.chars().any(|ch| {
                matches!(
                    ch,
                    '|' | ';' | '&' | '$' | '`' | '(' | ')' | '{' | '}' | '<' | '>'
                )
            }) {
                return Err(Error::InvalidRule(
                    "prefix tokens must not contain shell metacharacters".into(),
                ));
            }
        }
        let _guard = lock(&self.write_lock);
        std::fs::create_dir_all(&self.dir)
            .map_err(|error| Error::InvalidRule(format!("create rules directory: {error}")))?;
        let _ = crate::platform::restrict_dir_permissions(&self.dir);
        let path = self.dir.join(DEFAULT_RULE_FILE);
        let tokens = prefix
            .iter()
            .map(|token| {
                serde_json::to_string(token)
                    .map_err(|error| Error::InvalidRule(format!("serialize prefix: {error}")))
            })
            .collect::<Result<Vec<_>>>()?;
        let mut line = format!(
            "prefix_rule(pattern = [{}], decision = \"allow\"",
            tokens.join(", ")
        );
        if let Some(justification) = justification.map(str::trim).filter(|text| !text.is_empty()) {
            let escaped = serde_json::to_string(justification)
                .map_err(|error| Error::InvalidRule(format!("serialize justification: {error}")))?;
            line.push_str(&format!(", justification = {escaped}"));
        }
        line.push_str(")\n");
        let needs_leading_newline = match std::fs::File::open(&path) {
            Ok(mut reader) => {
                let len = reader
                    .metadata()
                    .map_err(|error| {
                        Error::InvalidRule(format!("stat {}: {error}", path.display()))
                    })?
                    .len();
                if len > 0 {
                    reader.seek(SeekFrom::End(-1)).map_err(|error| {
                        Error::InvalidRule(format!("seek {}: {error}", path.display()))
                    })?;
                    let mut last = [0u8; 1];
                    reader.read_exact(&mut last).map_err(|error| {
                        Error::InvalidRule(format!("read {}: {error}", path.display()))
                    })?;
                    last[0] != b'\n'
                } else {
                    false
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
            Err(error) => {
                return Err(Error::InvalidRule(format!(
                    "open {}: {error}",
                    path.display()
                )));
            }
        };
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|error| Error::InvalidRule(format!("open {}: {error}", path.display())))?;
        if needs_leading_newline {
            file.write_all(b"\n").map_err(|error| {
                Error::InvalidRule(format!("append {}: {error}", path.display()))
            })?;
        }
        file.write_all(line.as_bytes())
            .map_err(|error| Error::InvalidRule(format!("append {}: {error}", path.display())))?;
        let _ = crate::platform::restrict_file_permissions(&path);
        self.force_reload();
        Ok(())
    }

    /// 追加模型提议的 `allow` 前缀：先走 banned prefix 校验。
    pub fn append_model_allow_rule(
        &self,
        prefix: &[String],
        justification: Option<&str>,
    ) -> Result<()> {
        if crate::chat::tools::is_banned_prefix(prefix) {
            return Err(Error::InvalidRule(
                "prefix must not name a shell, interpreter, git, package runner, rm, or sudo"
                    .into(),
            ));
        }
        self.append_allow_rule(prefix, justification)
    }
}

/// 读取指定目录的规则加载状态（不依赖进程级全局仓库）。
pub fn rules_status_for_dir(dir: &std::path::Path) -> RulesStatus {
    load_dir(dir).2
}

fn load_dir(dir: &std::path::Path) -> (Policy, Signature, RulesStatus, bool) {
    let mut status = RulesStatus {
        directory: dir.display().to_string(),
        ..RulesStatus::default()
    };
    if dir.as_os_str().is_empty() {
        return (Policy::empty(), Signature::default(), status, false);
    }
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return (Policy::empty(), Signature::default(), status, false);
        }
        Err(error) => {
            status.errors.push(format!("read directory: {error}"));
            return (Policy::empty(), Signature::default(), status, true);
        }
    };
    let mut files: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "rules")
        })
        .collect();
    files.sort();
    status.total_files = files.len();
    let mut signature = Vec::with_capacity(files.len());
    let mut rules = Vec::new();
    for path in files {
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let metadata = std::fs::metadata(&path).ok();
        signature.push((
            name.clone(),
            metadata.as_ref().and_then(|meta| meta.modified().ok()),
            metadata.as_ref().map(|meta| meta.len()).unwrap_or(0),
        ));
        let contents = match std::fs::read_to_string(&path) {
            Ok(contents) => contents,
            Err(error) => {
                status.errors.push(format!("{name}: {error}"));
                continue;
            }
        };
        let mut parser = parser::PolicyParser::new();
        match parser.parse(&name, &contents) {
            Ok(()) => {
                rules.extend(parser.build());
                status.loaded_files += 1;
            }
            Err(error) => {
                tracing::warn!(file = %name, %error, "skipping invalid exec policy file");
                status.errors.push(format!("{name}: {error}"));
            }
        }
    }
    (
        Policy::from_rules(rules),
        Signature(signature),
        status,
        false,
    )
}

fn signature_of(dir: &std::path::Path) -> std::io::Result<Signature> {
    if dir.as_os_str().is_empty() {
        return Ok(Signature::default());
    }
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Signature::default());
        }
        Err(error) => return Err(error),
    };
    let mut signature = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path
            .extension()
            .is_none_or(|extension| extension != "rules")
        {
            continue;
        }
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let metadata = std::fs::metadata(&path).ok();
        signature.push((
            name,
            metadata.as_ref().and_then(|meta| meta.modified().ok()),
            metadata.as_ref().map(|meta| meta.len()).unwrap_or(0),
        ));
    }
    signature.sort();
    Ok(Signature(signature))
}

static GLOBAL: OnceLock<PolicyStore> = OnceLock::new();

/// 进程级规则仓库（目录来自 `user_data_root`）；目录缺失时为空策略。
pub fn global() -> &'static PolicyStore {
    GLOBAL.get_or_init(|| PolicyStore::new(rules_dir().unwrap_or_default()))
}

/// turn 起点调用：规则文件变化时重载。
pub fn reload_if_changed() -> bool {
    global().reload_if_changed()
}

/// 本次执行使用的规则集。
///
/// 单元测试不读用户真实规则目录（避免测试依赖开发者本机状态），一律返回空策略；
/// 需要规则的测试直接构造 `Policy` 传入决策函数。
pub fn current_policy() -> Arc<Policy> {
    #[cfg(test)]
    {
        Arc::new(Policy::empty())
    }
    #[cfg(not(test))]
    {
        global().current()
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

#[cfg(test)]
mod tests;
