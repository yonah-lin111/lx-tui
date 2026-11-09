//! 内嵌系统 skill 安装：对齐 Codex `install_system_skills`。
//!
//! `lx-guide` 的 `SKILL.md` 与 references 通过 `include_str!` 编译进二进制，
//! 每次扫描前镜像到 `<user_root>/.system/`。marker 文件保存全部内嵌内容的
//! FNV-1a 指纹，指纹一致且文件齐全时不产生任何写盘。内容变化时只原子替换
//! `lx-guide` 内的目标文件，不删除 `.system` 里的其它条目。

use std::path::{Path, PathBuf};

/// 系统 skill 根目录名（位于用户 skill 根下）。
const SYSTEM_SKILLS_DIR_NAME: &str = ".system";
/// 内容指纹文件名。
const SYSTEM_SKILLS_MARKER_FILENAME: &str = ".lx-system-skills.marker";

/// 进程内安装互斥：并发调用串行化，避免同名临时文件互相覆盖。
static INSTALL_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

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
/// 指纹一致且所有内嵌文件都存在时直接返回（零写盘）；否则就地创建目录并
/// 逐个原子替换文件，保留 `.system` 下的其它内容。
pub fn install_system_skills(user_root: &Path) -> std::io::Result<()> {
    let _guard = INSTALL_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let dest_system = system_skills_root(user_root);
    if dest_system.is_file() || dest_system.is_symlink() {
        std::fs::remove_file(&dest_system)?;
    }
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
    std::fs::create_dir_all(dest_system.join("lx-guide/references"))?;
    for (relative, content) in LX_GUIDE_FILES {
        crate::platform::write_file_atomic(&dest_system.join(relative), content.as_bytes())?;
    }
    crate::platform::write_file_atomic(&marker_path, format!("{fingerprint}\n").as_bytes())
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
