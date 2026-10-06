//! 浮层模型：右键菜单、重命名输入与关闭确认；同一时刻最多存在一个浮层。

/// 浮层种类：按键路由与渲染按种类分派。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverlayKind {
    Menu,
    Rename,
    ConfirmClose,
}

/// 浮层：同一时刻最多一个，由 `Option` 保证。
#[derive(Debug)]
pub enum Overlay {
    Menu(Menu),
    Rename(Rename),
    ConfirmClose(ConfirmClose),
}

impl Overlay {
    /// 浮层种类。
    pub fn kind(&self) -> OverlayKind {
        match self {
            Self::Menu(_) => OverlayKind::Menu,
            Self::Rename(_) => OverlayKind::Rename,
            Self::ConfirmClose(_) => OverlayKind::ConfirmClose,
        }
    }
}

/// 右键菜单：锚点、目标、命令列表与高亮项索引。
#[derive(Debug)]
pub struct Menu {
    pub anchor: (u16, u16),
    pub target: MenuTarget,
    pub commands: Vec<MenuCommand>,
    pub selected: usize,
}

/// 菜单作用目标。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuTarget {
    Workspace(usize),
}

/// 菜单命令；文案由 `ui/text.rs` 按命令映射，app 层不持有用户可见字符串。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuCommand {
    RenameWorkspace,
    CloseWorkspace,
}

/// 重命名浮层：目标工作区索引与单行输入。
#[derive(Debug)]
pub struct Rename {
    pub target: usize,
    pub input: TextInput,
}

/// 关闭确认浮层：目标工作区索引。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConfirmClose {
    pub target: usize,
}

/// 单行文本输入：字符缓冲与字符索引光标（范围 `0..=字符数`）。
#[derive(Debug)]
pub struct TextInput {
    text: String,
    cursor: usize,
}

impl TextInput {
    /// 构造输入并预填文本，光标落在末尾。
    pub fn new(text: impl Into<String>) -> Self {
        let text = text.into();
        let cursor = text.chars().count();
        Self { text, cursor }
    }

    /// 当前文本。
    pub fn text(&self) -> &str {
        &self.text
    }

    /// 光标字符索引。
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// 在光标处插入字符。
    pub fn insert_char(&mut self, ch: char) {
        let at = self.byte_offset(self.cursor);
        self.text.insert(at, ch);
        self.cursor += 1;
    }

    /// 清空文本并把光标移到开头。
    pub fn clear(&mut self) {
        self.text.clear();
        self.cursor = 0;
    }

    /// 删除光标前一个字符。
    pub fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let start = self.byte_offset(self.cursor - 1);
        let end = self.byte_offset(self.cursor);
        self.text.replace_range(start..end, "");
        self.cursor -= 1;
    }

    /// 删除光标处字符。
    pub fn delete(&mut self) {
        if self.cursor >= self.text.chars().count() {
            return;
        }
        let start = self.byte_offset(self.cursor);
        let end = self.byte_offset(self.cursor + 1);
        self.text.replace_range(start..end, "");
    }

    /// 光标左移。
    pub fn move_left(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    /// 光标右移。
    pub fn move_right(&mut self) {
        if self.cursor < self.text.chars().count() {
            self.cursor += 1;
        }
    }

    /// 光标移到开头。
    pub fn move_home(&mut self) {
        self.cursor = 0;
    }

    /// 光标移到末尾。
    pub fn move_end(&mut self) {
        self.cursor = self.text.chars().count();
    }

    /// 字符索引到字节偏移；越界返回文本长度。
    fn byte_offset(&self, char_index: usize) -> usize {
        self.text
            .char_indices()
            .nth(char_index)
            .map(|(byte, _)| byte)
            .unwrap_or(self.text.len())
    }
}

#[cfg(test)]
mod tests;
