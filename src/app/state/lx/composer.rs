//! 详情区共享的独立编辑缓冲。
//!
//! BTW 线程与 Agents 追问各持一份，互不污染主输入框；编辑核心仍是 `Lx`，
//! 只禁用不该出现在详情里的入口。

use crate::app::state::Lx;

/// 独立详情编辑缓冲：包一层 `Lx`，只暴露文本编辑与面板能力。
#[derive(Debug)]
pub struct DetailComposer(Box<Lx>);

impl DetailComposer {
    /// BTW 追问：保留 `@`、粘贴与 markdown 面板。
    pub fn new_btw() -> Self {
        Self(Box::new(Lx::composer()))
    }

    /// Agents 追问：开启最小 `@`（4 Tab、朴素插入）；`/` / 路径粘贴 / markdown 面板保持关闭。
    pub fn new_agent() -> Self {
        Self(Box::new(Lx::agent_composer()))
    }

    pub fn editor(&self) -> &Lx {
        &self.0
    }

    pub fn editor_mut(&mut self) -> &mut Lx {
        &mut self.0
    }

    pub fn text(&self) -> &str {
        &self.0.input
    }

    pub fn is_empty(&self) -> bool {
        self.0.input.trim().is_empty()
    }

    pub fn clear(&mut self) {
        self.0.clear();
    }

    /// 当前文本修订号（`Lx` 每次编辑递增）。
    pub fn revision(&self) -> u64 {
        self.0.revision()
    }
}
