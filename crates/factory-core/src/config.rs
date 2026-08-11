use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    OpenAi,
    Local,
}

#[derive(Debug, Clone)]
pub struct ProviderConfig {
    pub kind: ProviderKind,
    pub base_url: String,
    pub api_key: String,
    pub model: String,
}

impl Default for ProviderConfig {
    fn default() -> Self {
        ProviderConfig {
            kind: ProviderKind::OpenAi,
            base_url: "https://api.openai.com/v1".to_string(),
            api_key: String::new(),
            model: "gpt-4o-mini".to_string(),
        }
    }
}

pub fn config_from_env() -> ProviderConfig {
    let mut cfg = ProviderConfig::default();
    if let Ok(kind) = std::env::var("FACTORY_PROVIDER") {
        cfg.kind = match kind.as_str() {
            "local" => ProviderKind::Local,
            _ => ProviderKind::OpenAi,
        };
    }
    if let Ok(url) = std::env::var("FACTORY_BASE_URL") {
        cfg.base_url = url;
    }
    if let Ok(key) = std::env::var("FACTORY_API_KEY") {
        cfg.api_key = key;
    }
    if let Ok(model) = std::env::var("FACTORY_MODEL") {
        cfg.model = model;
    }
    cfg
}
