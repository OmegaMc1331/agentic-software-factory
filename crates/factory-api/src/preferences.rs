use std::sync::Arc;

use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use rusqlite::{params, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::application::FactoryApplication;

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Theme {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Density {
    #[default]
    Comfortable,
    Compact,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub(crate) enum Layout {
    #[default]
    Focus,
    Code,
    Agents,
    Review,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Preferences {
    pub schema_version: u8,
    pub theme: Theme,
    pub density: Density,
    pub editor_font_size: u8,
    pub editor_tab_size: u8,
    pub editor_word_wrap: bool,
    pub default_layout: Layout,
    pub notification_toasts: bool,
    pub browser_notifications: bool,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            schema_version: 1,
            theme: Theme::default(),
            density: Density::default(),
            editor_font_size: 13,
            editor_tab_size: 4,
            editor_word_wrap: false,
            default_layout: Layout::default(),
            notification_toasts: true,
            browser_notifications: false,
        }
    }
}

impl Preferences {
    fn validate(&self) -> Result<(), Error> {
        if self.schema_version != 1
            || !(11..=24).contains(&self.editor_font_size)
            || !(1..=8).contains(&self.editor_tab_size)
        {
            return Err(Error::Invalid);
        }
        Ok(())
    }
}

#[derive(Debug)]
pub(crate) enum Error {
    Invalid,
    Conflict,
    Unavailable,
}
impl IntoResponse for Error {
    fn into_response(self) -> Response {
        let (status, code, message) = match self {
            Self::Invalid => (StatusCode::BAD_REQUEST, "invalid_preferences", "Unsupported preference or value."),
            Self::Conflict => (StatusCode::CONFLICT, "stale_preferences", "Another window changed application preferences. Reload before saving; your edits are retained."),
            Self::Unavailable => (StatusCode::SERVICE_UNAVAILABLE, "preferences_unavailable", "Application preferences could not be stored. Existing values are retained."),
        };
        (status, Json(json!({"error":message,"code":code}))).into_response()
    }
}
impl From<rusqlite::Error> for Error {
    fn from(_: rusqlite::Error) -> Self {
        Self::Unavailable
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Envelope {
    pub revision: i64,
    pub preferences: Preferences,
}

impl crate::workspaces::WorkspaceStore {
    pub(crate) fn preferences(&self) -> Result<Envelope, Error> {
        let connection = self.connect().map_err(|_| Error::Unavailable)?;
        let row: Option<(i64, String)> = connection
            .query_row(
                "SELECT revision,payload FROM application_preferences WHERE id=1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let (revision, preferences) = match row {
            Some((revision, payload)) => (
                revision,
                serde_json::from_str::<Preferences>(&payload).map_err(|_| Error::Unavailable)?,
            ),
            None => (0, Preferences::default()),
        };
        preferences.validate().map_err(|_| Error::Unavailable)?;
        Ok(Envelope {
            revision,
            preferences,
        })
    }

    pub(crate) fn save_preferences(&self, update: Envelope) -> Result<Envelope, Error> {
        update.preferences.validate()?;
        if update.revision < 0 || update.revision == i64::MAX {
            return Err(Error::Invalid);
        }
        let mut connection = self.connect().map_err(|_| Error::Unavailable)?;
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current: i64 = tx
            .query_row(
                "SELECT revision FROM application_preferences WHERE id=1",
                [],
                |r| r.get(0),
            )
            .optional()?
            .unwrap_or(0);
        if current != update.revision {
            return Err(Error::Conflict);
        }
        let revision = current + 1;
        let payload = serde_json::to_string(&update.preferences).map_err(|_| Error::Invalid)?;
        tx.execute("INSERT INTO application_preferences(id,revision,payload) VALUES(1,?1,?2) ON CONFLICT(id) DO UPDATE SET revision=excluded.revision,payload=excluded.payload", params![revision,payload])?;
        tx.commit()?;
        Ok(Envelope {
            revision,
            preferences: update.preferences,
        })
    }
}

pub(crate) async fn read(
    State(app): State<Arc<FactoryApplication>>,
) -> Result<Json<Envelope>, Error> {
    let permit = app.workspaces.permit().map_err(|_| Error::Unavailable)?;
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        app.workspaces.preferences()
    })
    .await
    .map_err(|_| Error::Unavailable)?
    .map(Json)
}

pub(crate) async fn save(
    State(app): State<Arc<FactoryApplication>>,
    Json(update): Json<Envelope>,
) -> Result<Json<Envelope>, Error> {
    let permit = app.workspaces.permit().map_err(|_| Error::Unavailable)?;
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        app.workspaces.save_preferences(update)
    })
    .await
    .map_err(|_| Error::Unavailable)?
    .map(Json)
}

pub(crate) fn project_routes() -> axum::Router<crate::app::SharedState> {
    axum::Router::new().route("/api/settings", axum::routing::get(project_summary))
}

fn policy_summary(scope: Option<&factory_policy::PolicyScope>) -> serde_json::Value {
    json!({
        "configured":scope.is_some(),
        "preset":scope.and_then(|p|p.preset),
        "commands_mode":scope.and_then(|p|p.commands.as_ref().map(|c|c.mode)),
        "network_mode":scope.and_then(|p|p.network.as_ref().map(|n|n.mode)),
        "read_scope_count":scope.and_then(|p|p.filesystem.as_ref().map(|f|f.read.len())),
        "write_scope_count":scope.and_then(|p|p.filesystem.as_ref().map(|f|f.write.len())),
        "deny_write_scope_count":scope.and_then(|p|p.filesystem.as_ref().map(|f|f.deny_write.len())),
        "git_operation_count":scope.and_then(|p|p.git.as_ref().map(|g|g.allow.len())),
        "note":"Unset dimensions inherit existing policy. Inspect the selected agent's resolved permission view before execution. This summary cannot grant permissions."
    })
}

async fn project_summary(
    State(state): State<crate::app::SharedState>,
) -> Result<Json<serde_json::Value>, Error> {
    let project = state
        ._context
        .as_ref()
        .ok_or(Error::Invalid)?
        .project_id
        .to_string();
    tokio::task::spawn_blocking(move|| {
        let (config,revision)=factory_core::Config::load_with_revision(&state.root).map_err(|_|Error::Unavailable)?;
        let roles=config.policies.roles.iter().map(|(id,scope)|(id.clone(),policy_summary(Some(scope)))).collect::<serde_json::Map<_,_>>();
        Ok(Json(json!({"schema_version":1,"scope":"project","project_id":project,"revision":revision,"runtime":config.runtime,"scheduler":config.scheduler,"routing":config.routing,"integration_check_count":config.integration_checks.len(),"policies":{"ceiling":policy_summary(config.policies.ceiling.as_ref()),"roles":roles},"roles":config.roles.keys().collect::<Vec<_>>(),"agent_count":config.agents.len(),"mcp_server_count":config.mcp.len(),"configuration_editor":"existing project config API/TOML with revision checks","privacy":"Environment values, provider arguments, tool headers, command strings and role instructions are excluded."})))
    }).await.map_err(|_|Error::Unavailable)?
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspaces::WorkspaceStore;

    #[test]
    fn preferences_persist_reject_stale_and_invalid_and_preserve_workspaces() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("workspace.sqlite3");
        let old = rusqlite::Connection::open(&path).unwrap();
        old.execute_batch("CREATE TABLE workspaces(id TEXT PRIMARY KEY,revision INTEGER NOT NULL,document TEXT NOT NULL,updated_at TEXT NOT NULL);INSERT INTO workspaces VALUES('retained',7,'{}','now');PRAGMA user_version=1;").unwrap();
        drop(old);
        let store = WorkspaceStore::new(path.clone());
        let mut value = store.preferences().unwrap();
        assert_eq!(value.revision, 0);
        value.preferences.theme = Theme::Dark;
        assert_eq!(store.save_preferences(value).unwrap().revision, 1);
        assert!(matches!(
            store.save_preferences(Envelope {
                revision: 0,
                preferences: Preferences::default()
            }),
            Err(Error::Conflict)
        ));
        let reopened = WorkspaceStore::new(path);
        assert_eq!(
            reopened.preferences().unwrap().preferences.theme,
            Theme::Dark
        );
        let mut invalid = reopened.preferences().unwrap();
        invalid.preferences.editor_font_size = 250;
        assert!(matches!(
            reopened.save_preferences(invalid),
            Err(Error::Invalid)
        ));
        let conn = reopened.connect().unwrap();
        assert_eq!(
            conn.query_row(
                "SELECT revision FROM workspaces WHERE id='retained'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            7
        );
        conn.execute("UPDATE application_preferences SET payload='not json'", [])
            .unwrap();
        assert!(matches!(reopened.preferences(), Err(Error::Unavailable)));
    }
}
