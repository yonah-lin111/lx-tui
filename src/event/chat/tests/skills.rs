//! 当前轮 `@[refer-skill]` 注入去重。

use std::sync::atomic::{AtomicU64, Ordering};

static SEQ: AtomicU64 = AtomicU64::new(0);

#[test]
fn duplicate_skill_reference_injects_once() {
    let dir = std::env::temp_dir().join(format!(
        "lx-skill-dedupe-{}-{}",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    let skill_dir = dir.join("demo");
    std::fs::create_dir_all(&skill_dir).expect("create skill dir");
    let skill = skill_dir.join("SKILL.md");
    std::fs::write(&skill, "unique skill body").expect("write skill");

    let text = format!(
        "用 @[refer-skill]({}) 和 @[refer-skill]({}) 两次",
        skill.display(),
        skill.display()
    );
    let blocks = crate::event::chat::skills::reference_blocks(&text);
    assert_eq!(blocks.len(), 1, "同一 skill 只注入一次");
    assert_eq!(blocks[0].matches("unique skill body").count(), 1);

    let _ = std::fs::remove_dir_all(&dir);
}
