//! 顶层 CLI 路由、全局选项提取、帮助手册与多行内容输入单元测试。

use std::ffi::OsString;
use std::path::PathBuf;

use crate::chat::agent::exec::ColorMode;
use crate::cli::args::{ContentInput, ListScopeArg, ScopeArg, extract_global_opts};
use crate::cli::output::mask_secret;
use crate::cli::{CliAction, HelpTopic, help_text, parse_cli};

use super::CliTestEnv;

#[test]
fn top_level_routes_tui_version_and_all_help_topics() {
    assert_eq!(parse_cli(["lx-tui"]).expect("tui"), CliAction::Tui);
    assert_eq!(
        parse_cli(["lx-tui", "--version"]).expect("version"),
        CliAction::Version
    );
    assert_eq!(
        parse_cli(["lx-tui", "-V"]).expect("short version"),
        CliAction::Version
    );
    assert_eq!(
        parse_cli(["lx-tui", "version"]).expect("subcommand version"),
        CliAction::Version
    );
    assert_eq!(
        parse_cli(["lx-tui", "--help"]).expect("root help"),
        CliAction::Help(HelpTopic::Root)
    );

    let topics = [
        ("exec", HelpTopic::Exec),
        ("session", HelpTopic::Session),
        ("prompt", HelpTopic::Prompt),
        ("todo", HelpTopic::Todo),
        ("workspace", HelpTopic::Workspace),
        ("model", HelpTopic::Model),
        ("mcp", HelpTopic::Mcp),
        ("skill", HelpTopic::Skill),
        ("command", HelpTopic::Command),
        ("subagent", HelpTopic::Subagent),
        ("settings", HelpTopic::Settings),
    ];
    for (name, expected) in topics {
        assert_eq!(
            parse_cli(["lx-tui", "help", name]).expect("help topic"),
            CliAction::Help(expected)
        );
        assert_eq!(
            parse_cli(["lx-tui", name, "--help"]).expect("subcommand --help"),
            CliAction::Help(expected)
        );
        let text = help_text(expected);
        assert!(
            text.contains("Usage:"),
            "help text for {name} must contain Usage:"
        );
    }
    assert_eq!(
        parse_cli(["lx-tui", "help", "exec", "resume"]).expect("exec resume help"),
        CliAction::Help(HelpTopic::ExecResume)
    );
}

#[test]
fn top_level_rejects_unknown_subcommand_and_help_topic() {
    let err = parse_cli(["lx-tui", "nonexistent"]).expect_err("unknown top subcommand");
    assert!(err.contains("unexpected argument 'nonexistent'"));

    let help_err = parse_cli(["lx-tui", "help", "bogus"]).expect_err("unknown help topic");
    assert!(help_err.contains("unknown help topic 'bogus'"));
}

#[test]
fn extract_global_opts_parses_cd_json_color_and_preserves_rest() {
    let raw: Vec<OsString> = [
        "--json",
        "-C",
        "/tmp/demo",
        "list",
        "--color=never",
        "--limit",
        "10",
    ]
    .into_iter()
    .map(OsString::from)
    .collect();
    let (global, rest, help) = extract_global_opts(&raw).expect("extract global opts");
    assert!(!help);
    assert!(global.json);
    assert_eq!(global.cwd, Some(PathBuf::from("/tmp/demo")));
    assert_eq!(global.color, ColorMode::Never);
    assert_eq!(rest, vec!["list", "--limit", "10"]);
}

#[test]
fn content_input_supports_inline_file_and_stdin_and_rejects_conflicts() {
    let env = CliTestEnv::new("content-input");
    let file_path = env.write_file("body.md", "file body content");

    let inline = ContentInput {
        inline: Some("inline text".to_string()),
        file: None,
    };
    assert_eq!(
        inline
            .resolve_with(true, true, || Ok("unused".to_string()))
            .expect("inline"),
        Some("inline text".to_string())
    );

    let from_file = ContentInput {
        inline: None,
        file: Some(file_path),
    };
    assert_eq!(
        from_file
            .resolve_with(true, true, || Ok("unused".to_string()))
            .expect("file"),
        Some("file body content".to_string())
    );

    let from_pipe = ContentInput::default();
    assert_eq!(
        from_pipe
            .resolve_with(true, false, || Ok("piped content".to_string()))
            .expect("piped stdin"),
        Some("piped content".to_string())
    );

    let conflict = ContentInput {
        inline: Some("a".to_string()),
        file: Some(PathBuf::from("b")),
    };
    assert!(
        conflict
            .resolve_with(true, true, || Ok(String::new()))
            .is_err()
    );

    let missing = ContentInput::default();
    assert!(
        missing
            .resolve_with(true, true, || Ok(String::new()))
            .is_err()
    );
    assert_eq!(
        missing
            .resolve_with(false, true, || Ok(String::new()))
            .expect("optional"),
        None
    );
}

#[test]
fn scope_args_and_secret_masking_behave_as_expected() {
    assert_eq!(ScopeArg::parse("user").expect("user"), ScopeArg::User);
    assert_eq!(
        ScopeArg::parse("PROJECT").expect("project"),
        ScopeArg::Project
    );
    assert!(ScopeArg::parse("all").is_err());

    assert_eq!(ListScopeArg::parse("all").expect("all"), ListScopeArg::All);
    assert!(ListScopeArg::All.includes_user() && ListScopeArg::All.includes_project());

    assert_eq!(mask_secret(""), "");
    assert_eq!(mask_secret("short123"), "***");
    assert_eq!(mask_secret("sk-1234567890abcd"), "sk-...abcd");
}
