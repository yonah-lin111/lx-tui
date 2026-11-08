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
        ("web", HelpTopic::Web),
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

fn os_args(items: &[&str]) -> Vec<OsString> {
    items.iter().map(OsString::from).collect()
}

#[test]
fn extract_global_opts_parses_cd_json_color_and_preserves_rest() {
    let raw = os_args(&[
        "--json",
        "-C",
        "/tmp/demo",
        "list",
        "--color=never",
        "--limit",
        "10",
    ]);
    let (global, rest, help) = extract_global_opts(&raw).expect("extract global opts");
    assert!(!help);
    assert!(global.json);
    assert_eq!(global.cwd, Some(PathBuf::from("/tmp/demo")));
    assert_eq!(global.color, ColorMode::Never);
    assert_eq!(rest, vec!["list", "--limit", "10"]);
}

#[test]
fn extract_global_opts_stops_at_double_dash_and_skips_value_slots() {
    // `--` 终止全局扫描：后续 token（即使形如全局选项）必须原样交给子命令。
    let raw = os_args(&["todo", "add", "--", "-C", "/tmp/demo", "--json"]);
    let (global, rest, help) = extract_global_opts(&raw).expect("double dash");
    assert!(!help);
    assert!(!global.json);
    assert_eq!(global.cwd, None);
    assert_eq!(rest, vec!["todo", "add", "--", "-C", "/tmp/demo", "--json"]);

    // `--content` 的取值即使叫 `--json`，也不能被全局扫描器吞掉。
    let raw = os_args(&["prompt", "edit", "1", "--content", "--json"]);
    let (global, rest, _) = extract_global_opts(&raw).expect("content value");
    assert!(!global.json);
    assert_eq!(rest, vec!["prompt", "edit", "1", "--content", "--json"]);

    // `--description` 的取值即使叫 `-C`，也不能被当作用户工作目录。
    let raw = os_args(&["prompt", "edit", "1", "--description", "-C"]);
    let (global, rest, _) = extract_global_opts(&raw).expect("description value");
    assert_eq!(global.cwd, None);
    assert_eq!(rest, vec!["prompt", "edit", "1", "--description", "-C"]);

    // 全局标志仍可在 `--` 之前生效，`--` 之后的同名 token 保持字面值。
    let raw = os_args(&["--json", "--", "-C", "/tmp/demo"]);
    let (global, rest, _) = extract_global_opts(&raw).expect("global then separator");
    assert!(global.json);
    assert_eq!(global.cwd, None);
    assert_eq!(rest, vec!["--", "-C", "/tmp/demo"]);
}

#[test]
fn parse_cli_keeps_literal_tokens_after_double_dash() {
    assert!(matches!(
        parse_cli(["lx-tui", "todo", "add", "--", "-C"]).expect("todo literal"),
        CliAction::Todo(_)
    ));
    assert!(matches!(
        parse_cli(["lx-tui", "prompt", "edit", "1", "--content", "--json"])
            .expect("content named like global flag"),
        CliAction::Prompt(_)
    ));
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
fn optional_content_ignores_empty_non_tty_stdin() {
    // P1-5：脚本/CI 中空 stdin 不得被当成显式正文，否则 `skill edit` /
    // `command edit` 仅改 `--description` 时会静默清空已有正文。
    let empty = ContentInput::default();
    assert_eq!(
        empty
            .resolve_with(false, false, || Ok(String::new()))
            .expect("empty piped stdin"),
        None
    );
    assert_eq!(
        empty
            .resolve_with(false, false, || Ok("piped body".to_string()))
            .expect("non-empty piped stdin"),
        Some("piped body".to_string())
    );
    // 必填正文仍必须接受空 stdin（由调用方决定是否拒绝空正文）。
    assert_eq!(
        empty
            .resolve_with(true, false, || Ok(String::new()))
            .expect("required empty stdin"),
        Some(String::new())
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

#[test]
fn web_cli_parses_subcommands_models_thinking_and_session_prefixes() {
    use crate::app::state::AiWebSite;
    use crate::cli::web::WebCommand;

    let models_act =
        parse_cli(["lx-tui", "web", "models", "--site", "gemini", "--json"]).expect("web models");
    assert!(matches!(
        models_act,
        CliAction::Web(ref cli)
            if cli.global.json && cli.command == WebCommand::Models { site: Some(AiWebSite::Gemini) }
    ));

    let chat_act = parse_cli([
        "lx-tui",
        "web",
        "chat",
        "--site",
        "gemini",
        "-m",
        "4 Argon",
        "-t",
        "high",
        "--edit-index",
        "0",
        "--keep-file",
        "report.pdf",
        "updated prompt",
    ])
    .expect("web chat gemini");
    assert!(matches!(
        chat_act,
        CliAction::Web(ref cli)
            if matches!(
                &cli.command,
                WebCommand::Chat {
                    site: AiWebSite::Gemini,
                    model,
                    thinking,
                    prompt,
                    edit_user_index: Some(0),
                    keep_files,
                    ..
                } if model == "4 Argon"
                    && thinking == "high"
                    && prompt == "updated prompt"
                    && keep_files == &vec!["report.pdf".to_string()]
            )
    ));

    // 会话 ID 前缀自动推断站点（`ai-web-gm-*` -> Gemini，`ai-web-ds-*` -> Deepseek）。
    let show_act =
        parse_cli(["lx-tui", "web", "show", "ai-web-gm-abc123", "--thinking"]).expect("web show");
    assert!(matches!(
        show_act,
        CliAction::Web(ref cli)
            if cli.command
                == WebCommand::Show {
                    site: AiWebSite::Gemini,
                    session_id: "ai-web-gm-abc123".into(),
                    remote_session_id: "abc123".into(),
                    show_thinking: true,
                }
    ));

    // 前缀与 `--site` 冲突时必须报错拦截。
    let mismatch = parse_cli([
        "lx-tui",
        "web",
        "show",
        "ai-web-gm-abc123",
        "--site",
        "deepseek",
    ])
    .expect_err("site prefix mismatch");
    assert!(mismatch.contains("belongs to 'gemini'"));

    // 非法模型或思考等级必须报错拦截。
    assert!(
        parse_cli([
            "lx-tui",
            "web",
            "chat",
            "--site",
            "gemini",
            "-m",
            "bad-model",
            "hi"
        ])
        .is_err()
    );
    assert!(
        parse_cli([
            "lx-tui", "web", "chat", "--site", "deepseek", "-t", "high", "hi"
        ])
        .is_err()
    );
}
