use crate::config::{digest, valid_id};
use crate::store::Store;
use crate::{IdeError, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::Arc;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExtensionManifest {
    pub id: String,
    pub name: String,
    pub version: String,
    pub publisher: String,
    pub description: String,
    pub entrypoint: String,
    pub extension_api_version: u32,
    pub capabilities: Vec<String>,
    pub permissions: Vec<String>,
    pub supported_platforms: Vec<String>,
    #[serde(default)]
    pub dependencies: BTreeMap<String, String>,
    pub contributions: Contributions,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Contributions {
    #[serde(default)]
    pub formatters: Vec<Formatter>,
    #[serde(default)]
    pub diagnostics: Vec<DiagnosticRule>,
    #[serde(default)]
    pub commands: Vec<ExtensionCommand>,
    #[serde(default)]
    pub languages: Vec<Language>,
    #[serde(default)]
    pub views: Vec<ExtensionView>,
    #[serde(default)]
    pub settings: BTreeMap<String, String>,
    #[serde(default)]
    pub lsp_profiles: Vec<String>,
    #[serde(default)]
    pub dap_profiles: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Formatter {
    pub id: String,
    pub title: String,
    pub operation: String,
    pub languages: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiagnosticRule {
    pub id: String,
    pub needle: String,
    pub message: String,
    pub severity: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExtensionCommand {
    pub id: String,
    pub title: String,
    pub action: String,
    pub target: String,
    #[serde(default)]
    pub keybinding: Option<String>,
    #[serde(default)]
    pub context_menu: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Language {
    pub extension: String,
    pub syntax: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExtensionView {
    pub id: String,
    pub title: String,
    pub text: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InstalledExtension {
    pub manifest: ExtensionManifest,
    pub sha256: String,
    pub provenance: String,
    pub installed_at: String,
    pub enabled: bool,
    pub status: String,
    pub error: Option<String>,
    #[serde(default)]
    pub preferences: BTreeMap<String, String>,
}

pub struct ExtensionManager {
    project: String,
    store: Arc<Store>,
}

impl ExtensionManifest {
    pub fn validate(&self) -> Result<()> {
        if !valid_id(&self.id)
            || !self.id.contains('.')
            || !valid_id(&self.publisher)
            || self.name.is_empty()
            || self.name.len() > 100
            || self.description.len() > 2000
            || self.entrypoint != "contributions"
            || self.extension_api_version != 1
            || !valid_version(&self.version)
            || self.capabilities.len() > 8
            || self.permissions.len() > 8
            || self.supported_platforms.is_empty()
            || self.supported_platforms.len() > 4
            || self.dependencies.len() > 8
            || serde_json::to_vec(self)
                .map_err(|_| IdeError::Invalid)?
                .len()
                > 131072
        {
            return Err(IdeError::Invalid);
        }
        let allowed = ["editor", "commands", "views", "settings", "profiles"];
        if self
            .capabilities
            .iter()
            .chain(self.permissions.iter())
            .any(|s| !allowed.contains(&s.as_str()))
            || self
                .capabilities
                .iter()
                .any(|s| !self.permissions.contains(s))
        {
            return Err(IdeError::Forbidden);
        }
        if self
            .supported_platforms
            .iter()
            .any(|s| !["any", "windows", "linux", "macos"].contains(&s.as_str()))
            || !self
                .supported_platforms
                .iter()
                .any(|s| s == "any" || s == std::env::consts::OS)
        {
            return Err(IdeError::Unsupported);
        }
        if self
            .dependencies
            .iter()
            .any(|(id, version)| !valid_id(id) || !valid_version(version) || id == &self.id)
        {
            return Err(IdeError::Invalid);
        }
        let c = &self.contributions;
        if c.formatters.len()
            + c.diagnostics.len()
            + c.commands.len()
            + c.languages.len()
            + c.views.len()
            + c.settings.len()
            + c.lsp_profiles.len()
            + c.dap_profiles.len()
            > 64
        {
            return Err(IdeError::Limit);
        }
        if (!c.formatters.is_empty() || !c.diagnostics.is_empty() || !c.languages.is_empty())
            && !self.permissions.contains(&"editor".into())
            || !c.commands.is_empty() && !self.permissions.contains(&"commands".into())
            || !c.views.is_empty() && !self.permissions.contains(&"views".into())
            || !c.settings.is_empty() && !self.permissions.contains(&"settings".into())
            || (!c.lsp_profiles.is_empty() || !c.dap_profiles.is_empty())
                && !self.permissions.contains(&"profiles".into())
        {
            return Err(IdeError::Forbidden);
        }
        for formatter in &c.formatters {
            if !valid_id(&formatter.id)
                || formatter.title.len() > 100
                || formatter.languages.len() > 32
                || !["trim-trailing-whitespace", "final-newline", "sort-lines"]
                    .contains(&formatter.operation.as_str())
            {
                return Err(IdeError::Invalid);
            }
        }
        for diagnostic in &c.diagnostics {
            if !valid_id(&diagnostic.id)
                || diagnostic.needle.is_empty()
                || diagnostic.needle.len() > 128
                || diagnostic.message.len() > 1000
                || !["error", "warning", "info"].contains(&diagnostic.severity.as_str())
            {
                return Err(IdeError::Invalid);
            }
        }
        for language in &c.languages {
            if !valid_id(&language.extension)
                || ![
                    "rust",
                    "typescript",
                    "javascript",
                    "python",
                    "json",
                    "html",
                    "css",
                    "markdown",
                    "toml",
                    "yaml",
                    "text",
                ]
                .contains(&language.syntax.as_str())
            {
                return Err(IdeError::Invalid);
            }
        }
        for view in &c.views {
            if !valid_id(&view.id) || view.title.len() > 100 || view.text.len() > 16000 {
                return Err(IdeError::Invalid);
            }
        }
        for command in &c.commands {
            if !valid_id(&command.id)
                || command.title.len() > 100
                || command
                    .keybinding
                    .as_ref()
                    .is_some_and(|s| !valid_binding(s))
                || match command.action.as_str() {
                    "open-view" => !c.views.iter().any(|view| view.id == command.target),
                    "format" => !c
                        .formatters
                        .iter()
                        .any(|formatter| formatter.id == command.target),
                    _ => true,
                }
            {
                return Err(IdeError::Invalid);
            }
        }
        if c.settings
            .iter()
            .any(|(key, value)| !valid_id(key) || value.len() > 1000)
            || c.lsp_profiles
                .iter()
                .chain(c.dap_profiles.iter())
                .any(|id| !valid_id(id))
        {
            return Err(IdeError::Invalid);
        }
        Ok(())
    }
}

fn valid_version(value: &str) -> bool {
    value.split('.').count() == 3
        && value.split('.').all(|part| {
            !part.is_empty() && part.len() < 8 && part.bytes().all(|b| b.is_ascii_digit())
        })
}

fn valid_binding(value: &str) -> bool {
    if value.len() > 60 {
        return false;
    }
    let parts = value.split('-').collect::<Vec<_>>();
    let Some(key) = parts.last() else {
        return false;
    };
    if parts[..parts.len() - 1]
        .iter()
        .any(|part| !["Alt", "Shift", "Ctrl", "Control", "Meta", "Mod"].contains(part))
    {
        return false;
    }
    if !((key.len() == 1 && key.as_bytes()[0].is_ascii_alphanumeric())
        || [
            "Enter",
            "Tab",
            "Escape",
            "Backspace",
            "Delete",
            "ArrowUp",
            "ArrowDown",
            "ArrowLeft",
            "ArrowRight",
            "Home",
            "End",
            "PageUp",
            "PageDown",
            "Space",
        ]
        .contains(key)
        || key
            .strip_prefix('F')
            .and_then(|s| s.parse::<u8>().ok())
            .is_some_and(|n| (1..=12).contains(&n)))
    {
        return false;
    }
    !(parts
        .iter()
        .any(|part| ["Mod", "Ctrl", "Control", "Meta"].contains(part))
        && ["r", "w", "t", "n", "l"].contains(&key.to_ascii_lowercase().as_str()))
}

impl ExtensionManager {
    pub fn new(project: &str, store: Arc<Store>) -> Self {
        Self {
            project: project.into(),
            store,
        }
    }
    pub fn list(&self) -> Result<Vec<InstalledExtension>> {
        let mut installed = self
            .store
            .list::<InstalledExtension>(&self.project, "extensions")?;
        for extension in &mut installed {
            extension.preferences = extension.manifest.contributions.settings.clone();
            if let Ok(preferences) = self.store.get::<BTreeMap<String, String>>(
                &self.project,
                "extension-settings",
                &extension.manifest.id,
            ) {
                extension
                    .preferences
                    .extend(preferences.into_iter().filter(|(key, _)| {
                        extension.manifest.contributions.settings.contains_key(key)
                    }));
            }
            if let Err(error) = extension
                .manifest
                .validate()
                .and_then(|_| verify_integrity(extension))
                .and_then(|_| self.dependencies(&extension.manifest))
            {
                extension.status = "failed".into();
                extension.error = Some(error.to_string());
            }
        }
        Ok(installed)
    }
    fn dependencies(&self, manifest: &ExtensionManifest) -> Result<()> {
        for (id, version) in &manifest.dependencies {
            let dependency =
                self.store
                    .get::<InstalledExtension>(&self.project, "extensions", id)?;
            if !dependency.enabled || dependency.manifest.version != *version {
                return Err(IdeError::Conflict);
            }
            verify_integrity(&dependency)?;
        }
        Ok(())
    }
    pub fn install(
        &self,
        manifest: ExtensionManifest,
        trusted: bool,
    ) -> Result<InstalledExtension> {
        if !trusted {
            return Err(IdeError::Unapproved);
        }
        manifest.validate()?;
        self.store
            .audit(&self.project, "request_extension_install", &manifest.id)?;
        if self
            .store
            .get::<InstalledExtension>(&self.project, "extensions", &manifest.id)
            .is_ok()
        {
            return Err(IdeError::Conflict);
        }
        self.dependencies(&manifest)?;
        let extension = InstalledExtension {
            sha256: digest(serde_json::to_vec(&manifest).map_err(|_| IdeError::Invalid)?),
            manifest,
            provenance: "explicit local declarative package import".into(),
            installed_at: chrono::Utc::now().to_rfc3339(),
            enabled: false,
            status: "installed".into(),
            error: None,
            preferences: BTreeMap::new(),
        };
        self.store.insert(
            &self.project,
            "extensions",
            &extension.manifest.id,
            &extension,
        )?;
        Ok(extension)
    }
    pub fn enable(&self, id: &str, enabled: bool) -> Result<InstalledExtension> {
        self.store.audit(
            &self.project,
            if enabled {
                "request_extension_enable"
            } else {
                "request_extension_disable"
            },
            id,
        )?;
        let mut extension: InstalledExtension = self.store.get(&self.project, "extensions", id)?;
        extension.manifest.validate()?;
        verify_integrity(&extension)?;
        if enabled {
            self.dependencies(&extension.manifest)?;
        }
        extension.enabled = enabled;
        extension.status = if enabled { "active" } else { "disabled" }.into();
        extension.error = None;
        self.store
            .put(&self.project, "extensions", id, &extension)?;
        Ok(extension)
    }
    pub fn uninstall(&self, id: &str) -> Result<()> {
        self.store
            .audit(&self.project, "request_extension_uninstall", id)?;
        if self
            .list()?
            .iter()
            .any(|extension| extension.enabled && extension.manifest.dependencies.contains_key(id))
        {
            return Err(IdeError::Conflict);
        }
        self.store.remove(&self.project, "extension-settings", id)?;
        self.store.remove(&self.project, "extensions", id)
    }
    pub fn settings(&self, id: &str, settings: BTreeMap<String, String>) -> Result<()> {
        let extension: InstalledExtension = self.store.get(&self.project, "extensions", id)?;
        if settings.iter().any(|(key, value)| {
            !extension.manifest.contributions.settings.contains_key(key) || value.len() > 1000
        }) {
            return Err(IdeError::Forbidden);
        }
        self.store
            .put(&self.project, "extension-settings", id, &settings)
    }
}

fn verify_integrity(extension: &InstalledExtension) -> Result<()> {
    if digest(serde_json::to_vec(&extension.manifest).map_err(|_| IdeError::Invalid)?)
        != extension.sha256
    {
        return Err(IdeError::Storage);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn keybindings_are_valid_and_preserve_browser_navigation() {
        for value in ["Alt-Shift-w", "Mod-Shift-k", "F8", "Ctrl-Space"] {
            assert!(valid_binding(value));
        }
        for value in [
            "Ctrl-r",
            "Mod-w",
            "Bad-Key",
            "Alt-Shift-",
            "",
            "Alt-ScriptCode",
            "Ctrl-n",
        ] {
            assert!(!valid_binding(value));
        }
    }
}
