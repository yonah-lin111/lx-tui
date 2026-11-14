//! 附件回放：重复目标只回放一次（避免历史里的重复 skill 行重复注入）。

use super::*;
use crate::db::chat::{MessageAttachmentKind, MessageAttachmentMeta};

fn skill_attachment(id: &str, path: &str) -> MessageAttachmentMeta {
    MessageAttachmentMeta {
        id: id.into(),
        position: 0,
        kind: MessageAttachmentKind::Skill,
        label: "demo".into(),
        marker_start: None,
        marker_end: None,
        source_path: Some(path.into()),
        mime: None,
        width: None,
        height: None,
    }
}

#[test]
fn replay_dedupes_repeated_skill_attachments() {
    let mut user = message("u1", 1, 0, "user", "complete", "用一下 skill");
    user.attachments = vec![
        skill_attachment("att1", "/tmp/skills/demo/SKILL.md"),
        skill_attachment("att2", "/tmp/skills/demo/SKILL.md"),
    ];
    let assistant = message("a1", 2, 0, "assistant", "complete", "好的");
    let assembly = assemble_unbounded(&[user, assistant], "");
    let text = &assembly.request.turns[0].text;
    assert_eq!(
        text.matches("[skill demo was loaded from").count(),
        1,
        "同一 skill 只回放一行摘要: {text}"
    );
}
