//! prompt 附件与粘贴提案：纯数据模型、状态转换与 Prompt 上的附件/粘贴操作。

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use super::{EditKind, Prompt};

/// 附件种类；目录不成为附件。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AttachmentKind {
    File,
    Image,
}

impl AttachmentKind {
    /// DB 稳定值。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::File => "file",
            Self::Image => "image",
        }
    }

    /// 从 DB 稳定值解析；未知值回退普通文件。
    pub fn parse(value: &str) -> Self {
        match value {
            "image" => Self::Image,
            _ => Self::File,
        }
    }
}

/// 附件来源：外部文件引用或本应用管理的截图资产。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttachmentSource {
    External,
    Managed,
}

/// prompt 附件；路径是对外部文件的引用，不复制原文件。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptAttachment {
    /// 稳定 ID：新增时本地生成，保存后与数据库 external_id 一致。
    pub id: String,
    pub path: PathBuf,
    pub kind: AttachmentKind,
    pub name: String,
    pub source: AttachmentSource,
    /// Managed 附件的资产外部标识（与文件名 stem 一致）；外部附件为 None。
    pub managed_asset_id: Option<String>,
}

impl PromptAttachment {
    /// 外部文件附件。
    pub fn external(path: PathBuf, kind: AttachmentKind) -> Self {
        Self {
            id: new_attachment_id(),
            name: attachment_name(&path),
            path,
            kind,
            source: AttachmentSource::External,
            managed_asset_id: None,
        }
    }

    /// 应用管理截图附件。
    pub fn managed(path: PathBuf, kind: AttachmentKind, asset_id: String) -> Self {
        Self {
            id: new_attachment_id(),
            name: attachment_name(&path),
            path,
            kind,
            source: AttachmentSource::Managed,
            managed_asset_id: Some(asset_id),
        }
    }
}

/// 附件展示名：文件末段；无末段回退完整路径。
pub fn attachment_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
}

/// 附件路径去重键（平台稳定比较规则）。
pub fn attachment_key(path: &Path) -> String {
    crate::platform::path_key(path)
}

/// 附件稳定 ID：时间戳纳秒 + 进程内序号；仅用于业务层标识。
fn new_attachment_id() -> String {
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or(0);
    let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
    format!("att-{nanos:x}-{sequence:x}")
}

/// 粘贴候选种类。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PasteCandidateKind {
    File,
    Folder,
    Image,
}

/// 粘贴候选：规范路径、类型、来源与展示名。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PasteCandidate {
    pub path: PathBuf,
    pub kind: PasteCandidateKind,
    pub name: String,
    /// 待处理 managed 截图租约的资产 ID；外部候选为 None。
    pub managed_asset_id: Option<String>,
    /// 引用语法是否可无损表达该路径。
    pub reference_ok: bool,
}

impl PasteCandidate {
    /// 外部文件系统候选。
    pub fn external(path: PathBuf, kind: PasteCandidateKind) -> Self {
        let reference_ok =
            crate::app::markdown::format_reference(candidate_refer_kind(kind), &path).is_some();
        Self {
            name: attachment_name(&path),
            path,
            kind,
            managed_asset_id: None,
            reference_ok,
        }
    }

    /// managed 截图候选。
    pub fn managed(path: PathBuf, asset_id: String) -> Self {
        let reference_ok =
            crate::app::markdown::format_reference(crate::app::markdown::ReferKind::Image, &path)
                .is_some();
        Self {
            name: attachment_name(&path),
            path,
            kind: PasteCandidateKind::Image,
            managed_asset_id: Some(asset_id),
            reference_ok,
        }
    }

    /// 是否可上传：目录不能成为附件。
    pub fn can_upload(&self) -> bool {
        self.kind != PasteCandidateKind::Folder
    }

    /// 转成附件；目录返回 None。
    pub fn to_attachment(&self) -> Option<PromptAttachment> {
        match (&self.kind, &self.managed_asset_id) {
            (PasteCandidateKind::Folder, _) => None,
            (PasteCandidateKind::File, _) => Some(PromptAttachment::external(
                self.path.clone(),
                AttachmentKind::File,
            )),
            (PasteCandidateKind::Image, Some(asset_id)) => Some(PromptAttachment::managed(
                self.path.clone(),
                AttachmentKind::Image,
                asset_id.clone(),
            )),
            (PasteCandidateKind::Image, None) => Some(PromptAttachment::external(
                self.path.clone(),
                AttachmentKind::Image,
            )),
        }
    }
}

/// 候选种类对应的引用类型。
fn candidate_refer_kind(kind: PasteCandidateKind) -> crate::app::markdown::ReferKind {
    use crate::app::markdown::ReferKind;
    match kind {
        PasteCandidateKind::File => ReferKind::File,
        PasteCandidateKind::Folder => ReferKind::Folder,
        PasteCandidateKind::Image => ReferKind::Image,
    }
}

/// 粘贴处理方式；面板三项固定顺序。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PasteAction {
    Upload,
    Reference,
    Path,
}

impl PasteAction {
    /// 面板固定顺序。
    pub const ALL: [Self; 3] = [Self::Upload, Self::Reference, Self::Path];
}

/// 提案锚点：请求时刻的选区（优先）与光标字节偏移。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PasteAnchor {
    pub selection: Option<(usize, usize)>,
    pub cursor: usize,
}

impl PasteAnchor {
    /// 当前光标锚点。
    pub fn at_cursor(cursor: usize) -> Self {
        Self {
            selection: None,
            cursor,
        }
    }

    /// 替换范围：有选区取选区，否则空区间落在光标处。
    pub fn range(&self) -> (usize, usize) {
        self.selection.unwrap_or((self.cursor, self.cursor))
    }
}

/// 粘贴二级面板：三项操作、候选与请求代号。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PastePanel {
    pub request: u64,
    pub candidates: Vec<PasteCandidate>,
    pub anchor: PasteAnchor,
    active: usize,
}

impl PastePanel {
    /// 组装面板；高亮落在首个可用项。
    pub fn new(request: u64, candidates: Vec<PasteCandidate>, anchor: PasteAnchor) -> Self {
        let mut panel = Self {
            request,
            candidates,
            anchor,
            active: 0,
        };
        panel.active = panel.first_enabled();
        panel
    }

    /// 上传是否可用：全部候选可上传（无目录）。
    pub fn upload_enabled(&self) -> bool {
        !self.candidates.is_empty() && self.candidates.iter().all(PasteCandidate::can_upload)
    }

    /// 引用是否可用：全部候选路径可无损表达。
    pub fn reference_enabled(&self) -> bool {
        !self.candidates.is_empty() && self.candidates.iter().all(|item| item.reference_ok)
    }

    /// 操作是否可用；路径模式只要候选非空。
    pub fn enabled(&self, action: PasteAction) -> bool {
        match action {
            PasteAction::Upload => self.upload_enabled(),
            PasteAction::Reference => self.reference_enabled(),
            PasteAction::Path => !self.candidates.is_empty(),
        }
    }

    /// 高亮索引。
    pub fn active(&self) -> usize {
        self.active
    }

    /// 高亮操作。
    pub fn active_action(&self) -> Option<PasteAction> {
        PasteAction::ALL.get(self.active).copied()
    }

    /// 鼠标悬停：仅接受可用项。
    pub fn set_active(&mut self, index: usize) -> bool {
        let Some(action) = PasteAction::ALL.get(index).copied() else {
            return false;
        };
        if !self.enabled(action) || self.active == index {
            return false;
        }
        self.active = index;
        true
    }

    /// 键盘移动高亮：跳过禁用项；无可移动项返回 false。
    pub fn move_active(&mut self, delta: isize) -> bool {
        let len = PasteAction::ALL.len() as isize;
        let mut index = self.active as isize;
        for _ in 0..len {
            index = (index + delta.signum()).rem_euclid(len);
            if self.enabled(PasteAction::ALL[index as usize]) {
                let next = index as usize;
                if next == self.active {
                    return false;
                }
                self.active = next;
                return true;
            }
        }
        false
    }

    /// 首个可用项索引；全禁用时为 0。
    fn first_enabled(&self) -> usize {
        PasteAction::ALL
            .iter()
            .position(|action| self.enabled(*action))
            .unwrap_or(0)
    }
}

/// 在途粘贴读取请求：请求代号、发起时修订号与锚点。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PendingPaste {
    request: u64,
    revision: u64,
    anchor: PasteAnchor,
}

/// 粘贴状态机：在途请求、选择面板与待清理标记。
#[derive(Debug, Default)]
pub struct PasteState {
    pending: Option<PendingPaste>,
    panel: Option<PastePanel>,
    next_request: u64,
    cleanup_requested: bool,
}

impl PasteState {
    /// 发起读取请求：取消旧面板/在途任务并登记新请求代号。
    pub fn begin(&mut self, revision: u64, anchor: PasteAnchor) -> u64 {
        self.cancel();
        let request = self.next_request.wrapping_add(1);
        self.next_request = request;
        self.pending = Some(PendingPaste {
            request,
            revision,
            anchor,
        });
        request
    }

    /// 当前在途请求代号。
    pub fn pending_request(&self) -> Option<u64> {
        self.pending.map(|pending| pending.request)
    }

    /// 请求是否仍是在途目标（用于结果登记前的快速判定）。
    pub fn request_matches(&self, request: u64) -> bool {
        self.pending
            .is_some_and(|pending| pending.request == request)
    }

    /// 写入读取结果：请求代号、修订号与锚点均匹配时打开面板并返回 true。
    pub fn complete(
        &mut self,
        request: u64,
        revision: u64,
        anchor: PasteAnchor,
        candidates: Vec<PasteCandidate>,
    ) -> bool {
        let Some(pending) = self.pending else {
            return false;
        };
        if pending.request != request
            || pending.revision != revision
            || pending.anchor != anchor
            || candidates.is_empty()
        {
            return false;
        }
        self.pending = None;
        self.panel = Some(PastePanel::new(request, candidates, pending.anchor));
        true
    }

    /// 当前面板。
    pub fn panel(&self) -> Option<&PastePanel> {
        self.panel.as_ref()
    }

    /// 当前面板（可变）。
    pub fn panel_mut(&mut self) -> Option<&mut PastePanel> {
        self.panel.as_mut()
    }

    /// 关闭面板并请求清理无人引用的待处理截图。
    pub fn cancel(&mut self) -> bool {
        let had = self.pending.is_some() || self.panel.is_some();
        if had {
            self.cleanup_requested = true;
        }
        self.pending = None;
        self.panel = None;
        had
    }

    /// 关闭面板（不请求清理）。
    pub fn close(&mut self) -> bool {
        let had = self.pending.is_some() || self.panel.is_some();
        self.pending = None;
        self.panel = None;
        had
    }

    /// 换绑文档：作废在途请求并请求清理。
    pub fn invalidate(&mut self) {
        self.cancel();
    }

    /// 取走待清理标记。
    pub fn take_cleanup(&mut self) -> bool {
        std::mem::take(&mut self.cleanup_requested)
    }
}

impl Prompt {
    /// 当前附件列表。
    pub fn attachments(&self) -> &[PromptAttachment] {
        &self.attachments
    }

    /// 附件是否可用（事件层按存在性刷新）；未知 ID 视为可用。
    pub fn attachment_available(&self, id: &str) -> bool {
        !self.unavailable.iter().any(|unavailable| unavailable == id)
    }

    /// 刷新不可用附件集合；返回是否变化。
    pub fn set_unavailable_attachments(&mut self, unavailable: Vec<String>) -> bool {
        if self.unavailable == unavailable {
            return false;
        }
        self.unavailable = unavailable;
        true
    }

    /// 待检查的附件（ID、路径）。
    pub fn attachment_paths(&self) -> Vec<(String, PathBuf)> {
        self.attachments
            .iter()
            .map(|attachment| (attachment.id.clone(), attachment.path.clone()))
            .collect()
    }

    /// 原子加载正文与附件（载入提示词条目）：清空历史与浮层并回到已保存态。
    pub fn set_document(&mut self, text: &str, attachments: Vec<PromptAttachment>) {
        self.paste.invalidate();
        self.set_text(text);
        self.attachments = attachments;
        self.unavailable.clear();
    }

    /// 追加附件：过滤目录、按路径去重（含同批重复）；单步可撤销并置脏。
    pub fn add_attachments(&mut self, candidates: &[PasteCandidate]) -> bool {
        let mut seen: Vec<String> = self
            .attachments
            .iter()
            .map(|attachment| attachment_key(&attachment.path))
            .collect();
        let mut additions: Vec<PromptAttachment> = Vec::new();
        for candidate in candidates {
            let Some(attachment) = candidate.to_attachment() else {
                continue;
            };
            let key = attachment_key(&attachment.path);
            if seen.contains(&key) {
                continue;
            }
            seen.push(key);
            additions.push(attachment);
        }
        if additions.is_empty() {
            return false;
        }
        self.record(EditKind::Other);
        self.attachments.extend(additions);
        true
    }

    /// 移除附件；不存在时 no-op；单步可撤销并置脏。
    ///
    /// 移除后登记 managed 清理请求：被移除的截图若无人再引用会被回收。
    pub fn remove_attachment(&mut self, id: &str) -> bool {
        if !self
            .attachments
            .iter()
            .any(|attachment| attachment.id == id)
        {
            return false;
        }
        self.record(EditKind::Other);
        self.attachments.retain(|attachment| attachment.id != id);
        self.unavailable.retain(|unavailable| unavailable != id);
        self.paste.cleanup_requested = true;
        true
    }

    /// 当前粘贴面板。
    pub fn paste_panel(&self) -> Option<&PastePanel> {
        self.paste.panel()
    }

    /// 发起粘贴读取请求；返回请求代号。
    pub fn begin_paste_read(&mut self, anchor: PasteAnchor) -> u64 {
        let revision = self.revision;
        self.paste.begin(revision, anchor)
    }

    /// 写入粘贴读取结果；过期结果（请求/修订/锚点不符）返回 false（调用方负责清理资源）。
    ///
    /// 成功打开面板时捕获其余互斥面板（优先级：粘贴选择最高）。
    pub fn paste_complete(
        &mut self,
        request: u64,
        anchor: PasteAnchor,
        candidates: Vec<PasteCandidate>,
    ) -> bool {
        let revision = self.revision;
        if !self.paste.complete(request, revision, anchor, candidates) {
            return false;
        }
        self.mention.clear();
        self.refer = None;
        self.slash = None;
        self.panel = None;
        true
    }

    /// 结果仍在途且目标是当前编辑器时返回 true；登记 managed 资源前校验。
    pub fn paste_request_matches(&self, request: u64) -> bool {
        self.paste.request_matches(request)
    }

    /// 丢弃在途请求（不请求清理；普通文本回退用）；返回是否有在途状态。
    pub fn paste_discard(&mut self) -> bool {
        self.paste.close()
    }

    /// 键盘移动粘贴面板高亮；返回是否消费。
    pub fn paste_move(&mut self, delta: isize) -> bool {
        self.paste
            .panel_mut()
            .is_some_and(|panel| panel.move_active(delta))
    }

    /// 鼠标悬停粘贴面板条目；返回是否变化。
    pub fn paste_set_active(&mut self, index: usize) -> bool {
        self.paste
            .panel_mut()
            .is_some_and(|panel| panel.set_active(index))
    }

    /// Esc 取消粘贴面板；返回是否消费。
    pub fn paste_escape(&mut self) -> bool {
        self.paste.cancel()
    }

    /// 确认粘贴面板操作：上传为一次附件变更，引用/路径为一次正文编辑。
    pub fn paste_confirm(&mut self, action: PasteAction) -> bool {
        let Some(panel) = self.paste.panel().cloned() else {
            return false;
        };
        if !panel.enabled(action) {
            return false;
        }
        let applied = match action {
            PasteAction::Upload => self.add_attachments(&panel.candidates),
            PasteAction::Reference => {
                let parts: Option<Vec<String>> = panel
                    .candidates
                    .iter()
                    .map(|candidate| {
                        crate::app::markdown::format_reference(
                            candidate_refer_kind(candidate.kind),
                            &candidate.path,
                        )
                    })
                    .collect();
                parts.is_some_and(|parts| self.insert_pasted(&panel.anchor, parts))
            }
            PasteAction::Path => {
                let parts: Vec<String> = panel
                    .candidates
                    .iter()
                    .map(|candidate| candidate.path.to_string_lossy().into_owned())
                    .collect();
                self.insert_pasted(&panel.anchor, parts)
            }
        };
        if applied {
            self.paste.close();
        }
        applied
    }

    /// 取消粘贴面板并登记待处理截图清理；返回是否消费。
    pub fn paste_cancel(&mut self) -> bool {
        self.paste.cancel()
    }

    /// 取走待处理截图清理请求（事件层据此运行垃圾回收）。
    pub fn take_paste_cleanup(&mut self) -> bool {
        self.paste.take_cleanup()
    }

    /// 在锚点写入路径/引用片段：多个值以单个空格连接，左邻非空白先补空格，末尾补空格；
    /// 作为一次可撤销编辑。触碰受保护行时不写入并返回 false。
    fn insert_pasted(&mut self, anchor: &PasteAnchor, parts: Vec<String>) -> bool {
        if parts.is_empty() {
            return false;
        }
        let (start, end) = anchor.range();
        let end = end.min(self.text.len());
        if start > end || !self.text.is_char_boundary(start) || !self.text.is_char_boundary(end) {
            return false;
        }
        if self.protected_edit(start, end) {
            return false;
        }
        let mut insert = parts.join(" ");
        let left_blank = self.text[..start]
            .chars()
            .next_back()
            .is_none_or(char::is_whitespace);
        if !left_blank {
            insert.insert(0, ' ');
        }
        insert.push(' ');
        self.break_group();
        self.record(EditKind::Other);
        self.text.replace_range(start..end, &insert);
        self.cursor = start + insert.len();
        self.panel_suppressed = true;
        self.scroll_cursor_into_view();
        true
    }
}
