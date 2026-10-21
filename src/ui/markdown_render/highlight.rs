//! `markdown` item 代码块的**轻量语法高亮**：按语言关键字/字符串/数字/注释/类型分色。
//!
//! 面向主流 coding agent 的观感：不做完整解析，只按词法扫描一行，够用且零依赖。
//! 覆盖 js/ts/java/rust 等常见语言的代码块（用户主要关心这些）；其余语言走通用
//! C 系关键字集。所有分色走 [`crate::ui::style`] 的 `code_*` 语义样式，禁止写死颜色。

use ratatui::style::Style;

use crate::ui::style;

/// 高亮一行代码为若干分色分段（保持原字符顺序，拼接即原行）。
///
/// `lang` 为 None 表示代码块**没有写语言**：此时不做任何分色，整行按代码正文色返回。
/// 「无语言」等于「纯文本」，猜成 C 系关键字集会把 `if`/`return` 之类染成关键字色，
/// 是不该有的假高亮；写了未知语言（如 `foo`）才回退通用 C 系集。
pub(crate) fn highlight_line(lang: Option<&str>, line: &str) -> Vec<(String, Style)> {
    if lang.is_none() {
        if line.is_empty() {
            return Vec::new();
        }
        return vec![(line.to_string(), style::md_read_code_block())];
    }
    let keywords = keywords_for(lang);
    // `#` 注释只对脚本/配置语言成立；Rust/C 系的 `#` 是属性/预处理指令，不算注释。
    let hash_comment = matches!(lang_kind(lang), LangKind::Hash);
    let mut spans: Vec<(String, Style)> = Vec::new();
    let mut plain = String::new();
    let chars: Vec<char> = line.chars().collect();
    let mut index = 0;
    while index < chars.len() {
        let ch = chars[index];
        // 行注释：`//`，或脚本语言里的 `#`；之后整行取注释色。
        if (ch == '/' && chars.get(index + 1) == Some(&'/')) || (ch == '#' && hash_comment) {
            flush(&mut plain, &mut spans);
            spans.push((chars[index..].iter().collect(), style::code_comment()));
            break;
        }
        // 块注释：`/* … */`（单行内成对时高亮；跨行按后续行普通处理）。
        if ch == '/' && chars.get(index + 1) == Some(&'*') {
            match find_close(&chars, index + 2, '*', '/') {
                Some(end) => {
                    flush(&mut plain, &mut spans);
                    spans.push((chars[index..=end].iter().collect(), style::code_comment()));
                    index = end + 1;
                    continue;
                }
                None => {
                    flush(&mut plain, &mut spans);
                    spans.push((chars[index..].iter().collect(), style::code_comment()));
                    break;
                }
            }
        }
        // 字符串 / 字符 / 模板串：`"…"`、`'…'`、`` `…` ``（含反斜杠转义）。
        if ch == '"' || ch == '\'' || ch == '`' {
            let mut end = index + 1;
            while end < chars.len() {
                if chars[end] == '\\' {
                    end += 2;
                    continue;
                }
                if chars[end] == ch {
                    end += 1;
                    break;
                }
                end += 1;
            }
            flush(&mut plain, &mut spans);
            spans.push((
                chars[index..end.min(chars.len())].iter().collect(),
                style::code_string(),
            ));
            index = end;
            continue;
        }
        // 数字：十进制/浮点/十六进制（`0x…`）——连续取数字与 `.`/`x`/`_`/进制的字母。
        if ch.is_ascii_digit() {
            let mut end = index;
            while end < chars.len()
                && (chars[end].is_ascii_alphanumeric() || chars[end] == '.' || chars[end] == '_')
            {
                end += 1;
            }
            flush(&mut plain, &mut spans);
            spans.push((chars[index..end].iter().collect(), style::code_number()));
            index = end;
            continue;
        }
        // 标识符：关键字 / 类型（首字母大写）/ 普通标识符。
        if ch.is_alphabetic() || ch == '_' {
            let mut end = index;
            while end < chars.len() && (chars[end].is_alphanumeric() || chars[end] == '_') {
                end += 1;
            }
            let word: String = chars[index..end].iter().collect();
            flush(&mut plain, &mut spans);
            if keywords.contains(&word.as_str()) {
                spans.push((word, style::code_keyword()));
            } else if word.chars().next().is_some_and(char::is_uppercase) {
                spans.push((word, style::code_type()));
            } else {
                plain.push_str(&word);
            }
            index = end;
            continue;
        }
        plain.push(ch);
        index += 1;
    }
    flush(&mut plain, &mut spans);
    spans
}

/// 把累积的普通文本刷入 spans（代码正文色）。
fn flush(plain: &mut String, spans: &mut Vec<(String, Style)>) {
    if !plain.is_empty() {
        spans.push((std::mem::take(plain), style::md_read_code_block()));
    }
}

/// 从 `start` 起找成对的 `open close`，返回 `close` 的下标。
fn find_close(chars: &[char], start: usize, open: char, close: char) -> Option<usize> {
    let mut index = start;
    while index + 1 < chars.len() {
        if chars[index] == open && chars[index + 1] == close {
            return Some(index + 1);
        }
        index += 1;
    }
    None
}

/// 按语言取关键字集；未知语言回退通用 C 系集。
fn keywords_for(lang: Option<&str>) -> &'static [&'static str] {
    match lang_kind(lang) {
        LangKind::Rust => RUST,
        LangKind::Web => WEB,
        LangKind::Jvm => JVM,
        LangKind::Hash => PYTHON,
        LangKind::CLike => C_LIKE,
    }
}

/// 语言大类：决定关键字集与注释规则（`#` 是否为行注释）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LangKind {
    Rust,
    Web,
    Jvm,
    /// `#` 为行注释的脚本/配置语言（Python/Ruby/Shell/YAML…）。
    Hash,
    CLike,
}

/// 由语言标记归类。
fn lang_kind(lang: Option<&str>) -> LangKind {
    match lang.unwrap_or("").to_ascii_lowercase().as_str() {
        "rs" | "rust" => LangKind::Rust,
        "js" | "jsx" | "javascript" | "ts" | "tsx" | "typescript" | "mjs" | "cjs" | "vue"
        | "svelte" => LangKind::Web,
        "java" | "kotlin" | "scala" | "groovy" => LangKind::Jvm,
        "py" | "python" | "rb" | "ruby" | "sh" | "bash" | "zsh" | "fish" | "shell" | "yaml"
        | "yml" | "toml" | "ini" | "conf" | "pl" | "perl" | "r" => LangKind::Hash,
        _ => LangKind::CLike,
    }
}

/// Rust 关键字。
const RUST: &[&str] = &[
    "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum", "extern",
    "false", "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub",
    "ref", "return", "self", "Self", "static", "struct", "super", "trait", "true", "type",
    "unsafe", "use", "where", "while", "yield",
];

/// JavaScript / TypeScript 关键字。
const WEB: &[&str] = &[
    "async",
    "await",
    "break",
    "case",
    "catch",
    "class",
    "const",
    "continue",
    "default",
    "delete",
    "do",
    "else",
    "export",
    "extends",
    "false",
    "finally",
    "for",
    "from",
    "function",
    "if",
    "implements",
    "import",
    "in",
    "instanceof",
    "interface",
    "let",
    "new",
    "null",
    "of",
    "private",
    "protected",
    "public",
    "readonly",
    "return",
    "static",
    "super",
    "switch",
    "this",
    "throw",
    "true",
    "try",
    "typeof",
    "undefined",
    "var",
    "void",
    "while",
    "yield",
];

/// Java / JVM 关键字。
const JVM: &[&str] = &[
    "abstract",
    "assert",
    "boolean",
    "break",
    "byte",
    "case",
    "catch",
    "char",
    "class",
    "const",
    "continue",
    "default",
    "do",
    "double",
    "else",
    "enum",
    "extends",
    "false",
    "final",
    "finally",
    "float",
    "for",
    "if",
    "implements",
    "import",
    "instanceof",
    "int",
    "interface",
    "long",
    "native",
    "new",
    "null",
    "package",
    "private",
    "protected",
    "public",
    "record",
    "return",
    "short",
    "static",
    "strictfp",
    "super",
    "switch",
    "synchronized",
    "this",
    "throw",
    "throws",
    "transient",
    "true",
    "try",
    "var",
    "void",
    "volatile",
    "while",
    "val",
];

/// Python 关键字（`#` 注释由 [`LangKind::Hash`] 判定）。
const PYTHON: &[&str] = &[
    "and", "as", "assert", "async", "await", "break", "class", "continue", "def", "del", "elif",
    "else", "except", "False", "finally", "for", "from", "global", "if", "import", "in", "is",
    "lambda", "None", "nonlocal", "not", "or", "pass", "raise", "return", "True", "try", "while",
    "with", "yield", "self", "print",
];

/// 通用 C 系关键字集（C/C++/C#/Go/Swift/未知语言回退）。
const C_LIKE: &[&str] = &[
    "auto",
    "bool",
    "break",
    "case",
    "catch",
    "char",
    "class",
    "const",
    "continue",
    "default",
    "defer",
    "delete",
    "do",
    "double",
    "else",
    "enum",
    "explicit",
    "extern",
    "false",
    "final",
    "float",
    "for",
    "func",
    "go",
    "goto",
    "if",
    "import",
    "inline",
    "int",
    "interface",
    "let",
    "long",
    "namespace",
    "new",
    "nil",
    "null",
    "nilptr",
    "package",
    "private",
    "protected",
    "public",
    "range",
    "register",
    "return",
    "short",
    "signed",
    "sizeof",
    "static",
    "struct",
    "switch",
    "template",
    "this",
    "throw",
    "true",
    "try",
    "typedef",
    "typename",
    "union",
    "unsigned",
    "var",
    "virtual",
    "void",
    "volatile",
    "while",
];

#[cfg(test)]
mod tests;
