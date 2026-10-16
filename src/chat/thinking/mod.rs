//! `<think>` / `<thinking>` 正文标签状态机：跨 chunk 拆分 thinking 与正文。
//!
//! 协议原生 reasoning（如 OpenAI `reasoning_content`、Anthropic
//! `thinking_delta`）不经过此模块——直接进入 Reasoning part。
//! 只有正文可能内嵌标签时才用本状态机：完整开/闭标签才切换模式，
//! 跨 chunk 的半截标签先缓冲，不泄漏进正文；普通比较符号（`a < b`）原样吐出。

use super::events::PartKind;

/// 开标签集合（正文 → 思考）。
const OPEN_TAGS: [&str; 2] = ["<think>", "<thinking>"];
/// 闭标签集合（思考 → 正文）。
const CLOSE_TAGS: [&str; 2] = ["</think>", "</thinking>"];

/// 标签状态机的输出：一段正文或一段思考。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Segment {
    pub kind: PartKind,
    pub text: String,
}

/// 跨 chunk 标签拆分器。
///
/// 实现：把新输入并入缓冲，从缓冲头逐段消化——在任意位置尝试匹配标签；
/// 匹配失败时推进到下一个可能成为标签起点的 `<` 位置。缓冲尾部保留
/// 最长的“候选标签真前缀”（如 `<thi`），等下一个 chunk 判定。
#[derive(Debug, Default)]
pub struct TagSplitter {
    /// 当前模式：正文内检测开标签，思考内检测闭标签。
    in_reasoning: bool,
    /// 尚未判定的尾部缓冲（可能是半截标签）。
    pending: String,
}

impl TagSplitter {
    pub fn new() -> Self {
        Self::default()
    }

    /// 喂入一段正文增量；返回拆分后的段列表（可能为空：全部进缓冲）。
    pub fn feed(&mut self, text: &str) -> Vec<Segment> {
        let mut buffer = std::mem::take(&mut self.pending);
        buffer.push_str(text);
        let chars: Vec<char> = buffer.chars().collect();
        let mut out = Vec::new();
        // 已消化的字符位置（emit 内容为 buffer[consumed..index]）。
        let mut consumed = 0_usize;
        let mut index = 0_usize;
        while index < chars.len() {
            let rest: String = chars[index..].iter().collect();
            if let Some(tag) = self.match_tag(&rest) {
                let before: String = chars[consumed..index].iter().collect();
                self.emit(&mut out, &before);
                self.in_reasoning = !self.in_reasoning;
                index += tag.chars().count();
                consumed = index;
                continue;
            }
            // 跳到下一个 `<`（下一个可能的标签起点）；无 `<` 时跳到结尾。
            let next = chars[index + 1..]
                .iter()
                .position(|ch| *ch == '<')
                .map(|offset| index + 1 + offset)
                .unwrap_or(chars.len());
            index = next;
        }
        // 尾部保留可能是半截标签的最长后缀，其余立即吐出。
        let tail: String = chars[consumed..].iter().collect();
        let hold = self.hold_suffix(&tail);
        let emit_len = tail.len() - hold.len();
        if emit_len > 0 {
            let before: String = tail[..emit_len].to_string();
            self.emit(&mut out, &before);
        }
        self.pending = tail[emit_len..].to_string();
        out
    }

    /// 流结束：吐出残余缓冲；已进入 reasoning 时归 thinking（调用方据此
    /// 把终态标为不完整，不能把残余标签写进正文）。
    pub fn finish(&mut self) -> Vec<Segment> {
        let mut out = Vec::new();
        if !self.pending.is_empty() {
            let text = std::mem::take(&mut self.pending);
            self.emit(&mut out, &text);
        }
        out
    }

    /// 是否处于 reasoning 模式（流中断时保留 partial 判定用）。
    pub fn is_in_reasoning(&self) -> bool {
        self.in_reasoning
    }

    /// 追加一段文本到输出。
    fn emit(&self, out: &mut Vec<Segment>, text: &str) {
        if text.is_empty() {
            return;
        }
        out.push(Segment {
            kind: if self.in_reasoning {
                PartKind::Reasoning
            } else {
                PartKind::Text
            },
            text: text.to_string(),
        });
    }

    /// 在字符串开头尝试匹配标签。
    fn match_tag<'a>(&self, rest: &'a str) -> Option<&'a str> {
        let candidates: &[&str] = if self.in_reasoning {
            &CLOSE_TAGS
        } else {
            &OPEN_TAGS
        };
        candidates.iter().copied().find(|tag| rest.starts_with(tag))
    }

    /// 尾部最长的“候选标签真前缀”后缀（尚未完整到达的半截标签，如 `a <thi`）。
    ///
    /// 扫描 `tail` 的每个字符边界起点，取以该起点开始的后缀中最长的
    /// 候选标签真前缀；这样跨 chunk 的 `<thi` + `nk>` 不会被吐进正文。
    fn hold_suffix<'a>(&self, tail: &'a str) -> &'a str {
        let candidates: &[&str] = if self.in_reasoning {
            &CLOSE_TAGS
        } else {
            &OPEN_TAGS
        };
        let mut hold = "";
        for (offset, _) in tail.char_indices() {
            let suffix = &tail[offset..];
            if candidates
                .iter()
                .any(|tag| tag.starts_with(suffix) && suffix.len() < tag.len())
                && suffix.len() > hold.len()
            {
                hold = suffix;
            }
        }
        hold
    }
}

#[cfg(test)]
mod tests;
