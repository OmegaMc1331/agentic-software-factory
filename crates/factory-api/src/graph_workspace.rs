use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

const VERSION: u8 = 1;
const MAX_COORDINATE: f64 = 10_000_000.0;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct GraphPosition {
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceNode {
    pub id: String,
    pub kind: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceEdge {
    pub id: String,
    pub source: String,
    pub target: String,
    pub kind: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphWorkspace {
    pub version: u8,
    #[serde(default)]
    pub nodes: BTreeMap<String, GraphPosition>,
    #[serde(default)]
    pub custom_nodes: Vec<WorkspaceNode>,
    #[serde(default)]
    pub edges: Vec<WorkspaceEdge>,
}

impl Default for GraphWorkspace {
    fn default() -> Self {
        Self {
            version: VERSION,
            nodes: BTreeMap::new(),
            custom_nodes: Vec::new(),
            edges: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphWorkspaceResponse {
    #[serde(flatten)]
    pub workspace: GraphWorkspace,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
}

#[derive(Debug, Error)]
pub enum GraphWorkspaceError {
    #[error("failed to read graph workspace")]
    Read(PathBuf, std::io::Error),
    #[error("failed to write graph workspace")]
    Write(PathBuf, std::io::Error),
    #[error("failed to serialize graph workspace")]
    Serialize(PathBuf, serde_json::Error),
}

impl GraphWorkspace {
    pub fn path(root: &Path) -> PathBuf {
        root.join(".factory").join("graph.json")
    }

    pub fn load(root: &Path) -> Result<GraphWorkspaceResponse, GraphWorkspaceError> {
        let path = Self::path(root);
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(GraphWorkspaceResponse {
                    workspace: Self::default(),
                    warning: None,
                });
            }
            Err(error) => return Err(GraphWorkspaceError::Read(path, error)),
        };
        let workspace: GraphWorkspace = match serde_json::from_str(&text) {
            Ok(workspace) => workspace,
            Err(error) => {
                return Ok(GraphWorkspaceResponse {
                    workspace: Self::default(),
                    warning: Some(format!(
                        "Ignored malformed graph workspace at {}: {error}",
                        path.display()
                    )),
                });
            }
        };
        if let Err(reason) = workspace.validate_shape() {
            return Ok(GraphWorkspaceResponse {
                workspace: Self::default(),
                warning: Some(format!(
                    "Ignored invalid graph workspace at {}: {reason}",
                    path.display()
                )),
            });
        }
        Ok(GraphWorkspaceResponse {
            workspace,
            warning: None,
        })
    }

    pub fn validate(&self, system_nodes: &HashMap<String, String>) -> Result<(), String> {
        self.validate_shape()?;
        let mut kinds = system_nodes.clone();
        for node in &self.custom_nodes {
            kinds.insert(node.id.clone(), node.kind.clone());
        }
        for id in self.nodes.keys() {
            if !kinds.contains_key(id) {
                return Err(format!("graph position refers to unknown node '{id}'"));
            }
        }
        for edge in &self.edges {
            let source = kinds.get(&edge.source).ok_or_else(|| {
                format!("edge '{}' has unknown source '{}'", edge.id, edge.source)
            })?;
            let target = kinds.get(&edge.target).ok_or_else(|| {
                format!("edge '{}' has unknown target '{}'", edge.id, edge.target)
            })?;
            match edge.kind.as_str() {
                "custom" if source == "agent" && target == "agent" => {}
                "membership"
                    if (source == "group" && target != "group")
                        || (target == "group" && source != "group") => {}
                "custom" => {
                    return Err(format!("custom edge '{}' must connect two agents", edge.id));
                }
                "membership" => {
                    return Err(format!(
                        "membership edge '{}' must connect a node and a group",
                        edge.id
                    ));
                }
                _ => return Err(format!("edge '{}' has unsupported kind", edge.id)),
            }
        }
        Ok(())
    }

    pub fn retain_known(&mut self, system_nodes: &HashMap<String, String>) -> bool {
        let custom_nodes: HashSet<&str> = self
            .custom_nodes
            .iter()
            .map(|node| node.id.as_str())
            .collect();
        let is_known = |id: &str| system_nodes.contains_key(id) || custom_nodes.contains(id);
        let node_count = self.nodes.len();
        let edge_count = self.edges.len();
        self.nodes.retain(|id, _| is_known(id));
        self.edges
            .retain(|edge| is_known(&edge.source) && is_known(&edge.target));
        self.nodes.len() != node_count || self.edges.len() != edge_count
    }

    fn validate_shape(&self) -> Result<(), String> {
        if self.version != VERSION {
            return Err(format!(
                "unsupported graph workspace version {}",
                self.version
            ));
        }
        for (id, position) in &self.nodes {
            if id.is_empty() || id.len() > 160 || contains_control(id) {
                return Err("graph position has an invalid node id".to_string());
            }
            if !position.x.is_finite()
                || !position.y.is_finite()
                || position.x.abs() > MAX_COORDINATE
                || position.y.abs() > MAX_COORDINATE
            {
                return Err(format!(
                    "graph position for '{id}' is outside the safe range"
                ));
            }
        }

        let mut node_ids = HashSet::new();
        for node in &self.custom_nodes {
            let prefix = match node.kind.as_str() {
                "group" => "group:",
                "note" => "note:",
                _ => return Err(format!("custom node '{}' has unsupported kind", node.id)),
            };
            if !node.id.starts_with(prefix) || node.id.len() > 160 || contains_control(&node.id) {
                return Err(format!("custom node '{}' has an invalid id", node.id));
            }
            if !node_ids.insert(node.id.as_str()) {
                return Err(format!("duplicate custom node '{}'", node.id));
            }
            if node.label.trim().is_empty()
                || node.label.len() > 120
                || contains_control(&node.label)
            {
                return Err(format!("custom node '{}' has an invalid label", node.id));
            }
            if node.text.as_deref().is_some_and(|text| text.len() > 2_000) {
                return Err(format!("custom node '{}' text is too long", node.id));
            }
        }

        let mut edge_ids = HashSet::new();
        let mut edge_keys = HashSet::new();
        for edge in &self.edges {
            if !edge.id.starts_with("edge:") || edge.id.len() > 160 || contains_control(&edge.id) {
                return Err(format!("edge '{}' has an invalid id", edge.id));
            }
            if !edge_ids.insert(edge.id.as_str()) {
                return Err(format!("duplicate edge id '{}'", edge.id));
            }
            if edge.source == edge.target {
                return Err(format!(
                    "edge '{}' cannot connect a node to itself",
                    edge.id
                ));
            }
            if !edge_keys.insert((&edge.source, &edge.target, &edge.kind)) {
                return Err(format!(
                    "duplicate {} edge from '{}' to '{}'",
                    edge.kind, edge.source, edge.target
                ));
            }
            if !matches!(edge.kind.as_str(), "custom" | "membership") {
                return Err(format!("edge '{}' has unsupported kind", edge.id));
            }
        }
        Ok(())
    }

    pub fn write_atomic(&self, root: &Path) -> Result<PathBuf, GraphWorkspaceError> {
        let path = Self::path(root);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| GraphWorkspaceError::Write(path.clone(), error))?;
        }
        let text = serde_json::to_string_pretty(self)
            .map_err(|error| GraphWorkspaceError::Serialize(path.clone(), error))?;
        let temporary = path.with_extension(format!("json.tmp{}", std::process::id()));
        std::fs::write(&temporary, format!("{text}\n"))
            .map_err(|error| GraphWorkspaceError::Write(temporary.clone(), error))?;
        std::fs::rename(&temporary, &path)
            .map_err(|error| GraphWorkspaceError::Write(path.clone(), error))?;
        Ok(path)
    }
}

fn contains_control(value: &str) -> bool {
    value.chars().any(char::is_control)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn system_nodes() -> HashMap<String, String> {
        HashMap::from([
            ("agent:codex".to_string(), "agent".to_string()),
            ("agent:opencode".to_string(), "agent".to_string()),
            ("role:planner".to_string(), "role".to_string()),
        ])
    }

    #[test]
    fn missing_workspace_loads_as_empty_version_one() {
        let dir = TempDir::new().unwrap();
        let response = GraphWorkspace::load(dir.path()).unwrap();
        assert_eq!(response.workspace.version, VERSION);
        assert!(response.workspace.nodes.is_empty());
        assert!(response.warning.is_none());
    }

    #[test]
    fn positions_and_edges_round_trip_through_atomic_file() {
        let dir = TempDir::new().unwrap();
        let workspace = GraphWorkspace {
            nodes: BTreeMap::from([
                (
                    "agent:codex".to_string(),
                    GraphPosition { x: 418.0, y: 216.0 },
                ),
                (
                    "agent:opencode".to_string(),
                    GraphPosition { x: 620.0, y: 250.0 },
                ),
            ]),
            edges: vec![WorkspaceEdge {
                id: "edge:custom:one".to_string(),
                source: "agent:codex".to_string(),
                target: "agent:opencode".to_string(),
                kind: "custom".to_string(),
            }],
            ..GraphWorkspace::default()
        };
        workspace.validate(&system_nodes()).unwrap();
        let path = workspace.write_atomic(dir.path()).unwrap();
        assert_eq!(path, GraphWorkspace::path(dir.path()));
        assert!(!path.with_extension("json.tmp0").exists());

        let loaded = GraphWorkspace::load(dir.path()).unwrap();
        assert_eq!(loaded.workspace.nodes["agent:codex"].x, 418.0);
        assert_eq!(loaded.workspace.edges, workspace.edges);
    }

    #[test]
    fn malformed_workspace_is_ignored_safely() {
        let dir = TempDir::new().unwrap();
        let path = GraphWorkspace::path(dir.path());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "{not json").unwrap();

        let response = GraphWorkspace::load(dir.path()).unwrap();
        assert!(response.workspace.nodes.is_empty());
        assert!(response
            .warning
            .unwrap()
            .contains("malformed graph workspace"));
    }

    #[test]
    fn validation_rejects_invalid_endpoints_and_duplicate_links() {
        let invalid = GraphWorkspace {
            edges: vec![WorkspaceEdge {
                id: "edge:custom:missing".to_string(),
                source: "agent:codex".to_string(),
                target: "agent:missing".to_string(),
                kind: "custom".to_string(),
            }],
            ..GraphWorkspace::default()
        };
        assert!(invalid
            .validate(&system_nodes())
            .unwrap_err()
            .contains("unknown target"));

        let duplicate = GraphWorkspace {
            edges: vec![
                WorkspaceEdge {
                    id: "edge:custom:one".to_string(),
                    source: "agent:codex".to_string(),
                    target: "agent:opencode".to_string(),
                    kind: "custom".to_string(),
                },
                WorkspaceEdge {
                    id: "edge:custom:two".to_string(),
                    source: "agent:codex".to_string(),
                    target: "agent:opencode".to_string(),
                    kind: "custom".to_string(),
                },
            ],
            ..GraphWorkspace::default()
        };
        assert!(duplicate
            .validate(&system_nodes())
            .unwrap_err()
            .contains("duplicate custom edge"));
    }

    #[test]
    fn stale_positions_are_removed_without_touching_known_nodes() {
        let mut workspace = GraphWorkspace {
            nodes: BTreeMap::from([
                (
                    "agent:codex".to_string(),
                    GraphPosition { x: 10.0, y: 20.0 },
                ),
                (
                    "agent:removed".to_string(),
                    GraphPosition { x: 30.0, y: 40.0 },
                ),
            ]),
            ..GraphWorkspace::default()
        };
        assert!(workspace.retain_known(&system_nodes()));
        assert!(workspace.nodes.contains_key("agent:codex"));
        assert!(!workspace.nodes.contains_key("agent:removed"));
    }
}
