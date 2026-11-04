//! 子 agent 昵称候选池、同父会话唯一昵称分配与 UI 身份展示格式化。

/// 默认内置子 agent 昵称池（对齐 Codex 经典科学家与数学家名字池）。
pub const DEFAULT_NICKNAME_CANDIDATES: &[&str] = &[
    "Turing",
    "Curie",
    "Euler",
    "Noether",
    "Gauss",
    "Lovelace",
    "Feynman",
    "Shannon",
    "Hilbert",
    "Dirac",
    "Riemann",
    "Poincare",
    "Pascal",
    "Hopper",
    "vonNeumann",
    "Knuth",
    "Dijkstra",
    "Hamming",
    "Ramanujan",
    "Galois",
    "Archimedes",
    "Kepler",
    "Maxwell",
    "Faraday",
];

/// 在同一父会话已使用昵称集合 `used` 之外分配下一个唯一昵称（ASCII 大小写不敏感去重）。
///
/// - 若角色配置了非空 `custom_candidates`，优先使用角色专属候选池；否则使用 [`DEFAULT_NICKNAME_CANDIDATES`]。
/// - 首轮按候选池顺序挑选未占用的裸名（如 `"Turing"`）；
/// - 候选池全部被占用后，按 `#2`, `#3`… 轮次递增追加后缀（如 `"Turing #2"`）。
pub fn allocate_nickname(used: &[String], custom_candidates: &[String]) -> String {
    let custom_clean: Vec<&str> = custom_candidates
        .iter()
        .map(String::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    let candidates: &[&str] = if custom_clean.is_empty() {
        DEFAULT_NICKNAME_CANDIDATES
    } else {
        &custom_clean
    };

    let is_taken = |candidate: &str| -> bool {
        used.iter()
            .any(|existing| existing.trim().eq_ignore_ascii_case(candidate))
    };

    for &candidate in candidates {
        if !is_taken(candidate) {
            return candidate.to_string();
        }
    }

    let mut round = 2usize;
    loop {
        for &candidate in candidates {
            let suffixed = format!("{candidate} #{round}");
            if !is_taken(&suffixed) {
                return suffixed;
            }
        }
        round = round.saturating_add(1);
    }
}

/// 格式化子 agent 的可读身份标签：
/// - 有非空 `nickname` 时返回 `"{nickname} [{role}]"`（如 `"Turing [explorer]"`）；
/// - 无 `nickname`（旧会话历史记录）时返回 `"{role}"`（如 `"explorer"`）。
pub fn format_agent_identity(nickname: Option<&str>, role: &str) -> String {
    let clean_nick = nickname.map(str::trim).filter(|s| !s.is_empty());
    let clean_role = role.trim();
    match (clean_nick, clean_role.is_empty()) {
        (Some(nick), false) => format!("{nick} [{clean_role}]"),
        (Some(nick), true) => nick.to_string(),
        (None, false) => clean_role.to_string(),
        (None, true) => String::new(),
    }
}
