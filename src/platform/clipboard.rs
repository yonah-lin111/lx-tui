//! 系统剪贴板读取：文件列表、位图与文本；macOS/Windows/Linux 三套实现。
//!
//! 返回平台中立数据；调用方在后台线程执行，事件循环只接收完成消息。
//! 纯解析（路径文本、URI、位图签名、资产 token）与 OS 命令执行同处本模块，
//! 核心模块不出现 `cfg(target_os)`。

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

#[cfg(windows)]
use base64::Engine;
#[cfg(windows)]
use base64::engine::general_purpose::STANDARD as BASE64;

/// 单次剪贴板命令的等待上限。
const READ_TIMEOUT: Duration = Duration::from_secs(3);

/// 图片格式；扩展名与实际编码一致。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageFormat {
    Png,
    Jpeg,
    Gif,
    WebP,
    Bmp,
    Tiff,
}

impl ImageFormat {
    /// 与编码一致的扩展名。
    pub fn extension(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Jpeg => "jpg",
            Self::Gif => "gif",
            Self::WebP => "webp",
            Self::Bmp => "bmp",
            Self::Tiff => "tiff",
        }
    }
}

/// 剪贴板内容：文件列表优先于位图，位图优先于文本。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClipboardContent {
    Files(Vec<PathBuf>),
    Image { bytes: Vec<u8>, format: ImageFormat },
    Text(String),
}

/// 剪贴板读取失败类别；调用方映射为用户可见错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClipboardError {
    /// 剪贴板为空。
    Empty,
    /// 平台能力不可用（工具缺失、读取失败或超时）。
    Unavailable(String),
}

impl std::fmt::Display for ClipboardError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => write!(formatter, "clipboard is empty"),
            Self::Unavailable(reason) => write!(formatter, "clipboard unavailable: {reason}"),
        }
    }
}

/// 读取系统剪贴板：文件列表 → 位图 → 文本；空剪贴板返回 `Empty`。
pub fn read_clipboard() -> Result<ClipboardContent, ClipboardError> {
    platform_read_clipboard()
}

/// 位图签名探测；无法识别返回 None。
pub fn sniff_image(bytes: &[u8]) -> Option<ImageFormat> {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]) {
        return Some(ImageFormat::Png);
    }
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Some(ImageFormat::Jpeg);
    }
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return Some(ImageFormat::Gif);
    }
    if bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
        return Some(ImageFormat::WebP);
    }
    if bytes.starts_with(b"BM") {
        return Some(ImageFormat::Bmp);
    }
    if bytes.starts_with(b"II\x2a\x00") || bytes.starts_with(b"MM\x00\x2a") {
        return Some(ImageFormat::Tiff);
    }
    None
}

/// 文件路径的图片探测：读取文件头签名；读不到按非图片处理。
pub fn sniff_image_file(path: &Path) -> bool {
    use std::io::Read;
    let Ok(mut file) = std::fs::File::open(path) else {
        return false;
    };
    let mut header = [0_u8; 16];
    let read = file.read(&mut header).unwrap_or(0);
    sniff_image(&header[..read]).is_some()
}

/// 随机资产 token：Unix 读 `/dev/urandom`，Windows 取 GUID；均不可用时回退时间熵。
pub fn secure_token() -> String {
    #[cfg(unix)]
    {
        use std::io::Read;
        if let Ok(mut file) = std::fs::File::open("/dev/urandom") {
            let mut bytes = [0_u8; 16];
            if file.read_exact(&mut bytes).is_ok() {
                return hex(&bytes);
            }
        }
    }
    #[cfg(windows)]
    {
        if let Some(output) = run_capture(
            "powershell.exe",
            &[
                "-NonInteractive",
                "-NoProfile",
                "-Command",
                "[guid]::NewGuid().ToString('N')",
            ],
            READ_TIMEOUT,
        ) && output.success
        {
            let token = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if token.len() == 32 {
                return token;
            }
        }
    }
    // 回退：系统熵不可用时用时间戳与进程标识生成，仍保证文件名唯一。
    tracing::warn!("secure random source unavailable; falling back to time entropy");
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or(0);
    let mut bytes = [0_u8; 16];
    bytes[..8].copy_from_slice(&(nanos as u64).to_le_bytes());
    bytes[8..12].copy_from_slice(&std::process::id().to_le_bytes());
    bytes[12..].copy_from_slice(&(nanos >> 64).to_le_bytes()[..4]);
    hex(&bytes)
}

/// 字节序列小写十六进制。
fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

/// 路径文本解析：单行/多行绝对路径或 `file://` URI，逐行去掉外层换行。
///
/// 任一行不是存在的本机绝对路径形式时整段返回 None（调用方回退普通文本）；
/// 不 `trim` 路径首尾合法空格，空行不参与。
pub fn parse_path_text(text: &str) -> Option<Vec<PathBuf>> {
    if text.is_empty() {
        return None;
    }
    // 末尾换行是外层行结束符，不构成空行；内部空行仍使整段回退普通文本。
    let text = text
        .strip_suffix('\n')
        .map(|rest| rest.strip_suffix('\r').unwrap_or(rest))
        .unwrap_or(text);
    if text.is_empty() {
        return None;
    }
    let mut paths = Vec::new();
    for line in text.split('\n') {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if line.is_empty() {
            return None;
        }
        paths.push(parse_path_line(line)?);
    }
    (!paths.is_empty()).then_some(paths)
}

/// 单行路径解析：`file://` URI 或本机绝对路径。
fn parse_path_line(line: &str) -> Option<PathBuf> {
    if line.contains('\0') {
        return None;
    }
    if let Some(rest) = line.strip_prefix("file://") {
        return parse_file_uri(rest);
    }
    let path = Path::new(line);
    path.is_absolute().then(|| path.to_path_buf())
}

/// `file://` URI 解析：仅本机 host（空或 `localhost`）；百分号序列按 UTF-8 解码。
fn parse_file_uri(rest: &str) -> Option<PathBuf> {
    let (host, path) = match rest.find('/') {
        Some(0) => ("", rest),
        Some(index) => (&rest[..index], &rest[index..]),
        None => return None,
    };
    if !host.is_empty() && host != "localhost" {
        return None;
    }
    let decoded = percent_decode(path)?;
    #[cfg(windows)]
    {
        // `/C:/Users/x` → `C:/Users/x`；UNC 与非法盘符拒绝。
        let bytes = decoded.as_bytes();
        if bytes.len() >= 3
            && bytes[0] == b'/'
            && bytes[1].is_ascii_alphabetic()
            && bytes[2] == b':'
        {
            return Some(PathBuf::from(&decoded[1..]));
        }
        return None;
    }
    #[cfg(not(windows))]
    {
        if decoded.starts_with('/') {
            Some(PathBuf::from(decoded))
        } else {
            None
        }
    }
}

/// 百分号解码；非法序列或非 UTF-8 结果返回 None。
fn percent_decode(input: &str) -> Option<String> {
    let mut bytes = Vec::with_capacity(input.len());
    let raw = input.as_bytes();
    let mut index = 0;
    while index < raw.len() {
        if raw[index] == b'%' {
            let hex = raw.get(index + 1..index + 3)?;
            let value = u8::from_str_radix(std::str::from_utf8(hex).ok()?, 16).ok()?;
            bytes.push(value);
            index += 3;
        } else {
            bytes.push(raw[index]);
            index += 1;
        }
    }
    String::from_utf8(bytes).ok()
}

/// 平台中立命令输出。
struct Captured {
    success: bool,
    stdout: Vec<u8>,
}

/// 执行命令并限时等待；超时或启动失败返回 None。
fn run_capture(program: &str, args: &[&str], timeout: Duration) -> Option<Captured> {
    let program = program.to_string();
    let args: Vec<String> = args.iter().map(|arg| arg.to_string()).collect();
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("clipboard-read".to_string())
        .spawn(move || {
            let output = Command::new(&program)
                .args(&args)
                .stdin(Stdio::null())
                .stderr(Stdio::null())
                .output();
            let _ = sender.send(output);
        })
        .ok()?;
    match receiver.recv_timeout(timeout) {
        Ok(Ok(output)) => Some(Captured {
            success: output.status.success(),
            stdout: output.stdout,
        }),
        _ => None,
    }
}

/// 三平台读取入口。
fn platform_read_clipboard() -> Result<ClipboardContent, ClipboardError> {
    #[cfg(target_os = "macos")]
    {
        macos::read()
    }
    #[cfg(target_os = "linux")]
    {
        linux::read()
    }
    #[cfg(windows)]
    {
        windows::read()
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
    {
        Err(ClipboardError::Unavailable(
            "clipboard reading is not supported on this platform".to_string(),
        ))
    }
}

/// macOS：osascript 探测剪贴板类型；文件列表与位图分别提取，文本走 pbpaste。
#[cfg(target_os = "macos")]
mod macos {
    use super::*;

    /// `clipboard info` 输出：`{{«class furl», 685}, {string, 100}}`。
    fn clipboard_info() -> Option<String> {
        let output = run_capture("osascript", &["-e", "clipboard info"], READ_TIMEOUT)?;
        output
            .success
            .then(|| String::from_utf8_lossy(&output.stdout).into_owned())
    }

    pub(super) fn read() -> Result<ClipboardContent, ClipboardError> {
        let info = clipboard_info()
            .ok_or_else(|| ClipboardError::Unavailable("osascript unavailable".to_string()))?;
        if (info.contains("furl") || info.contains("NSFilenamesPboardType"))
            && let Some(files) = read_file_list()
        {
            return Ok(ClipboardContent::Files(files));
        }
        if (info.contains("PNGf") || info.contains("TIFF"))
            && let Some(image) = read_image()
        {
            return Ok(image);
        }
        let text = run_capture("pbpaste", &[], READ_TIMEOUT)
            .filter(|output| output.success)
            .map(|output| String::from_utf8_lossy(&output.stdout).into_owned())
            .unwrap_or_default();
        if text.is_empty() {
            return Err(ClipboardError::Empty);
        }
        Ok(ClipboardContent::Text(text))
    }

    /// 文件列表：逐项 `POSIX path`；非列表内容返回 None。
    fn read_file_list() -> Option<Vec<PathBuf>> {
        let script = r#"set out to ""
try
  set theList to (the clipboard as list)
  repeat with item_ in theList
    set out to out & (POSIX path of (item_ as alias)) & linefeed
  end repeat
end try
return out"#;
        let output = run_capture("osascript", &["-e", script], READ_TIMEOUT)?;
        if !output.success {
            return None;
        }
        let text = String::from_utf8_lossy(&output.stdout);
        let files: Vec<PathBuf> = text
            .lines()
            .map(str::trim_end)
            .filter(|line| !line.is_empty())
            .map(PathBuf::from)
            .collect();
        (!files.is_empty()).then_some(files)
    }

    /// 位图：优先 PNGf，回退 TIFF；写入隔离临时文件后读回。
    fn read_image() -> Option<ClipboardContent> {
        for (class, format) in [("PNGf", ImageFormat::Png), ("TIFF", ImageFormat::Tiff)] {
            let token = secure_token();
            let path = std::env::temp_dir().join(format!("lx-clip-{token}.{}", format.extension()));
            let script = format!(
                r#"on run argv
  set outPath to item 1 of argv
  set target to open for access (POSIX file outPath) with write permission
  set eof target to 0
  write (the clipboard as «class {class}») to target
  close access target
end run"#
            );
            let output = run_capture(
                "osascript",
                &["-e", &script, path.to_string_lossy().as_ref()],
                READ_TIMEOUT,
            );
            let copied = output.is_some_and(|output| output.success)
                && std::fs::metadata(&path).is_ok_and(|meta| meta.len() > 0);
            if !copied {
                let _ = std::fs::remove_file(&path);
                continue;
            }
            let bytes = std::fs::read(&path).ok();
            let _ = std::fs::remove_file(&path);
            if let Some(bytes) = bytes
                && let Some(sniffed) = sniff_image(&bytes)
            {
                return Some(ClipboardContent::Image {
                    bytes,
                    format: sniffed,
                });
            }
        }
        None
    }
}

/// Linux：Wayland 优先 wl-paste，回退 xclip；URI 列表与 MIME 图片。
#[cfg(target_os = "linux")]
mod linux {
    use super::*;

    /// 剪贴板工具。
    #[derive(Clone, Copy)]
    enum Tool {
        WlPaste,
        Xclip,
    }

    /// 探测可用工具：优先 Wayland 会话，失败回退另一实现。
    fn tools() -> Vec<Tool> {
        let wayland = std::env::var_os("WAYLAND_DISPLAY").is_some()
            || std::env::var("XDG_SESSION_TYPE")
                .is_ok_and(|value| value.eq_ignore_ascii_case("wayland"));
        if wayland {
            vec![Tool::WlPaste, Tool::Xclip]
        } else {
            vec![Tool::Xclip, Tool::WlPaste]
        }
    }

    /// 列出剪贴板可用 MIME 类型。
    fn list_types(tool: Tool) -> Option<Vec<String>> {
        let output = match tool {
            Tool::WlPaste => run_capture("wl-paste", &["--list-types"], READ_TIMEOUT)?,
            Tool::Xclip => run_capture(
                "xclip",
                &["-selection", "clipboard", "-t", "TARGETS", "-o"],
                READ_TIMEOUT,
            )?,
        };
        if !output.success {
            return None;
        }
        Some(
            String::from_utf8_lossy(&output.stdout)
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .map(str::to_string)
                .collect(),
        )
    }

    /// 按类型读取二进制内容。
    fn read_type(tool: Tool, mime: &str) -> Option<Vec<u8>> {
        let output = match tool {
            Tool::WlPaste => run_capture("wl-paste", &["-t", mime], READ_TIMEOUT)?,
            Tool::Xclip => run_capture(
                "xclip",
                &["-selection", "clipboard", "-t", mime, "-o"],
                READ_TIMEOUT,
            )?,
        };
        (output.success && !output.stdout.is_empty()).then_some(output.stdout)
    }

    pub(super) fn read() -> Result<ClipboardContent, ClipboardError> {
        let mut last_error = String::from("no clipboard tool available");
        for tool in tools() {
            let Some(types) = list_types(tool) else {
                continue;
            };
            if types.iter().any(|mime| mime == "text/uri-list") {
                if let Some(bytes) = read_type(tool, "text/uri-list")
                    && let Ok(text) = String::from_utf8(bytes.clone())
                    && let Some(files) = parse_uri_list(&text)
                {
                    return Ok(ClipboardContent::Files(files));
                }
            }
            if let Some(mime) = types.iter().find(|mime| mime.starts_with("image/")) {
                if let Some(bytes) = read_type(tool, mime)
                    && let Some(format) = sniff_image(&bytes)
                {
                    return Ok(ClipboardContent::Image { bytes, format });
                }
            }
            if types.iter().any(|mime| mime.starts_with("text/")) {
                if let Some(mime) = types
                    .iter()
                    .find(|mime| mime.starts_with("text/plain"))
                    .or(types.first())
                    && let Some(bytes) = read_type(tool, mime)
                    && !bytes.is_empty()
                {
                    let text = String::from_utf8_lossy(&bytes).into_owned();
                    if !text.is_empty() {
                        return Ok(ClipboardContent::Text(text));
                    }
                }
            }
            last_error = "clipboard is empty".to_string();
        }
        if last_error == "clipboard is empty" {
            Err(ClipboardError::Empty)
        } else {
            Err(ClipboardError::Unavailable(last_error))
        }
    }

    /// URI 列表解析：逐行 `file://`；非 URI 行忽略，全部无效返回 None。
    fn parse_uri_list(text: &str) -> Option<Vec<PathBuf>> {
        let files: Vec<PathBuf> = text
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
            .filter_map(|line| {
                line.strip_prefix("file://").and_then(|rest| {
                    let (host, path) = match rest.find('/') {
                        Some(0) => ("", rest),
                        Some(index) => (&rest[..index], &rest[index..]),
                        None => return None,
                    };
                    if !host.is_empty() && host != "localhost" {
                        return None;
                    }
                    let decoded = percent_decode(path)?;
                    #[cfg(windows)]
                    {
                        return None;
                    }
                    #[cfg(not(windows))]
                    {
                        if decoded.starts_with('/') {
                            Some(PathBuf::from(decoded))
                        } else {
                            None
                        }
                    }
                })
            })
            .collect();
        (!files.is_empty()).then_some(files)
    }
}

/// Windows：PowerShell 读取文件拖放列表、位图（PNG base64）与文本。
#[cfg(windows)]
mod windows {
    use super::*;

    pub(super) fn read() -> Result<ClipboardContent, ClipboardError> {
        if let Some(files) = read_file_list() {
            return Ok(ClipboardContent::Files(files));
        }
        if let Some(image) = read_image() {
            return Ok(image);
        }
        let text = run_capture(
            "powershell.exe",
            &[
                "-NonInteractive",
                "-NoProfile",
                "-Sta",
                "-Command",
                "[Console]::Out.Write((Get-Clipboard -Raw))",
            ],
            READ_TIMEOUT,
        )
        .filter(|output| output.success)
        .map(|output| String::from_utf8_lossy(&output.stdout).into_owned())
        .unwrap_or_default();
        if text.is_empty() {
            return Err(ClipboardError::Empty);
        }
        Ok(ClipboardContent::Text(text))
    }

    /// 文件拖放列表。
    fn read_file_list() -> Option<Vec<PathBuf>> {
        let script = "Add-Type -AssemblyName System.Windows.Forms; \
                      $list = [Windows.Forms.Clipboard]::GetFileDropList(); \
                      foreach ($item in $list) { [Console]::Out.WriteLine($item) }";
        let output = run_capture(
            "powershell.exe",
            &["-NonInteractive", "-NoProfile", "-Sta", "-Command", script],
            READ_TIMEOUT,
        )?;
        if !output.success {
            return None;
        }
        let files: Vec<PathBuf> = String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(PathBuf::from)
            .collect();
        (!files.is_empty()).then_some(files)
    }

    /// 位图：`Clipboard::GetImage` 存 PNG 再 base64 输出。
    fn read_image() -> Option<ClipboardContent> {
        let script = "Add-Type -AssemblyName System.Windows.Forms; \
                      $img = [Windows.Forms.Clipboard]::GetImage(); \
                      if ($img) { $ms = New-Object System.IO.MemoryStream; \
                      $img.Save($ms, [System.Drawing.Imaging.ImageFormat]::Png); \
                      [Console]::Out.Write([Convert]::ToBase64String($ms.ToArray())) }";
        let output = run_capture(
            "powershell.exe",
            &["-NonInteractive", "-NoProfile", "-Sta", "-Command", script],
            READ_TIMEOUT,
        )?;
        if !output.success {
            return None;
        }
        let encoded = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if encoded.is_empty() {
            return None;
        }
        let bytes = BASE64.decode(encoded).ok()?;
        let format = sniff_image(&bytes)?;
        Some(ClipboardContent::Image { bytes, format })
    }
}

#[cfg(test)]
mod tests;
