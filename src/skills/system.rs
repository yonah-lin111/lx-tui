//! 内嵌系统 skill 安装：对齐 Codex `install_system_skills`。
//!
//! `lx-guide` 的 `SKILL.md` 与 references 通过 `include_str!` 编译进二进制，
//! 每次扫描前镜像到 `<user_root>/.system/`。marker 文件保存全部内嵌内容的
//! FNV-1a 指纹，指纹一致且文件齐全时不产生任何写盘。

use std::path::{Path, PathBuf};

/// 系统 skill 根目录名（位于用户 skill 根下）。
const SYSTEM_SKILLS_DIR_NAME: &str = ".system";
/// 内容指纹文件名。
const SYSTEM_SKILLS_MARKER_FILENAME: &str = ".lx-system-skills.marker";

/// 内嵌 `lx-guide` 文件：相对 `.system` 的路径与正文。
const LX_GUIDE_FILES: &[(&str, &str)] = &[
    (
        "lx-guide/SKILL.md",
        include_str!("assets/lx-guide/SKILL.md"),
    ),
    (
        "lx-guide/references/tools.md",
        include_str!("assets/lx-guide/references/tools.md"),
    ),
    (
        "lx-guide/references/subagents.md",
        include_str!("assets/lx-guide/references/subagents.md"),
    ),
    (
        "lx-guide/references/commands-and-mentions.md",
        include_str!("assets/lx-guide/references/commands-and-mentions.md"),
    ),
];

/// 系统 skill 根：`<user_root>/.system`。
pub fn system_skills_root(user_root: &Path) -> PathBuf {
    user_root.join(SYSTEM_SKILLS_DIR_NAME)
}

/// 把内嵌系统 skill 同步到 `<user_root>/.system/`。
///
/// 指纹一致且所有内嵌文件都存在时直接返回（零写盘）；否则重建整个目录。
pub fn install_system_skills(user_root: &Path) -> std::io::Result<()> {
    let dest_system = system_skills_root(user_root);
    let marker_path = dest_system.join(SYSTEM_SKILLS_MARKER_FILENAME);
    let fingerprint = fingerprint();
    if dest_system.is_dir()
        && std::fs::read_to_string(&marker_path).is_ok_and(|stored| stored.trim() == fingerprint)
        && LX_GUIDE_FILES
            .iter()
            .all(|(relative, _)| dest_system.join(relative).is_file())
    {
        return Ok(());
    }
    match std::fs::remove_dir_all(&dest_system) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    for (relative, content) in LX_GUIDE_FILES {
        let path = dest_system.join(relative);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, content)?;
    }
    std::fs::write(&marker_path, format!("{fingerprint}\n"))
}

/// 全部内嵌文件的确定性 FNV-1a 64 位指纹（十六进制）。
fn fingerprint() -> String {
    const OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut hash = OFFSET_BASIS;
    for (relative, content) in LX_GUIDE_FILES {
        for bytes in [relative.as_bytes(), content.as_bytes()] {
            for byte in bytes {
                hash ^= u64::from(*byte);
                hash = hash.wrapping_mul(PRIME);
            }
            // 路径与正文之间插入分隔字节，避免不同拼接方式产生同一指纹。
            hash ^= 0xff;
            hash = hash.wrapping_mul(PRIME);
        }
    }
    format!("{hash:016x}")
}
