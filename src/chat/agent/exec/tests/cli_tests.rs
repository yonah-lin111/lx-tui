//! `cli.rs` 单元测试：参数解析、stdin 组合与模型目标解析。

use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::chat::agent::exec::cli::{
    CliAction, ColorMode, ExecCli, ExecCommand, HelpTopic, ResumeArgs, help_text, parse_cli,
    prompt_with_stdin_context, resolve_model_target, resolve_prompt_input,
};
use crate::config::ai::{
    AiSettings, ModelConfig, ModelLimit, ModelSelection, ModelTransport, ProviderConfig,
    ProviderOptions, ProviderTransport, WebSearchConfig,
};
use crate::db::chat::ModelSnapshot;

// ---------- parse_cli ----------

fn parse(args: &[&str]) -> Result<CliAction, String> {
    parse_cli(
        std::iter::once("lx-tui")
            .chain(args.iter().copied())
            .map(str::to_string),
    )
}

fn exec(action: CliAction) -> ExecCli {
    match action {
        CliAction::Exec(cli) => cli,
        other => panic!("expected exec action, got {other:?}"),
    }
}

fn parse_exec(args: &[&str]) -> ExecCli {
    exec(parse(args).expect("exec parses"))
}

fn resume_args(action: CliAction) -> ResumeArgs {
    match exec(action).command {
        Some(ExecCommand::Resume(args)) => args,
        other => panic!("expected resume command, got {other:?}"),
    }
}

#[test]
fn no_args_enters_tui() {
    assert_eq!(parse_cli(Vec::<String>::new()), Ok(CliAction::Tui));
    assert_eq!(parse_cli(["lx-tui"]), Ok(CliAction::Tui));
}

#[test]
fn root_help_and_version_flags() {
    for flag in ["-h", "--help"] {
        assert_eq!(parse(&[flag]), Ok(CliAction::Help(HelpTopic::Root)));
    }
    for flag in ["-V", "--version", "version"] {
        assert_eq!(parse(&[flag]), Ok(CliAction::Version));
    }
}

#[test]
fn help_command_topics() {
    assert_eq!(parse(&["help"]), Ok(CliAction::Help(HelpTopic::Root)));
    assert_eq!(
        parse(&["help", "exec"]),
        Ok(CliAction::Help(HelpTopic::Exec))
    );
    assert_eq!(
        parse(&["help", "exec", "resume"]),
        Ok(CliAction::Help(HelpTopic::ExecResume))
    );
    assert_eq!(
        parse(&["help", "resume"]),
        Ok(CliAction::Help(HelpTopic::ExecResume))
    );
    assert!(parse(&["help", "nope"]).is_err());
    assert!(parse(&["help", "exec", "resume", "extra"]).is_err());
}

#[test]
fn exec_help_topics() {
    assert_eq!(parse(&["exec", "-h"]), Ok(CliAction::Help(HelpTopic::Exec)));
    assert_eq!(
        parse(&["exec", "--help"]),
        Ok(CliAction::Help(HelpTopic::Exec))
    );
    assert_eq!(
        parse(&["exec", "resume", "--help"]),
        Ok(CliAction::Help(HelpTopic::ExecResume))
    );
    assert_eq!(
        parse(&["exec", "resume", "sess-1", "-h"]),
        Ok(CliAction::Help(HelpTopic::ExecResume))
    );
}

#[test]
fn exec_without_flags_uses_defaults() {
    assert_eq!(parse_exec(&["exec"]), ExecCli::default());
    let cli = parse_exec(&["exec", "hello"]);
    assert_eq!(cli.prompt.as_deref(), Some("hello"));
    assert_eq!(cli.color, ColorMode::Auto);
    assert!(!cli.json);
    assert!(!cli.ephemeral);
    assert!(cli.images.is_empty());
    assert!(cli.command.is_none());
}

#[test]
fn exec_parses_all_flags() {
    let cli = parse_exec(&[
        "exec",
        "-C",
        "/tmp/work",
        "-m",
        "anthropic/claude#high",
        "-o",
        "last.txt",
        "-i",
        "a.png",
        "--image",
        "b.png",
        "--json",
        "--ephemeral",
        "--color",
        "never",
        "fix the bug",
    ]);
    assert_eq!(cli.cwd, Some(PathBuf::from("/tmp/work")));
    assert_eq!(cli.model.as_deref(), Some("anthropic/claude#high"));
    assert_eq!(cli.last_message_file, Some(PathBuf::from("last.txt")));
    assert_eq!(
        cli.images,
        vec![PathBuf::from("a.png"), PathBuf::from("b.png")]
    );
    assert!(cli.json);
    assert!(cli.ephemeral);
    assert_eq!(cli.color, ColorMode::Never);
    assert_eq!(cli.prompt.as_deref(), Some("fix the bug"));
    assert!(cli.command.is_none());
}

#[test]
fn exec_parses_inline_option_values() {
    let cli = parse_exec(&[
        "exec",
        "--cd=/tmp/x",
        "--model=openai/gpt-5#low",
        "--output-last-message=out.txt",
        "--image=a.png,b.png",
        "--color=always",
        "go",
    ]);
    assert_eq!(cli.cwd, Some(PathBuf::from("/tmp/x")));
    assert_eq!(cli.model.as_deref(), Some("openai/gpt-5#low"));
    assert_eq!(cli.last_message_file, Some(PathBuf::from("out.txt")));
    assert_eq!(
        cli.images,
        vec![PathBuf::from("a.png"), PathBuf::from("b.png")]
    );
    assert_eq!(cli.color, ColorMode::Always);
    assert_eq!(cli.prompt.as_deref(), Some("go"));
}

#[test]
fn repeated_and_comma_separated_images_accumulate() {
    let cli = parse_exec(&["exec", "-i", "a.png,b.png", "--image", "c.png", "go"]);
    assert_eq!(
        cli.images,
        vec![
            PathBuf::from("a.png"),
            PathBuf::from("b.png"),
            PathBuf::from("c.png")
        ]
    );
}

#[test]
fn comma_separated_images_trim_padding() {
    let cli = parse_exec(&["exec", "-i", "a.png, b.png ,  c.png ", "go"]);
    assert_eq!(
        cli.images,
        vec![
            PathBuf::from("a.png"),
            PathBuf::from("b.png"),
            PathBuf::from("c.png")
        ]
    );
}

#[test]
fn last_with_a_session_id_prompt_is_rejected() {
    let expected = "cannot specify both '--last' and a session ID";
    assert_eq!(
        parse(&["exec", "resume", "--last", "sess-123"]).expect_err("conflict"),
        expected
    );
    // `--last` 之后不以 `sess-` 开头的文本仍是普通提示词。
    let args = resume_args(
        parse(&["exec", "resume", "--last", "continue"]).expect("prompt is not a session id"),
    );
    assert_eq!(args.prompt.as_deref(), Some("continue"));
}

#[test]
fn dash_prompt_is_kept_literal() {
    let cli = parse_exec(&["exec", "-"]);
    assert_eq!(cli.prompt.as_deref(), Some("-"));
}

#[test]
fn double_dash_makes_flags_literal_prompt() {
    let cli = parse_exec(&["exec", "--", "-m", "not-a-flag"]);
    assert_eq!(cli.prompt.as_deref(), Some("-m not-a-flag"));
    assert_eq!(cli.model, None);

    let cli = parse_exec(&["exec", "-m", "p/m", "--", "--json"]);
    assert_eq!(cli.model.as_deref(), Some("p/m"));
    assert_eq!(cli.prompt.as_deref(), Some("--json"));
    assert!(!cli.json);
}

#[test]
fn double_dash_after_resume_session_keeps_prompt_literal() {
    let args = resume_args(
        parse(&["exec", "resume", "sess-1", "--", "--weird", "prompt"]).expect("resume parses"),
    );
    assert_eq!(args.session_id.as_deref(), Some("sess-1"));
    assert_eq!(args.prompt.as_deref(), Some("--weird prompt"));
}

#[test]
fn resume_parses_session_id_and_prompt() {
    let args =
        resume_args(parse(&["exec", "resume", "sess-123", "continue"]).expect("resume parses"));
    assert_eq!(
        args,
        ResumeArgs {
            session_id: Some("sess-123".to_string()),
            last: false,
            prompt: Some("continue".to_string()),
            images: Vec::new(),
        }
    );
}

#[test]
fn resume_session_without_prompt_reads_stdin_later() {
    let args = resume_args(parse(&["exec", "resume", "sess-123"]).expect("resume parses"));
    assert_eq!(args.session_id.as_deref(), Some("sess-123"));
    assert_eq!(args.prompt, None);
}

#[test]
fn resume_last_takes_single_prompt() {
    let args =
        resume_args(parse(&["exec", "resume", "--last", "continue"]).expect("resume parses"));
    assert!(args.last);
    assert_eq!(args.session_id, None);
    assert_eq!(args.prompt.as_deref(), Some("continue"));
}

#[test]
fn resume_accepts_global_flags_after_subcommand() {
    let cli = parse_exec(&[
        "exec", "resume", "sess-1", "-C", "/w", "-m", "p/m", "-o", "out.txt", "--json", "--color",
        "never", "-i", "pic.png", "go",
    ]);
    assert_eq!(cli.cwd, Some(PathBuf::from("/w")));
    assert_eq!(cli.model.as_deref(), Some("p/m"));
    assert_eq!(cli.last_message_file, Some(PathBuf::from("out.txt")));
    assert!(cli.json);
    assert_eq!(cli.color, ColorMode::Never);
    assert!(cli.images.is_empty(), "resume 模式下图片归入 ResumeArgs");
    let args = match cli.command {
        Some(ExecCommand::Resume(args)) => args,
        other => panic!("expected resume, got {other:?}"),
    };
    assert_eq!(args.session_id.as_deref(), Some("sess-1"));
    assert_eq!(args.prompt.as_deref(), Some("go"));
    assert_eq!(args.images, vec![PathBuf::from("pic.png")]);
}

#[test]
fn resume_accepts_global_flags_before_subcommand() {
    let cli = parse_exec(&[
        "exec", "--json", "-m", "p/m", "-C", "/w", "resume", "--last", "go",
    ]);
    assert!(cli.json);
    assert_eq!(cli.model.as_deref(), Some("p/m"));
    assert_eq!(cli.cwd, Some(PathBuf::from("/w")));
    let args = match cli.command {
        Some(ExecCommand::Resume(args)) => args,
        other => panic!("expected resume, got {other:?}"),
    };
    assert!(args.last);
    assert_eq!(args.prompt.as_deref(), Some("go"));
}

#[test]
fn images_before_resume_move_into_resume_args() {
    let cli = parse_exec(&[
        "exec",
        "-i",
        "before.png",
        "resume",
        "-i",
        "after.png",
        "--last",
    ]);
    assert!(cli.images.is_empty());
    let args = match cli.command {
        Some(ExecCommand::Resume(args)) => args,
        other => panic!("expected resume, got {other:?}"),
    };
    assert_eq!(
        args.images,
        vec![PathBuf::from("before.png"), PathBuf::from("after.png")]
    );
}

#[test]
fn ephemeral_with_resume_is_rejected() {
    let error = parse(&["exec", "resume", "sess-1", "--ephemeral", "go"]).expect_err("conflict");
    assert_eq!(error, "--ephemeral cannot be combined with 'resume'");
}

#[test]
fn resume_requires_session_id_or_last() {
    let expected = "Specify a session ID or pass '--last' to resume the most recent session.";
    assert_eq!(parse(&["exec", "resume"]).expect_err("missing"), expected);
    assert_eq!(
        parse(&["exec", "resume", "--json"]).expect_err("missing"),
        expected
    );
}

#[test]
fn last_and_session_id_are_mutually_exclusive() {
    let expected = "cannot specify both '--last' and a session ID";
    assert_eq!(
        parse(&["exec", "resume", "--last", "one", "two"]).expect_err("conflict"),
        expected
    );
    assert_eq!(
        parse(&["exec", "resume", "sess-1", "--last"]).expect_err("conflict"),
        expected
    );
}

#[test]
fn last_outside_resume_is_rejected() {
    let error = parse(&["exec", "--last"]).expect_err("no resume");
    assert!(error.contains("'--last' can only be used with 'resume'"));
}

#[test]
fn unknown_option_is_rejected() {
    let error = parse(&["exec", "--nope"]).expect_err("unknown flag");
    assert!(error.contains("unknown option '--nope'"), "{error}");
    let error = parse(&["bogus"]).expect_err("unknown command");
    assert!(error.contains("unexpected argument 'bogus'"), "{error}");
}

#[test]
fn missing_option_values_are_rejected() {
    for args in [
        vec!["exec", "-C"],
        vec!["exec", "-m"],
        vec!["exec", "-o"],
        vec!["exec", "-i"],
        vec!["exec", "--color"],
    ] {
        let error = parse(&args).expect_err("missing value");
        assert!(error.starts_with("missing value for '"), "{error}");
    }
}

#[test]
fn empty_option_values_are_rejected() {
    let error = parse(&["exec", "--model="]).expect_err("empty value");
    assert_eq!(error, "empty value for '--model'");
    let error = parse(&["exec", "--image="]).expect_err("empty value");
    assert_eq!(error, "empty value for '--image'");
}

#[test]
fn invalid_color_value_is_rejected() {
    let error = parse(&["exec", "--color", "rainbow", "go"]).expect_err("bad color");
    assert!(error.contains("invalid value 'rainbow'"), "{error}");
}

#[test]
fn second_positional_prompt_is_rejected() {
    let error = parse(&["exec", "one", "two"]).expect_err("two prompts");
    assert!(error.contains("unexpected argument 'two'"), "{error}");
}

#[test]
fn resume_after_prompt_is_not_a_subcommand() {
    let error = parse(&["exec", "hello", "resume"]).expect_err("second positional");
    assert!(error.contains("unexpected argument 'resume'"), "{error}");
}

// ---------- help_text ----------

#[test]
fn help_texts_are_topic_specific() {
    assert!(help_text(HelpTopic::Root).contains("Usage:"));
    assert!(help_text(HelpTopic::Exec).contains("--output-last-message"));
    assert!(help_text(HelpTopic::ExecResume).contains("--last"));
    assert_ne!(help_text(HelpTopic::Exec), help_text(HelpTopic::ExecResume));
}

// ---------- ColorMode ----------

#[test]
fn color_mode_parsing_and_decisions() {
    assert_eq!(ColorMode::parse("auto"), Some(ColorMode::Auto));
    assert_eq!(ColorMode::parse("ALWAYS"), Some(ColorMode::Always));
    assert_eq!(ColorMode::parse(" never "), Some(ColorMode::Never));
    assert_eq!(ColorMode::parse("sometimes"), None);

    assert!(!ColorMode::Auto.should_color(false));
    assert!(ColorMode::Auto.should_color(true));
    assert!(ColorMode::Always.should_color(false));
    assert!(!ColorMode::Never.should_color(true));

    assert!(!ColorMode::Auto.should_color_with_env(true, true));
    assert!(ColorMode::Auto.should_color_with_env(true, false));
    assert!(ColorMode::Always.should_color_with_env(false, true));
    assert!(!ColorMode::Never.should_color_with_env(true, false));
}

// ---------- stdin / prompt ----------

#[test]
fn stdin_context_wraps_non_empty_pipes() {
    assert_eq!(prompt_with_stdin_context("fix", ""), "fix");
    assert_eq!(prompt_with_stdin_context("fix", "  \n"), "fix");
    assert_eq!(
        prompt_with_stdin_context("fix", "line"),
        "fix\n\n<stdin>\nline\n</stdin>"
    );
    assert_eq!(
        prompt_with_stdin_context("fix", "line\n"),
        "fix\n\n<stdin>\nline\n</stdin>"
    );
}

#[test]
fn positional_prompt_on_terminal_skips_stdin() {
    let mut read = false;
    let prompt = resolve_prompt_input(Some("fix it"), true, || {
        read = true;
        Ok("ignored".to_string())
    })
    .expect("prompt");
    assert_eq!(prompt, "fix it");
    assert!(!read);
}

#[test]
fn piped_stdin_supplies_missing_prompt() {
    let prompt =
        resolve_prompt_input(None, false, || Ok("from stdin\n".to_string())).expect("stdin prompt");
    assert_eq!(prompt, "from stdin\n");
}

#[test]
fn dash_forces_stdin_even_on_terminal() {
    let prompt =
        resolve_prompt_input(Some("-"), true, || Ok("forced\n".to_string())).expect("stdin prompt");
    assert_eq!(prompt, "forced\n");
}

#[test]
fn positional_prompt_combines_with_piped_stdin() {
    let prompt = resolve_prompt_input(Some("summarize"), false, || Ok("file body".to_string()))
        .expect("combined prompt");
    assert_eq!(prompt, "summarize\n\n<stdin>\nfile body\n</stdin>");
}

#[test]
fn positional_prompt_with_empty_pipe_stays_alone() {
    let prompt = resolve_prompt_input(Some("go"), false, || Ok(String::new())).expect("prompt");
    assert_eq!(prompt, "go");
}

#[test]
fn empty_or_missing_prompt_is_rejected() {
    assert_eq!(
        resolve_prompt_input(Some("   "), true, || Ok(String::new())).expect_err("empty"),
        "Prompt must not be empty."
    );
    assert_eq!(
        resolve_prompt_input(None, true, || Ok(String::new())).expect_err("tty"),
        "No prompt provided. Either specify one as an argument or pipe the prompt into stdin."
    );
    assert_eq!(
        resolve_prompt_input(Some("-"), true, || Ok(" \n".to_string())).expect_err("empty stdin"),
        "No prompt provided on stdin."
    );
    assert_eq!(
        resolve_prompt_input(None, false, || Ok(String::new())).expect_err("empty pipe"),
        "No prompt provided on stdin."
    );
}

#[test]
fn stdin_read_failure_is_reported() {
    let error = resolve_prompt_input(None, false, || Err(std::io::Error::other("boom")))
        .expect_err("read failure");
    assert_eq!(error, "failed to read stdin: boom");
}

// ---------- resolve_model_target ----------

fn ai_settings(
    providers: Vec<ProviderConfig>,
    enabled: &[&str],
    default: Option<(&str, &str, Option<&str>)>,
) -> AiSettings {
    AiSettings {
        enabled_providers: enabled.iter().map(|id| (*id).to_string()).collect(),
        providers: providers
            .into_iter()
            .map(|provider| (provider.id.clone(), provider))
            .collect(),
        default_model: default.map(|(provider, model, variant)| ModelSelection {
            provider: provider.to_string(),
            model: model.to_string(),
            variant: variant.map(str::to_string),
        }),
        title_summary: None,
        title_language: None,
        stream_idle_timeout_ms: Some(4_321),
        web_search: WebSearchConfig::default(),
        extra: serde_json::Map::new(),
    }
}

fn base_model(id: &str) -> ModelConfig {
    ModelConfig {
        id: id.to_string(),
        name: format!("{id} display"),
        transport: None,
        limit: None,
        modalities: None,
        variants: BTreeMap::new(),
        variant: None,
        pricing: None,
        extra: serde_json::Map::new(),
    }
}

fn with_variants(mut config: ModelConfig, variants: &[&str], default: Option<&str>) -> ModelConfig {
    config.variants = variants
        .iter()
        .map(|variant| {
            (
                (*variant).to_string(),
                serde_json::json!({ "level": variant }),
            )
        })
        .collect();
    config.variant = default.map(str::to_string);
    config
}

fn provider(
    id: &str,
    kind: ProviderTransport,
    api_key: Option<&str>,
    models: Vec<ModelConfig>,
) -> ProviderConfig {
    ProviderConfig {
        id: id.to_string(),
        kind,
        name: format!("{id} name"),
        options: ProviderOptions {
            api_key: api_key.map(str::to_string),
            base_url: format!("https://{id}.example/v1"),
        },
        models: models
            .into_iter()
            .map(|config| (config.id.clone(), config))
            .collect(),
        extra: serde_json::Map::new(),
    }
}

fn sample_settings() -> AiSettings {
    let mut claude = with_variants(base_model("claude-sonnet-4"), &["high", "low"], Some("low"));
    claude.limit = Some(ModelLimit {
        context: 200_000,
        output: 8_192,
    });
    claude.modalities = Some(serde_json::json!({ "input": ["text", "image"] }));
    let mut gpt = base_model("gpt-5");
    gpt.transport = Some(ModelTransport::OpenaiResponses);
    ai_settings(
        vec![
            provider(
                "anthropic",
                ProviderTransport::Anthropic,
                Some("sk-ant"),
                vec![claude],
            ),
            provider(
                "openai",
                ProviderTransport::Openai,
                Some("sk-openai"),
                vec![gpt],
            ),
        ],
        &["anthropic", "openai"],
        Some(("anthropic", "claude-sonnet-4", None)),
    )
}

#[test]
fn explicit_provider_model_variant_is_frozen() {
    let target = resolve_model_target(
        &sample_settings(),
        Some("anthropic/claude-sonnet-4#high"),
        None,
    )
    .expect("resolves");
    assert_eq!(target.provider, "anthropic");
    assert_eq!(target.model, "claude-sonnet-4");
    assert_eq!(target.variant.as_deref(), Some("high"));
    assert_eq!(target.display_name, "claude-sonnet-4 display");
    assert_eq!(target.transport, "anthropic");
    assert_eq!(target.base_url, "https://anthropic.example/v1");
    assert_eq!(target.api_key, "sk-ant");
    assert_eq!(target.context_limit, Some(200_000));
    assert_eq!(target.output_limit, Some(8_192));
    assert_eq!(
        target.variant_params,
        Some(serde_json::json!({ "level": "high" }))
    );
    assert_eq!(target.idle_timeout_ms, 4_321);
    assert!(target.supports_image_input);
    assert_eq!(
        target.snapshot(),
        ModelSnapshot {
            provider: "anthropic".to_string(),
            model: "claude-sonnet-4".to_string(),
            variant: Some("high".to_string()),
        }
    );
}

#[test]
fn explicit_provider_model_without_variant_uses_model_default() {
    let target = resolve_model_target(&sample_settings(), Some("anthropic/claude-sonnet-4"), None)
        .expect("resolves");
    assert_eq!(target.variant.as_deref(), Some("low"));
    assert_eq!(
        target.variant_params,
        Some(serde_json::json!({ "level": "low" }))
    );
}

#[test]
fn unknown_explicit_variant_is_rejected() {
    let error = resolve_model_target(
        &sample_settings(),
        Some("anthropic/claude-sonnet-4#max"),
        None,
    )
    .expect_err("unknown variant");
    assert!(error.contains("unknown variant 'max'"), "{error}");
}

#[test]
fn short_model_id_uses_matching_default_provider() {
    let target = resolve_model_target(&sample_settings(), Some("claude-sonnet-4#high"), None)
        .expect("short");
    assert_eq!(target.provider, "anthropic");
    assert_eq!(target.variant.as_deref(), Some("high"));
}

#[test]
fn short_model_id_searches_enabled_providers_in_order() {
    let settings = ai_settings(
        vec![
            provider(
                "zeta",
                ProviderTransport::Openai,
                Some("sk-z"),
                vec![base_model("shared"), base_model("z-other")],
            ),
            provider(
                "alpha",
                ProviderTransport::Openai,
                Some("sk-a"),
                vec![base_model("shared"), base_model("a-other")],
            ),
        ],
        &["zeta", "alpha"],
        Some(("alpha", "a-other", None)),
    );
    let target = resolve_model_target(&settings, Some("shared"), None).expect("shared");
    assert_eq!(target.provider, "zeta");
}

#[test]
fn explicit_provider_must_be_enabled() {
    let mut settings = sample_settings();
    settings.enabled_providers = vec!["openai".to_string()];
    let error = resolve_model_target(&settings, Some("anthropic/claude-sonnet-4"), None)
        .expect_err("disabled");
    assert!(error.contains("not enabled"), "{error}");
}

#[test]
fn explicit_provider_requires_api_key() {
    let settings = ai_settings(
        vec![provider(
            "local",
            ProviderTransport::OpenaiCompatible,
            None,
            vec![base_model("m")],
        )],
        &["local"],
        Some(("local", "m", None)),
    );
    let error = resolve_model_target(&settings, Some("local/m"), None).expect_err("no key");
    assert!(error.contains("no API key configured"), "{error}");
}

#[test]
fn unknown_provider_and_model_are_reported() {
    let error = resolve_model_target(&sample_settings(), Some("ghost/m"), None)
        .expect_err("unknown provider");
    assert!(error.contains("provider 'ghost' is not defined"), "{error}");

    let error = resolve_model_target(&sample_settings(), Some("anthropic/nope"), None)
        .expect_err("unknown model");
    assert!(
        error.contains("model 'nope' is not defined on provider 'anthropic'"),
        "{error}"
    );
}

#[test]
fn malformed_model_references_are_rejected() {
    let settings = sample_settings();
    let error = resolve_model_target(&settings, Some(""), None).expect_err("empty");
    assert!(error.contains("model must not be empty"), "{error}");
    let error = resolve_model_target(&settings, Some("#high"), None).expect_err("no model");
    assert!(error.contains("missing model id"), "{error}");
    let error =
        resolve_model_target(&settings, Some("anthropic/claude#"), None).expect_err("no variant");
    assert!(error.contains("missing variant"), "{error}");
}

#[test]
fn resumed_session_model_wins_over_default() {
    let target = resolve_model_target(&sample_settings(), None, Some(("openai", "gpt-5", None)))
        .expect("session model");
    assert_eq!(target.provider, "openai");
    assert_eq!(target.model, "gpt-5");
    assert_eq!(target.transport, "openai-responses");
    assert_eq!(target.variant, None);
    assert_eq!(target.variant_params, None);
}

#[test]
fn unknown_session_variant_falls_back_to_model_default() {
    let target = resolve_model_target(
        &sample_settings(),
        None,
        Some(("anthropic", "claude-sonnet-4", Some("gone"))),
    )
    .expect("session model");
    assert_eq!(target.variant.as_deref(), Some("low"));
}

#[test]
fn invalid_session_model_falls_back_to_default() {
    let target = resolve_model_target(
        &sample_settings(),
        None,
        Some(("missing-provider", "m", None)),
    )
    .expect("default");
    assert_eq!(target.provider, "anthropic");
    assert_eq!(target.model, "claude-sonnet-4");
}

#[test]
fn default_model_is_used_when_no_overrides() {
    let target = resolve_model_target(&sample_settings(), None, None).expect("default");
    assert_eq!(target.provider, "anthropic");
    assert_eq!(target.model, "claude-sonnet-4");
    assert_eq!(target.variant.as_deref(), Some("low"));
}

#[test]
fn first_available_model_is_the_final_fallback() {
    let settings = ai_settings(
        vec![provider(
            "openai",
            ProviderTransport::Openai,
            Some("sk"),
            vec![base_model("first"), base_model("second")],
        )],
        &["openai"],
        None,
    );
    let target = resolve_model_target(&settings, None, None).expect("fallback");
    assert_eq!(target.provider, "openai");
    assert_eq!(target.model, "first");
}

#[test]
fn no_configured_model_is_rejected() {
    let error = resolve_model_target(&AiSettings::empty(), None, None).expect_err("no model");
    assert_eq!(
        error,
        "No configured AI model available. Configure a provider and API key in settings or ai.json."
    );
}

#[test]
fn provider_without_models_falls_through_to_error() {
    let settings = ai_settings(
        vec![provider(
            "empty",
            ProviderTransport::Openai,
            Some("sk"),
            Vec::new(),
        )],
        &["empty"],
        None,
    );
    let error = resolve_model_target(&settings, None, None).expect_err("no models");
    assert!(
        error.starts_with("No configured AI model available"),
        "{error}"
    );
}

#[test]
fn empty_model_name_falls_back_to_model_id() {
    let mut config = base_model("m");
    config.name = String::new();
    let settings = ai_settings(
        vec![provider(
            "p",
            ProviderTransport::Openai,
            Some("sk"),
            vec![config],
        )],
        &["p"],
        Some(("p", "m", None)),
    );
    let target = resolve_model_target(&settings, None, None).expect("model");
    assert_eq!(target.display_name, "m");
}
