//! 平台能力：剪贴板写入、鼠标捕获与默认 shell；OS 专属代码唯一落点。

use std::io::{self, IsTerminal, Write};
use std::time::{Duration, Instant};

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;

/// 原生复制命令的等待上限；超时终止，避免剪贴板被占用时无限阻塞。
const NATIVE_COPY_TIMEOUT: Duration = Duration::from_secs(2);

/// Unix 下关闭全部已知鼠标上报模式（含旧式模式，herdr `terminal_modes` 同款清理）。
#[cfg(not(windows))]
const DISABLE_MOUSE_REPORTING: &[u8] =
    b"\x1b[?1006l\x1b[?1016l\x1b[?1015l\x1b[?1005l\x1b[?1003l\x1b[?1002l\x1b[?1000l";
/// Unix 下上报按键、释放、拖动与无按键移动（SGR 1000/1002/1003/1006）；
/// 移动上报用于右栏分割线的悬停指针形状与高亮。
#[cfg(not(windows))]
const ENABLE_MOUSE_BUTTON_REPORTING: &[u8] = b"\x1b[?1000h\x1b[?1002h\x1b[?1003h\x1b[?1006h";

/// 鼠标指针形状（OSC 22）；不支持的终端静默忽略。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PointerShape {
    Default,
    EwResize,
    NsResize,
}

impl PointerShape {
    /// OSC 22 形状名；恢复默认必须显式发 `default`，空 payload 部分终端不认。
    fn name(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::EwResize => "ew-resize",
            Self::NsResize => "ns-resize",
        }
    }
}

/// 设置鼠标指针形状；输出到 stdout，失败只影响形状提示。
pub fn set_pointer_shape(shape: PointerShape) -> io::Result<()> {
    let mut stdout = io::stdout();
    stdout.write_all(
        pointer_shape_sequence(
            shape,
            std::env::var_os("TMUX").is_some(),
            std::env::var_os("STY").is_some(),
        )
        .as_bytes(),
    )?;
    stdout.flush()
}

/// OSC 22 输出；tmux/screen 下按需加 passthrough 包装（与 OSC 52 同款）。
fn pointer_shape_sequence(shape: PointerShape, tmux: bool, screen: bool) -> String {
    let sequence = format!("\x1b]22;{}\x1b\\", shape.name());
    let passthrough = format!("\x1bPtmux;\x1b{sequence}\x1b\\");
    if tmux {
        format!("{sequence}{passthrough}")
    } else if screen {
        passthrough
    } else {
        sequence
    }
}

/// 写入系统剪贴板：OSC 52 立即写；原生工具放到后台线程并限时执行，绝不阻塞事件循环。
pub fn write_clipboard(text: &str) -> bool {
    let osc52 = write_osc52(text);
    let owned = text.to_string();
    let native = std::thread::Builder::new()
        .name("clipboard-write".to_string())
        .spawn(move || {
            if !native_copy(&owned) {
                tracing::debug!("native clipboard write failed");
            }
        })
        .is_ok();
    osc52 || native
}

/// 默认 shell：Unix 取 `$SHELL`，Windows 取 `%COMSPEC%`。
pub fn default_shell() -> String {
    #[cfg(windows)]
    let shell = std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".to_string());
    #[cfg(not(windows))]
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
    shell
}

/// 读取进程当前工作目录；平台不支持、进程已退出或无权限时返回 None。
///
/// macOS 用 `proc_pidinfo(PROC_PIDVNODEPATHINFO)`，Linux 读 `/proc/<pid>/cwd`。
pub fn process_cwd(pid: u32) -> Option<std::path::PathBuf> {
    #[cfg(target_os = "macos")]
    {
        macos::process_cwd(pid)
    }
    #[cfg(target_os = "linux")]
    {
        std::fs::read_link(format!("/proc/{pid}/cwd")).ok()
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = pid;
        None
    }
}

/// 读取 PTY 子进程的前台进程组主进程展示名；无前台进程、读不到或平台不支持时返回 None。
///
/// macOS 用 `proc_pidinfo(PROC_PIDTBSDINFO)` 取 `e_tpgid` 与 `comm`、
/// `sysctl(KERN_PROCARGS2)` 取 argv；Linux 读 `/proc/<pid>/stat` 的 tpgid 与
/// `/proc/<tpgid>/comm`；主进程是 node/bun/python/shell 等包装运行时（如
/// `node .../codex.js`）时回退到脚本参数基名，未知程序返回其自身名字。
pub fn foreground_process_name(child_pid: u32) -> Option<String> {
    #[cfg(target_os = "macos")]
    {
        macos::foreground_process_name(child_pid)
    }
    #[cfg(target_os = "linux")]
    {
        linux::foreground_process_name(child_pid)
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = child_pid;
        None
    }
}

/// 前台主进程展示名归一化：
/// 1. argv0 基名（`process.title` / `exec -a` / nix 包装脚本的展示名）非通用运行时时优先；
/// 2. 否则取非通用运行时的 comm；
/// 3. 通用运行时（node/bun/python/shell）取脚本参数基名（去 `.js` 等扩展名）；
/// 4. 最后回退 comm 或 argv0 基名。
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn resolve_process_name(comm: &str, argv: &[String]) -> Option<String> {
    let comm = comm.trim();
    if let Some(name) = argv0_name(argv) {
        return Some(name);
    }
    if !comm.is_empty() && !is_generic_runtime(comm) {
        return Some(comm.to_string());
    }
    if let Some(script) = script_arg(argv) {
        let name = script_basename(script);
        if !name.is_empty() {
            return Some(name);
        }
    }
    if !comm.is_empty() {
        return Some(comm.to_string());
    }
    let argv0 = argv.first()?;
    let name = script_basename(argv0.trim_start_matches('-'));
    (!name.is_empty()).then_some(name)
}

/// argv0 的非通用展示名；空名、通用运行时名（node/sh 等）或登录 shell 的 `-zsh` 返回 None。
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn argv0_name(argv: &[String]) -> Option<String> {
    let argv0 = argv.first()?;
    let name = script_basename(argv0.trim_start_matches('-'));
    (!name.is_empty() && !is_generic_runtime(&name)).then_some(name)
}

/// 通用运行时的脚本参数：跳过选项；`-e`/`-c`/`-m` 等求值参数不是脚本文件，返回 None。
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn script_arg(argv: &[String]) -> Option<&String> {
    let mut eval = false;
    for arg in argv.iter().skip(1) {
        if matches!(
            arg.as_str(),
            "-e" | "--eval" | "-p" | "--print" | "-c" | "-m" | "-r" | "--require"
        ) {
            eval = true;
            continue;
        }
        if arg.starts_with('-') {
            continue;
        }
        return (!eval).then_some(arg);
    }
    None
}

/// 通用运行时/shell：Agent 身份藏在它们的脚本参数里。
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn is_generic_runtime(name: &str) -> bool {
    let name = name.trim().to_lowercase();
    let name = name.strip_suffix(".exe").unwrap_or(&name);
    matches!(
        name,
        "sh" | "bash" | "zsh" | "fish" | "dash" | "ksh" | "env" | "node" | "bun" | "deno"
    ) || is_python_runtime(name)
}

/// `python` / `python3` / `python3.12` 等版本化解释器。
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn is_python_runtime(name: &str) -> bool {
    name == "python"
        || name.strip_prefix("python").is_some_and(|version| {
            !version.is_empty()
                && version
                    .split('.')
                    .all(|part| !part.is_empty() && part.chars().all(|ch| ch.is_ascii_digit()))
        })
}

/// 路径基名并去脚本扩展名；`~/.nvm/bin/codex.js` → `codex`。
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn script_basename(path: &str) -> String {
    let basename = path
        .rsplit(['/', '\\'])
        .find(|part| !part.is_empty())
        .unwrap_or(path);
    let mut name = basename.trim().to_string();
    for suffix in [".js", ".mjs", ".cjs", ".exe", ".cmd", ".bat", ".ps1"] {
        if let Some(stripped) = name.strip_suffix(suffix) {
            name = stripped.to_string();
            break;
        }
    }
    name
}

/// macOS 专属实现：proc_pidinfo 读取进程 vnode 路径与前台进程组信息。
#[cfg(target_os = "macos")]
mod macos {
    use std::ffi::CStr;
    use std::path::PathBuf;

    /// 进程当前目录路径；调用失败返回 None。
    pub(super) fn process_cwd(pid: u32) -> Option<PathBuf> {
        // 零值初始化后由 proc_pidinfo 填充；失败时字段不会被读取。
        let mut info: libc::proc_vnodepathinfo = unsafe { std::mem::zeroed() };
        let size = std::mem::size_of::<libc::proc_vnodepathinfo>() as i32;
        // SAFETY: 传入本函数栈上、尺寸正确的结构体指针，pid 由调用方提供。
        let written = unsafe {
            libc::proc_pidinfo(
                pid as i32,
                libc::PROC_PIDVNODEPATHINFO,
                0,
                (&raw mut info).cast(),
                size,
            )
        };
        if written != size {
            return None;
        }
        // SAFETY: 成功后 vip_path 是 NUL 结尾的 C 字符串。
        let path = unsafe { CStr::from_ptr(info.pvi_cdir.vip_path.as_ptr().cast()) };
        path.to_str().ok().map(PathBuf::from)
    }

    /// 前台进程组主进程展示名：读取控制终端的 `e_tpgid`，取组长进程的 comm/argv。
    pub(super) fn foreground_process_name(child_pid: u32) -> Option<String> {
        if child_pid == 0 {
            return None;
        }
        let info = process_bsdinfo(child_pid)?;
        let foreground = info.e_tpgid;
        if foreground == 0 {
            return None;
        }
        let leader = process_bsdinfo(foreground)?;
        let comm = comm_from_bsdinfo(&leader).unwrap_or_default();
        let argv = process_argv(foreground);
        super::resolve_process_name(&comm, &argv)
    }

    /// 读取进程 bsdinfo；尺寸不符（进程退出或无权限）返回 None。
    fn process_bsdinfo(pid: u32) -> Option<libc::proc_bsdinfo> {
        let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
        let size = std::mem::size_of::<libc::proc_bsdinfo>() as i32;
        // SAFETY: 传入本函数栈上、尺寸正确的结构体指针，pid 由调用方提供。
        let written = unsafe {
            libc::proc_pidinfo(
                pid as i32,
                libc::PROC_PIDTBSDINFO,
                0,
                (&raw mut info).cast(),
                size,
            )
        };
        (written == size).then_some(info)
    }

    /// bsdinfo 中的进程名（固定长度、可能被截断）；空名返回 None。
    fn comm_from_bsdinfo(info: &libc::proc_bsdinfo) -> Option<String> {
        // SAFETY: pbi_comm 是 [c_char; 16] 的定长缓冲。
        let bytes = unsafe {
            std::slice::from_raw_parts(info.pbi_comm.as_ptr().cast::<u8>(), info.pbi_comm.len())
        };
        CStr::from_bytes_until_nul(bytes)
            .ok()
            .and_then(|name| name.to_str().ok())
            .map(str::to_string)
            .filter(|name| !name.is_empty())
    }

    /// 进程 argv（KERN_PROCARGS2）；读取失败返回空表。
    fn process_argv(pid: u32) -> Vec<String> {
        kern_procargs2(pid)
            .as_deref()
            .map(parse_procargs2)
            .unwrap_or_default()
    }

    /// 读取 KERN_PROCARGS2 原始缓冲。
    fn kern_procargs2(pid: u32) -> Option<Vec<u8>> {
        let mut mib = [libc::CTL_KERN, libc::KERN_PROCARGS2, pid as libc::c_int];
        let mut size: libc::size_t = 0;
        // SAFETY: 查询缓冲区尺寸；mib 指向本函数栈上数组，sysctl 只写 size。
        let probe = unsafe {
            libc::sysctl(
                mib.as_mut_ptr(),
                3,
                std::ptr::null_mut(),
                &mut size,
                std::ptr::null_mut(),
                0,
            )
        };
        if probe != 0 || size == 0 {
            return None;
        }
        let mut buffer = vec![0u8; size];
        // SAFETY: buffer 容量为 size；sysctl 写回实际读取长度并截断到 size。
        let read = unsafe {
            libc::sysctl(
                mib.as_mut_ptr(),
                3,
                buffer.as_mut_ptr().cast(),
                &mut size,
                std::ptr::null_mut(),
                0,
            )
        };
        if read != 0 {
            return None;
        }
        buffer.truncate(size);
        Some(buffer)
    }

    /// 解析 KERN_PROCARGS2 缓冲：`[argc][exec_path\0][padding][argv0\0][argv1\0]…`。
    pub(super) fn parse_procargs2(buffer: &[u8]) -> Vec<String> {
        if buffer.len() < 4 {
            return Vec::new();
        }
        let argc = i32::from_ne_bytes([buffer[0], buffer[1], buffer[2], buffer[3]]);
        if argc < 1 {
            return Vec::new();
        }
        let rest = &buffer[4..];
        let Some(exec_end) = rest.iter().position(|byte| *byte == 0) else {
            return Vec::new();
        };
        let mut position = exec_end;
        while position < rest.len() && rest[position] == 0 {
            position += 1;
        }
        let mut argv = Vec::with_capacity(argc as usize);
        let mut remaining = &rest[position..];
        while argv.len() < argc as usize {
            let Some(end) = remaining.iter().position(|byte| *byte == 0) else {
                break;
            };
            argv.push(String::from_utf8_lossy(&remaining[..end]).into_owned());
            if end + 1 >= remaining.len() {
                break;
            }
            remaining = &remaining[end + 1..];
        }
        argv
    }
}

/// Linux 专属实现：读取 `/proc` 获取前台进程组与进程名。
#[cfg(target_os = "linux")]
mod linux {
    /// 前台进程组主进程展示名：`/proc/<pid>/stat` 的 tpgid + `/proc/<tpgid>/comm`。
    pub(super) fn foreground_process_name(child_pid: u32) -> Option<String> {
        if child_pid == 0 {
            return None;
        }
        let stat = std::fs::read_to_string(format!("/proc/{child_pid}/stat")).ok()?;
        let foreground = parse_tpgid(&stat)?;
        let comm = process_comm(foreground).unwrap_or_default();
        let argv = process_argv(foreground);
        super::resolve_process_name(&comm, &argv)
    }

    /// 解析 `/proc/<pid>/stat` 的第 8 字段 tpgid；comm 含空格时取最后一个 `)` 之后。
    fn parse_tpgid(stat: &str) -> Option<u32> {
        let rest = &stat[stat.rfind(')')? + 1..];
        let tpgid: i32 = rest.split_whitespace().nth(5)?.parse().ok()?;
        (tpgid > 0).then_some(tpgid as u32)
    }

    /// `/proc/<pid>/comm`：进程名（最长 15 字符）。
    fn process_comm(pid: u32) -> Option<String> {
        std::fs::read_to_string(format!("/proc/{pid}/comm"))
            .ok()
            .map(|name| name.trim().to_string())
            .filter(|name| !name.is_empty())
    }

    /// `/proc/<pid>/cmdline`：NUL 分隔的 argv；读取失败返回空表。
    fn process_argv(pid: u32) -> Vec<String> {
        let Ok(raw) = std::fs::read(format!("/proc/{pid}/cmdline")) else {
            return Vec::new();
        };
        raw.split(|byte| *byte == 0)
            .filter(|part| !part.is_empty())
            .map(|part| String::from_utf8_lossy(part).into_owned())
            .collect()
    }
}

/// 开启鼠标上报：Unix 用显式 VT 序列；Windows 走 crossterm 的 WinAPI 捕获。
pub fn enable_mouse_capture(writer: &mut impl Write) -> io::Result<()> {
    #[cfg(windows)]
    {
        crossterm::execute!(writer, crossterm::event::EnableMouseCapture)
    }
    #[cfg(not(windows))]
    {
        writer.write_all(ENABLE_MOUSE_BUTTON_REPORTING)?;
        writer.flush()
    }
}

/// 关闭鼠标上报；与开启路径一一对应。
pub fn disable_mouse_capture(writer: &mut impl Write) -> io::Result<()> {
    #[cfg(windows)]
    {
        crossterm::execute!(writer, crossterm::event::DisableMouseCapture)
    }
    #[cfg(not(windows))]
    {
        writer.write_all(DISABLE_MOUSE_REPORTING)?;
        writer.flush()
    }
}

/// 启用 kitty 键盘协议（DISAMBIGUATE_ESCAPE_CODES），让 Super/Cmd 组合可被收到；
/// 不支持的终端忽略该私有序列；Windows legacy 控制台无此协议，直接 no-op。
pub fn enable_keyboard_enhancement(writer: &mut impl Write) -> io::Result<()> {
    #[cfg(not(windows))]
    {
        crossterm::execute!(
            writer,
            crossterm::event::PushKeyboardEnhancementFlags(
                crossterm::event::KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
            )
        )?;
    }
    #[cfg(windows)]
    let _ = writer;
    Ok(())
}

/// 关闭 kitty 键盘协议；与开启路径一一对应。
pub fn disable_keyboard_enhancement(writer: &mut impl Write) -> io::Result<()> {
    #[cfg(not(windows))]
    {
        crossterm::execute!(writer, crossterm::event::PopKeyboardEnhancementFlags)?;
    }
    #[cfg(windows)]
    let _ = writer;
    Ok(())
}

/// OSC 52 输出：tmux/screen 下按需加 passthrough 包装（opencode 同款）。
fn osc52_output(text: &str, tmux: bool, screen: bool) -> String {
    let sequence = format!("\x1b]52;c;{}\x07", BASE64.encode(text.as_bytes()));
    let passthrough = format!("\x1bPtmux;\x1b{sequence}\x1b\\");
    if tmux {
        format!("{sequence}{passthrough}")
    } else if screen {
        passthrough
    } else {
        sequence
    }
}

fn write_osc52(text: &str) -> bool {
    let mut stdout = io::stdout();
    if !stdout.is_terminal() {
        return false;
    }
    let output = osc52_output(
        text,
        std::env::var_os("TMUX").is_some(),
        std::env::var_os("STY").is_some(),
    );
    stdout
        .write_all(output.as_bytes())
        .and_then(|()| stdout.flush())
        .is_ok()
}

/// 各平台原生复制命令；无可用工具或执行失败返回 false。
fn native_copy(text: &str) -> bool {
    #[cfg(target_os = "macos")]
    let copied = run_with_stdin("pbcopy", &[], text);
    #[cfg(windows)]
    let copied = run_with_stdin(
        "powershell.exe",
        &[
            "-NonInteractive",
            "-NoProfile",
            "-Command",
            "[Console]::InputEncoding = [System.Text.Encoding]::UTF8; \
             Set-Clipboard -Value ([Console]::In.ReadToEnd())",
        ],
        text,
    );
    #[cfg(target_os = "linux")]
    let copied = {
        let candidates: &[(&str, &[&str])] = &[
            ("wl-copy", &[]),
            ("xclip", &["-selection", "clipboard"]),
            ("xsel", &["--clipboard", "--input"]),
        ];
        candidates
            .iter()
            .any(|(program, args)| run_with_stdin(program, args, text))
    };
    #[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
    let copied = {
        let _ = text;
        false
    };
    copied
}

/// 以 stdin 传入文本执行复制命令；超时或失败返回 false。
fn run_with_stdin(program: &str, args: &[&str], text: &str) -> bool {
    use std::process::{Command, Stdio};

    let Ok(mut child) = Command::new(program)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    else {
        return false;
    };
    let Some(mut stdin) = child.stdin.take() else {
        let _ = child.wait();
        return false;
    };
    if stdin.write_all(text.as_bytes()).is_err() {
        let _ = child.wait();
        return false;
    }
    drop(stdin);

    let deadline = Instant::now() + NATIVE_COPY_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return status.success(),
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return false;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return false;
            }
        }
    }
}

#[cfg(test)]
mod tests;
