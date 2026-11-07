use super::*;

#[test]
fn parse_path_text_accepts_absolute_lines() {
    let parsed = parse_path_text("/tmp/a b.png\n/tmp/中文 目录").expect("paths");
    assert_eq!(
        parsed,
        vec![
            std::path::PathBuf::from("/tmp/a b.png"),
            std::path::PathBuf::from("/tmp/中文 目录")
        ]
    );
}

#[test]
fn parse_path_text_rejects_empty_lines_and_relative() {
    assert_eq!(parse_path_text("/tmp/a\n\n/tmp/b"), None);
    assert_eq!(parse_path_text("relative/path"), None);
    assert_eq!(parse_path_text(""), None);
    assert_eq!(parse_path_text("/tmp/a\nrelative"), None);
}

#[test]
fn parse_path_text_keeps_inner_and_edge_spaces() {
    let parsed = parse_path_text("/tmp/a  b\n").expect("path");
    assert_eq!(parsed, vec![std::path::PathBuf::from("/tmp/a  b")]);
}

#[cfg(not(windows))]
#[test]
fn parse_file_uri_decodes_percent_and_localhost() {
    let parsed = parse_path_text("file:///tmp/a%20b.png").expect("uri");
    assert_eq!(parsed, vec![std::path::PathBuf::from("/tmp/a b.png")]);
    let parsed = parse_path_text("file://localhost/tmp/中文").expect("uri");
    assert_eq!(parsed, vec![std::path::PathBuf::from("/tmp/中文")]);
}

#[test]
fn parse_file_uri_rejects_remote_host_and_bad_escapes() {
    assert_eq!(parse_path_text("file://example.com/tmp/a"), None);
    assert_eq!(parse_path_text("file:///tmp/%2"), None);
    assert_eq!(parse_path_text("file:///tmp/%zz"), None);
    assert_eq!(parse_path_text("/tmp/a\0b"), None);
}

#[test]
fn is_image_matches_known_signatures() {
    assert!(is_image(&[
        0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0x00
    ]));
    assert!(is_image(&[0xFF, 0xD8, 0xFF, 0xE0]));
    assert!(is_image(b"GIF89a...."));
    assert!(is_image(b"RIFF\x00\x00\x00\x00WEBPVP8 "));
    assert!(is_image(b"BM....."));
    assert!(is_image(b"II\x2a\x00"));
    assert!(is_image(b"MM\x00\x2a"));
    assert!(!is_image(b"plain text"));
    assert!(!is_image(b"RIFF....."));
}

#[test]
fn sniff_image_file_reads_header_and_ignores_missing() {
    let dir = std::env::temp_dir().join("lx-tui-sniff-test");
    std::fs::create_dir_all(&dir).expect("temp dir");
    let png = dir.join("a.png");
    std::fs::write(&png, [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]).expect("write png");
    let text = dir.join("a.txt");
    std::fs::write(&text, b"hello").expect("write txt");
    assert!(sniff_image_file(&png));
    assert!(!sniff_image_file(&text));
    assert!(!sniff_image_file(&dir.join("missing.png")));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn resolve_pasted_path_prefers_literal_then_unescapes_shell_escapes() {
    let dir = std::env::temp_dir().join("lx-tui-resolve-pasted");
    std::fs::create_dir_all(&dir).expect("temp dir");
    let plain = dir.join("plain.md");
    std::fs::write(&plain, b"x").expect("plain file");
    let real = dir.join("ai_agent_api(1).md");
    std::fs::write(&real, b"x").expect("paren file");
    let escaped = std::path::PathBuf::from(format!("{}/ai_agent_api\\(1\\).md", dir.display()));

    assert_eq!(
        crate::platform::resolve_pasted_path(&plain),
        Some(plain.clone()),
        "原文存在时原样返回"
    );
    assert!(!escaped.exists(), "转义形式本身不是文件");
    assert_eq!(
        crate::platform::resolve_pasted_path(&escaped),
        Some(real.clone()),
        "shell 转义路径回落为真实路径"
    );

    let backslash = dir.join("literal\\(1).md");
    std::fs::write(&backslash, b"x").expect("backslash file");
    assert_eq!(
        crate::platform::resolve_pasted_path(&backslash),
        Some(backslash.clone()),
        "文件名真含反斜杠时优先原文"
    );
    assert_eq!(
        crate::platform::resolve_pasted_path(std::path::Path::new("/nope/\\a.md")),
        None
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn normalize_strips_shell_escapes_and_drag_trailing_space() {
    assert_eq!(
        normalize_pasted_path_input("/Users/yonah/Desktop/test\\ one"),
        "/Users/yonah/Desktop/test one"
    );
    assert_eq!(
        normalize_pasted_path_input("/Users/yonah/Desktop/test\\ one "),
        "/Users/yonah/Desktop/test one",
        "终端拖拽附带的尾随空格必须丢弃"
    );
    assert_eq!(
        normalize_pasted_path_input("~/Desktop/test\\ one"),
        "~/Desktop/test one"
    );
    assert_eq!(
        normalize_pasted_path_input("/Users/yonah/Desktop/ai\\(1\\).md"),
        "/Users/yonah/Desktop/ai(1).md"
    );
    assert_eq!(
        normalize_pasted_path_input("/Users/yonah/Desktop/test\\ "),
        "/Users/yonah/Desktop/test ",
        "被转义的尾随空格属于路径本身，必须保留"
    );
    assert_eq!(normalize_pasted_path_input(" \t\r\n"), "");
    assert_eq!(normalize_pasted_path_input(""), "");
}

#[test]
fn normalize_strips_posix_single_quotes() {
    assert_eq!(
        normalize_pasted_path_input("'/Users/yonah/Desktop/test one'"),
        "/Users/yonah/Desktop/test one"
    );
    assert_eq!(
        normalize_pasted_path_input("'/Users/yonah/Desktop/test one' "),
        "/Users/yonah/Desktop/test one"
    );
    assert_eq!(
        normalize_pasted_path_input("'/Users/yonah/Desktop/it'\\''s one'"),
        "/Users/yonah/Desktop/it's one",
        "POSIX 单引号内的转义序列必须还原"
    );
}

#[test]
fn normalize_strips_double_quotes_and_keeps_windows_separators() {
    assert_eq!(
        normalize_pasted_path_input("\"/Users/yonah/Desktop/test one\""),
        "/Users/yonah/Desktop/test one"
    );
    assert_eq!(
        normalize_pasted_path_input("\"C:\\Users\\yonah\\Desktop\\test one\""),
        "C:\\Users\\yonah\\Desktop\\test one",
        "Windows 原生反斜杠路径保留分隔符"
    );
    assert_eq!(
        normalize_pasted_path_input("\"/tmp/a\\\\b\""),
        "/tmp/a\\b",
        "非 Windows 路径的双引号转义还原为单个反斜杠"
    );
    assert_eq!(
        normalize_pasted_path_input("\"\\\\server\\share\""),
        "\\\\server\\share",
        "UNC 路径保持反斜杠分隔符"
    );
}

#[cfg(not(windows))]
#[test]
fn normalize_decodes_file_uris() {
    assert_eq!(
        normalize_pasted_path_input("file:///Users/yonah/Desktop/test%20one"),
        "/Users/yonah/Desktop/test one"
    );
    assert_eq!(
        normalize_pasted_path_input("file://localhost/Users/yonah/Desktop/test%20one"),
        "/Users/yonah/Desktop/test one"
    );
    assert_eq!(
        normalize_pasted_path_input("'file:///Users/yonah/Desktop/test%20one'"),
        "/Users/yonah/Desktop/test one"
    );
}

#[cfg(not(windows))]
#[test]
fn normalize_keeps_existing_literal_backslash_name() {
    let dir = std::env::temp_dir().join("lx-tui-normalize-literal");
    std::fs::create_dir_all(&dir).expect("temp dir");
    let literal = dir.join("test\\one");
    std::fs::write(&literal, b"x").expect("literal backslash file");
    let text = literal.display().to_string();
    assert_eq!(
        normalize_pasted_path_input(&text),
        text,
        "文件名真含反斜杠时优先原文"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(not(windows))]
#[test]
fn parse_path_text_accepts_quoted_escaped_and_file_uri_paths() {
    let dir = std::env::temp_dir().join("lx-tui-paste-forms");
    let target = dir.join("test one");
    std::fs::create_dir_all(&target).expect("temp dir");
    let display = target.display().to_string();
    let escaped = format!("{} ", display.replace(' ', "\\ "));
    let single = format!("'{display}'");
    let double = format!("\"{display}\"");
    let uri = format!("file://{}", display.replace(' ', "%20"));

    for input in [&escaped, &single, &double, &uri] {
        let parsed = parse_path_text(input).unwrap_or_else(|| panic!("解析失败: {input}"));
        assert_eq!(parsed.len(), 1, "input={input}");
        assert_eq!(
            crate::platform::resolve_pasted_path(&parsed[0]),
            Some(target.clone()),
            "input={input}"
        );
    }
    // 引号与 URI 形式解析后直接落到真实路径；转义形式由解析后的路径再归一化。
    for input in [&single, &double, &uri] {
        assert_eq!(
            parse_path_text(input),
            Some(vec![target.clone()]),
            "input={input}"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}
