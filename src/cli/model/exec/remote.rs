//! `model fetch` / `model config`：远端模型发现与全局模型设置。

use super::*;

pub(super) fn run_fetch(out: &Output, args: &FetchArgs) -> Result<(), String> {
    let mut settings = load_settings()?;
    let provider = settings
        .providers
        .get(&args.provider)
        .cloned()
        .ok_or_else(|| format!("provider '{}' is not configured", args.provider))?;
    if !matches!(
        provider.kind,
        ProviderTransport::OpenaiCompatible | ProviderTransport::Openai
    ) {
        return Err(format!(
            "provider type '{}' does not support remote model fetching",
            provider.kind.as_str()
        ));
    }
    let base_url = args
        .base_url
        .clone()
        .unwrap_or_else(|| provider.options.base_url.clone());
    let base_url = base_url.trim();
    if base_url.is_empty() {
        return Err(format!(
            "provider '{}' has no base URL; pass '--base-url <URL>'",
            args.provider
        ));
    }
    if !(base_url.starts_with("http://") || base_url.starts_with("https://")) {
        return Err(format!(
            "invalid base URL '{base_url}'; expected an http:// or https:// URL"
        ));
    }
    let api_key = args
        .api_key
        .clone()
        .or_else(|| provider.options.api_key.clone())
        .unwrap_or_default();
    if api_key.trim().is_empty() {
        return Err(format!(
            "provider '{}' requires an API key; pass '--api-key <KEY>'",
            args.provider
        ));
    }
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| format!("failed to initialize async runtime: {error}"))?;
    let models = runtime.block_on(crate::settings::service::fetch_remote_models(
        base_url,
        api_key.trim(),
    ))?;
    let mut added = 0;
    if args.save {
        let target = settings
            .providers
            .get_mut(&args.provider)
            .ok_or_else(|| format!("provider '{}' is not configured", args.provider))?;
        for (id, _) in &models {
            if target.models.contains_key(id) {
                continue;
            }
            target.models.insert(
                id.clone(),
                ModelConfig {
                    id: id.clone(),
                    name: id.clone(),
                    transport: None,
                    limit: None,
                    modalities: None,
                    variants: Default::default(),
                    variant: None,
                    pricing: None,
                    extra: serde_json::Map::new(),
                },
            );
            added += 1;
        }
        save_settings(&settings)?;
    }
    let value = json!({"provider": args.provider, "models": models.iter().map(|(id, owner)| json!({"id": id, "ownedBy": owner})).collect::<Vec<_>>(), "saved": args.save, "added": added});
    out.emit(&value, |out, w| {
        if models.is_empty() {
            return writeln!(w, "The provider returned no models.");
        }
        let rows: Vec<Vec<String>> = models
            .iter()
            .map(|(id, owner)| vec![id.clone(), owner.clone()])
            .collect();
        out.print_table(w, &["MODEL", "OWNED BY"], &rows)?;
        if args.save {
            writeln!(w, "\nSaved {added} new model(s) under '{}'.", args.provider)?;
        }
        Ok(())
    })
}
pub(super) fn run_config(out: &Output, args: &ConfigArgs) -> Result<(), String> {
    let mut settings = load_settings()?;
    if let Some(language) = args.title_language {
        settings.title_language = Some(language);
    }
    if let Some(timeout) = args.timeout_ms {
        settings.stream_idle_timeout_ms = (timeout > 0).then_some(timeout);
    }
    if let Some(key) = &args.exa_key {
        settings.web_search.exa_api_key = key.trim().into();
    }
    if let Some(key) = &args.tavily_key {
        settings.web_search.tavily_api_key = key.trim().into();
    }
    save_settings(&settings)?;
    let value = json!({"titleLanguage": settings.effective_title_language().as_str(), "streamIdleTimeoutMs": settings.idle_timeout_ms(), "exaApiKey": mask_opt(Some(settings.web_search.exa_api_key.as_str())), "tavilyApiKey": mask_opt(Some(settings.web_search.tavily_api_key.as_str()))});
    out.emit(&value, |_, w| {
        for (label, value) in [
            (
                "Title language:",
                settings.effective_title_language().as_str().to_string(),
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
            writeln!(w, "{label:<18}{value}")?;
        }
        Ok(())
    })
}
