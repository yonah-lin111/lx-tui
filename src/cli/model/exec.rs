//! `lx-tui model` 子命令执行与远端模型发现。

use super::*;

mod remote;
use remote::{run_config, run_fetch};

pub fn run(cli: ModelCli) -> Result<i32, Box<dyn std::error::Error>> {
    let out = Output::new(cli.global.json, cli.global.color);
    match cli.command {
        ModelCommand::List { all } => run_list(&out, all)?,
        ModelCommand::Show { target } => run_show(&out, target.as_deref())?,
        ModelCommand::Default {
            selection,
            clear,
            title_summary,
        } => run_default(&out, selection.as_deref(), clear, title_summary)?,
        ModelCommand::ProviderSet(args) => run_provider_set(&out, &args)?,
        ModelCommand::ProviderRm { provider } => run_provider_rm(&out, &provider)?,
        ModelCommand::ModelSet(args) => run_model_set(&out, &args)?,
        ModelCommand::ModelRm { target } => run_model_rm(&out, &target)?,
        ModelCommand::Fetch(args) => run_fetch(&out, &args)?,
        ModelCommand::Config(args) => run_config(&out, &args)?,
    }
    Ok(EXIT_OK)
}
fn ai_path() -> Result<PathBuf, String> {
    crate::config::ai::ai_config_path().map_err(|error| error.to_string())
}
fn load_settings() -> Result<AiSettings, String> {
    match crate::config::ai::load_ai(&ai_path()?).map_err(|error| error.to_string())? {
        Some(settings) => Ok(settings),
        None => Ok(AiSettings::empty()),
    }
}
fn save_settings(settings: &AiSettings) -> Result<(), String> {
    crate::config::ai::save_ai(&ai_path()?, settings).map_err(|error| error.to_string())
}
fn is_enabled(settings: &AiSettings, provider: &str) -> bool {
    settings.enabled_providers.iter().any(|id| id == provider)
}
fn run_list(out: &Output, all: bool) -> Result<(), String> {
    let settings = load_settings()?;
    let (mut rows, mut providers) = (Vec::new(), Vec::new());
    for (id, provider) in &settings.providers {
        let enabled = is_enabled(&settings, id);
        if !all && !enabled {
            continue;
        }
        let mut models = Vec::new();
        if provider.models.is_empty() {
            rows.push(vec![
                id.clone(),
                "-".into(),
                provider.kind.as_str().into(),
                if enabled { "yes" } else { "no" }.into(),
                "-".into(),
            ]);
        }
        for model in provider.models.values() {
            let default = settings
                .default_model
                .as_ref()
                .is_some_and(|sel| sel.provider == *id && sel.model == model.id);
            let title = settings
                .title_summary
                .as_ref()
                .is_some_and(|sel| sel.provider == *id && sel.model == model.id);
            let marks = match (default, title) {
                (true, true) => "default,title",
                (true, false) => "default",
                (false, true) => "title",
                _ => "-",
            };
            rows.push(vec![
                id.clone(),
                model.id.clone(),
                transport_label(provider, model).into(),
                if enabled { "yes" } else { "no" }.into(),
                marks.into(),
            ]);
            models.push(json!({"id": model.id, "name": model.name, "default": default, "titleSummary": title}));
        }
        providers.push(json!({"id": id, "type": provider.kind.as_str(), "enabled": enabled, "baseUrl": provider.options.base_url, "apiKey": mask_opt(provider.options.api_key.as_deref()), "models": models}));
    }
    out.emit(&json!({"defaultModel": settings.default_model, "titleSummary": settings.title_summary, "providers": providers}), |out, w| {
        if rows.is_empty() {
            return writeln!(w, "No providers configured.");
        }
        out.print_table(w, &["PROVIDER", "MODEL", "TRANSPORT", "ENABLED", "MARKS"], &rows)
    })
}
fn run_show(out: &Output, target: Option<&str>) -> Result<(), String> {
    let settings = load_settings()?;
    let Some(spec) = target else {
        let value = json!({"defaultModel": settings.default_model, "titleSummary": settings.title_summary, "titleLanguage": settings.effective_title_language().as_str(), "streamIdleTimeoutMs": settings.idle_timeout_ms(), "exaApiKey": mask_opt(Some(settings.web_search.exa_api_key.as_str())), "tavilyApiKey": mask_opt(Some(settings.web_search.tavily_api_key.as_str()))});
        return out.emit(&value, |_, w| {
            for (label, value) in [
                (
                    "Default model:",
                    format_selection(settings.default_model.as_ref()),
                ),
                (
                    "Title model:",
                    format_selection(settings.title_summary.as_ref()),
                ),
                (
                    "Title language:",
                    settings.effective_title_language().as_str().into(),
                ),
                ("Stream idle (ms):", settings.idle_timeout_ms().to_string()),
                (
                    "Exa API key:",
                    mask_opt(Some(settings.web_search.exa_api_key.as_str())),
                ),
                (
                    "Tavily API key:",
                    mask_opt(Some(settings.web_search.tavily_api_key.as_str())),
                ),
            ] {
                writeln!(w, "{label:<20}{value}")?;
            }
            Ok(())
        });
    };
    let (provider_id, model_id) = match spec.split_once('/') {
        Some((provider, model)) => (provider.trim(), Some(model.trim())),
        None => (spec.trim(), None),
    };
    let provider = settings
        .providers
        .get(provider_id)
        .ok_or_else(|| format!("provider '{provider_id}' is not configured"))?;
    let value = match model_id {
        None => {
            json!({"provider": provider_id, "name": provider.name, "type": provider.kind.as_str(), "enabled": is_enabled(&settings, provider_id), "baseUrl": provider.options.base_url, "apiKey": mask_opt(provider.options.api_key.as_deref()), "models": provider.models.keys().collect::<Vec<_>>()})
        }
        Some(model_id) => {
            let model = provider.models.get(model_id).ok_or_else(|| {
                format!("model '{model_id}' is not defined on provider '{provider_id}'")
            })?;
            json!({"provider": provider_id, "id": model.id, "name": model.name, "transport": transport_label(provider, model), "contextLimit": model.limit.map(|limit| limit.context), "outputLimit": model.limit.map(|limit| limit.output), "variants": model.variants.keys().collect::<Vec<_>>(), "variant": model.variant, "imageInput": model.supports_image_input()})
        }
    };
    out.emit(&value, |_, w| {
        for (label, value) in [
            ("Provider:", provider_id.to_string()),
            ("Name:", provider.name.clone()),
            ("Type:", provider.kind.as_str().to_string()),
            (
                "Enabled:",
                if is_enabled(&settings, provider_id) {
                    "yes"
                } else {
                    "no"
                }
                .to_string(),
            ),
            ("Base URL:", provider.options.base_url.clone()),
            ("API key:", mask_opt(provider.options.api_key.as_deref())),
            (
                "Models:",
                match model_id.and_then(|id| provider.models.get(id)) {
                    Some(model) => format!(
                        "{} ({}; {})",
                        model.id,
                        transport_label(provider, model),
                        model
                            .limit
                            .map(|limit| format!("{}/{}", limit.context, limit.output))
                            .unwrap_or_else(|| "-".to_string())
                    ),
                    None => provider
                        .models
                        .keys()
                        .cloned()
                        .collect::<Vec<_>>()
                        .join(", "),
                },
            ),
        ] {
            writeln!(w, "{label:<12}{value}")?;
        }
        Ok(())
    })
}
fn parse_selection(spec: &str, settings: &AiSettings) -> Result<ModelSelection, String> {
    let (path, variant) = match spec.split_once('#') {
        Some((path, variant)) => (path, Some(variant.trim().to_string())),
        None => (spec, None),
    };
    let (provider, model) = path.split_once('/').ok_or_else(|| {
        format!("invalid model '{spec}'; expected '<PROVIDER>/<MODEL>[#<VARIANT>]'")
    })?;
    let (provider, model) = (provider.trim(), model.trim());
    let config = settings
        .providers
        .get(provider)
        .ok_or_else(|| format!("provider '{provider}' is not configured"))?;
    if !is_enabled(settings, provider) {
        return Err(format!("provider '{provider}' is not enabled"));
    }
    let config_model = config
        .models
        .get(model)
        .ok_or_else(|| format!("model '{model}' is not defined on provider '{provider}'"))?;
    if let Some(variant) = &variant
        && !config_model.variants.contains_key(variant)
    {
        return Err(format!(
            "variant '{variant}' is not defined on model '{provider}/{model}'"
        ));
    }
    Ok(ModelSelection {
        provider: provider.into(),
        model: model.into(),
        variant,
    })
}
fn run_default(
    out: &Output,
    spec: Option<&str>,
    clear: bool,
    title_summary: bool,
) -> Result<(), String> {
    let mut settings = load_settings()?;
    let (label, slot) = if title_summary {
        ("Title model", "titleSummary")
    } else {
        ("Default model", "defaultModel")
    };
    if clear {
        if title_summary {
            settings.title_summary = None;
        } else {
            settings.default_model = None;
        }
        save_settings(&settings)?;
        return out.emit(&json!({"slot": slot, "selection": Value::Null}), |_, w| {
            writeln!(w, "{label} cleared.")
        });
    }
    let Some(spec) = spec else {
        let current = if title_summary {
            &settings.title_summary
        } else {
            &settings.default_model
        };
        return out.emit(&json!({"slot": slot, "selection": current}), |_, w| {
            writeln!(w, "{label}: {}", format_selection(current.as_ref()))
        });
    };
    let selection = parse_selection(spec, &settings)?;
    if title_summary {
        settings.title_summary = Some(selection.clone());
    } else {
        settings.default_model = Some(selection.clone());
    }
    save_settings(&settings)?;
    out.emit(&json!({"slot": slot, "selection": selection}), |_, w| {
        writeln!(w, "{label} set to {}.", format_selection(Some(&selection)))
    })
}
fn run_provider_set(out: &Output, args: &ProviderSetArgs) -> Result<(), String> {
    if args.provider.trim().is_empty() {
        return Err("provider ID cannot be empty".into());
    }
    let mut settings = load_settings()?;
    let existing = settings.providers.get(&args.provider).cloned();
    let was_enabled = is_enabled(&settings, &args.provider);
    let kind = args
        .kind
        .or_else(|| existing.as_ref().map(|p| p.kind))
        .unwrap_or(ProviderTransport::OpenaiCompatible);
    let name = args
        .name
        .clone()
        .or_else(|| existing.as_ref().map(|p| p.name.clone()))
        .unwrap_or_else(|| args.provider.clone());
    let base_url = args
        .base_url
        .clone()
        .or_else(|| existing.as_ref().map(|p| p.options.base_url.clone()))
        .unwrap_or_default();
    if base_url.trim().is_empty() {
        return Err(format!(
            "provider '{}' requires '--base-url <URL>'",
            args.provider
        ));
    }
    let api_key = if args.clear_api_key {
        None
    } else {
        args.api_key
            .clone()
            .or_else(|| existing.as_ref().and_then(|p| p.options.api_key.clone()))
    };
    settings.providers.insert(
        args.provider.clone(),
        ProviderConfig {
            id: args.provider.clone(),
            kind,
            name,
            options: ProviderOptions {
                api_key,
                base_url: base_url.trim().to_string(),
            },
            models: existing
                .as_ref()
                .map(|p| p.models.clone())
                .unwrap_or_default(),
            extra: existing
                .as_ref()
                .map(|p| p.extra.clone())
                .unwrap_or_default(),
        },
    );
    let enabled = if args.enable {
        true
    } else if args.disable {
        false
    } else if existing.is_some() {
        was_enabled
    } else {
        true
    };
    if enabled && !is_enabled(&settings, &args.provider) {
        settings.enabled_providers.push(args.provider.clone());
    }
    if !enabled {
        settings.enabled_providers.retain(|id| id != &args.provider);
        settings.default_model = settings
            .default_model
            .filter(|sel| sel.provider != args.provider);
        settings.title_summary = settings
            .title_summary
            .filter(|sel| sel.provider != args.provider);
    }
    settings.sanitize_optional_selections();
    save_settings(&settings)?;
    out.emit(
        &json!({"provider": args.provider, "enabled": enabled}),
        |_, w| {
            writeln!(
                w,
                "Provider '{}' saved ({}).",
                args.provider,
                if enabled { "enabled" } else { "disabled" }
            )
        },
    )
}
fn run_provider_rm(out: &Output, provider: &str) -> Result<(), String> {
    if provider.trim().is_empty() {
        return Err("provider ID cannot be empty".into());
    }
    let mut settings = load_settings()?;
    if settings.providers.remove(provider).is_none() {
        return Err(format!("provider '{provider}' is not configured"));
    }
    settings.enabled_providers.retain(|id| id != provider);
    settings.default_model = settings
        .default_model
        .filter(|sel| sel.provider != provider);
    settings.title_summary = settings
        .title_summary
        .filter(|sel| sel.provider != provider);
    save_settings(&settings)?;
    out.emit(&json!({"removed": provider}), |_, w| {
        writeln!(w, "Provider '{provider}' removed.")
    })
}
fn run_model_set(out: &Output, args: &ModelSetArgs) -> Result<(), String> {
    let (provider_id, model_id) = args
        .target
        .split_once('/')
        .ok_or_else(|| format!("invalid model target '{}'", args.target))?;
    let (provider_id, model_id) = (provider_id.trim(), model_id.trim());
    if provider_id.is_empty() {
        return Err("provider ID cannot be empty".into());
    }
    if model_id.is_empty() {
        return Err("model ID cannot be empty".into());
    }
    let mut settings = load_settings()?;
    let provider = settings
        .providers
        .get_mut(provider_id)
        .ok_or_else(|| format!("provider '{provider_id}' is not configured"))?;
    let mut model = provider
        .models
        .get(model_id)
        .cloned()
        .unwrap_or(ModelConfig {
            id: model_id.into(),
            name: model_id.into(),
            transport: None,
            limit: None,
            modalities: None,
            variants: Default::default(),
            variant: None,
            pricing: None,
            extra: serde_json::Map::new(),
        });
    if let Some(name) = &args.name {
        model.name = name.clone();
    }
    if let Some(transport) = args.transport {
        model.transport = transport;
    }
    if args.context_limit.is_some() || args.output_limit.is_some() {
        let mut limit = model.limit.unwrap_or(ModelLimit {
            context: 0,
            output: 0,
        });
        if let Some(value) = args.context_limit {
            limit.context = value;
        }
        if let Some(value) = args.output_limit {
            limit.output = value;
        }
        if limit.context == 0 || limit.output == 0 {
            return Err("both '--context-limit' and '--output-limit' are required when the model has no limits".into());
        }
        model.limit = Some(limit);
    }
    if let Some(variant) = &args.variant {
        model
            .variants
            .entry(variant.clone())
            .or_insert_with(|| Value::Object(serde_json::Map::new()));
        model.variant = Some(variant.clone());
    }
    if let Some(enabled) = args.image_input {
        let mut inputs: Vec<Value> = model
            .modalities
            .as_ref()
            .and_then(|value| value.get("input"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_else(|| vec![json!("text")]);
        inputs.retain(|value| value.as_str() != Some("image"));
        if enabled {
            inputs.push(json!("image"));
        }
        let outputs = model
            .modalities
            .as_ref()
            .and_then(|value| value.get("output"))
            .cloned()
            .unwrap_or_else(|| json!(["text"]));
        model.modalities = Some(json!({"input": inputs, "output": outputs}));
    }
    provider.models.insert(model_id.into(), model);
    save_settings(&settings)?;
    out.emit(
        &json!({"provider": provider_id, "model": model_id}),
        |_, w| writeln!(w, "Model '{provider_id}/{model_id}' saved."),
    )
}
fn run_model_rm(out: &Output, target: &str) -> Result<(), String> {
    let (provider_id, model_id) = target
        .split_once('/')
        .ok_or_else(|| format!("invalid model target '{target}'"))?;
    let (provider_id, model_id) = (provider_id.trim(), model_id.trim());
    if provider_id.is_empty() {
        return Err("provider ID cannot be empty".into());
    }
    if model_id.is_empty() {
        return Err("model ID cannot be empty".into());
    }
    let mut settings = load_settings()?;
    let provider = settings
        .providers
        .get_mut(provider_id)
        .ok_or_else(|| format!("provider '{provider_id}' is not configured"))?;
    if provider.models.remove(model_id).is_none() {
        return Err(format!(
            "model '{model_id}' is not defined on provider '{provider_id}'"
        ));
    }
    settings.default_model = settings
        .default_model
        .filter(|sel| !(sel.provider == provider_id && sel.model == model_id));
    settings.title_summary = settings
        .title_summary
        .filter(|sel| !(sel.provider == provider_id && sel.model == model_id));
    save_settings(&settings)?;
    out.emit(
        &json!({"provider": provider_id, "removed": model_id}),
        |_, w| writeln!(w, "Model '{provider_id}/{model_id}' removed."),
    )
}
