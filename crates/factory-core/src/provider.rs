use factory_models::ModelUsage;
use thiserror::Error;

use crate::config::{ProviderConfig, ProviderKind};

#[derive(Debug, Error)]
pub enum ProviderError {
    #[error("no FACTORY_API_KEY set; set one or use FACTORY_PROVIDER=local")]
    MissingApiKey,
    #[error("provider request failed: {0}")]
    Request(String),
    #[error("provider returned an invalid response: {0}")]
    InvalidResponse(String),
}

#[derive(Debug, Clone)]
pub struct ChatResponse {
    pub content: String,
    pub model: String,
    pub usage: ModelUsage,
}

pub trait Provider: Send + Sync {
    fn model(&self) -> &str;
    fn generate(&self, system: &str, user: &str) -> Result<ChatResponse, ProviderError>;
}

pub struct OpenAICompatibleProvider {
    base_url: String,
    api_key: String,
    model: String,
}

impl OpenAICompatibleProvider {
    pub fn new(cfg: &ProviderConfig) -> Result<Self, ProviderError> {
        if cfg.api_key.is_empty() {
            return Err(ProviderError::MissingApiKey);
        }
        Ok(OpenAICompatibleProvider {
            base_url: cfg.base_url.trim_end_matches('/').to_string(),
            api_key: cfg.api_key.clone(),
            model: cfg.model.clone(),
        })
    }
}

impl Provider for OpenAICompatibleProvider {
    fn model(&self) -> &str {
        &self.model
    }

    fn generate(&self, system: &str, user: &str) -> Result<ChatResponse, ProviderError> {
        let body = serde_json::json!({
            "model": self.model,
            "messages": [
                {"role": "system", "content": system},
                {"role": "user", "content": user}
            ],
            "temperature": 0.2,
            "response_format": {"type": "json_object"}
        });
        let url = format!("{}/chat/completions", self.base_url);
        let resp = ureq::post(&url)
            .set("Authorization", &format!("Bearer {}", self.api_key))
            .set("Content-Type", "application/json")
            .send_json(body)
            .map_err(|e| ProviderError::Request(e.to_string()))?;
        let text = resp
            .into_string()
            .map_err(|e| ProviderError::Request(e.to_string()))?;
        let parsed: serde_json::Value = serde_json::from_str(&text)
            .map_err(|e| ProviderError::InvalidResponse(e.to_string()))?;
        let content = parsed
            .pointer("/choices/0/message/content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                ProviderError::InvalidResponse("missing choices[0].message.content".into())
            })?
            .to_string();
        let usage = usage_from_json(parsed.get("usage")).unwrap_or_default();
        let model = parsed
            .get("model")
            .and_then(|v| v.as_str())
            .unwrap_or(&self.model)
            .to_string();
        Ok(ChatResponse {
            content,
            model,
            usage,
        })
    }
}

pub struct LocalProvider {
    model: String,
}

impl LocalProvider {
    pub fn new() -> Self {
        LocalProvider {
            model: "local-planner".to_string(),
        }
    }
}

impl Default for LocalProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl Provider for LocalProvider {
    fn model(&self) -> &str {
        &self.model
    }

    fn generate(&self, _system: &str, user: &str) -> Result<ChatResponse, ProviderError> {
        let objective = user.trim();
        let plan = factory_models::Plan {
            objective: objective.to_string(),
            tasks: vec![
                factory_models::PlannedTask {
                    id: "T1".into(),
                    title: "Clarify the objective".into(),
                    objective: format!(
                        "Restate \"{objective}\" as concrete requirements and list open questions"
                    ),
                    dependencies: vec![],
                    acceptance_criteria: vec![
                        "Requirements are explicit".into(),
                        "Open questions are listed".into(),
                    ],
                },
                factory_models::PlannedTask {
                    id: "T2".into(),
                    title: "Set up the scaffold".into(),
                    objective:
                        "Create the repository, workspace and project skeleton for the objective"
                            .into(),
                    dependencies: vec!["T1".into()],
                    acceptance_criteria: vec![
                        "Project builds cleanly".into(),
                        "Directory layout matches the plan".into(),
                    ],
                },
                factory_models::PlannedTask {
                    id: "T3".into(),
                    title: "Implement the core functionality".into(),
                    objective: format!("Implement the core behaviour described in \"{objective}\""),
                    dependencies: vec!["T2".into()],
                    acceptance_criteria: vec![
                        "Core behaviour is implemented".into(),
                        "Happy paths are tested".into(),
                    ],
                },
                factory_models::PlannedTask {
                    id: "T4".into(),
                    title: "Write tests".into(),
                    objective: "Add tests covering the implemented behaviour and edge cases".into(),
                    dependencies: vec!["T3".into()],
                    acceptance_criteria: vec![
                        "Tests run and pass".into(),
                        "Edge cases are covered".into(),
                    ],
                },
                factory_models::PlannedTask {
                    id: "T5".into(),
                    title: "Document and finalize".into(),
                    objective: "Write documentation and prepare the final review of the objective"
                        .into(),
                    dependencies: vec!["T4".into()],
                    acceptance_criteria: vec![
                        "Documentation is written".into(),
                        "Final review notes recorded".into(),
                    ],
                },
            ],
        };
        let content = serde_json::to_string(&plan)
            .map_err(|e| ProviderError::InvalidResponse(e.to_string()))?;
        Ok(ChatResponse {
            content,
            model: self.model.clone(),
            usage: ModelUsage::none(),
        })
    }
}

pub fn build_provider(cfg: &ProviderConfig) -> Result<Box<dyn Provider>, ProviderError> {
    match cfg.kind {
        ProviderKind::OpenAi => Ok(Box::new(OpenAICompatibleProvider::new(cfg)?)),
        ProviderKind::Local => Ok(Box::new(LocalProvider::new())),
    }
}

fn usage_from_json(value: Option<&serde_json::Value>) -> Option<ModelUsage> {
    let value = value?;
    Some(ModelUsage {
        prompt_tokens: value
            .get("prompt_tokens")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32,
        completion_tokens: value
            .get("completion_tokens")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32,
        total_tokens: value
            .get("total_tokens")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32,
    })
}
