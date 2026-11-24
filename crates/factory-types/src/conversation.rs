use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Conversation {
    pub id: String,
    pub project_id: String,
    pub title: String,
    pub status: String,
    pub revision: i64,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationMessage {
    pub id: i64,
    pub conversation_id: String,
    pub role: String,
    pub content: String,
    pub status: String,
    pub run_id: Option<i64>,
    pub parent_message_id: Option<i64>,
    pub provenance: Value,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationCommand {
    pub id: i64,
    pub conversation_id: String,
    pub message_id: i64,
    pub response_id: i64,
    pub key: String,
    pub fingerprint: String,
    pub kind: String,
    pub state: String,
    pub payload: Value,
    pub run_id: Option<i64>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationEvent {
    pub schema_version: u32,
    pub id: i64,
    pub conversation_id: String,
    pub event_type: String,
    pub payload: Value,
    pub timestamp: String,
}
