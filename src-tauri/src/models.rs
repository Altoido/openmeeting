use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct Meeting {
    pub id: String,
    pub title: String,
    pub created_at: f64,
    pub duration: f64,
    pub audio_file: String,
    pub original_file_name: Option<String>,
    pub source: String,         // recording | imported
    pub status: String,         // recorded queued preparing transcribing summarizing ready transcribed failed interrupted
    pub status_detail: String,
    pub progress: f64,
    pub language: String,
    pub collection_id: Option<String>,
    pub is_favorite: bool,
    pub is_archived: bool,
    pub user_notes: String,
    pub template_id: String,
    pub stereo_sources: bool,
    pub transcribed_until: f64,
    pub title_user_set: bool,
    pub tags: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Segment {
    pub id: i64,
    pub index: i64,
    pub start: f64,
    pub end: f64,
    pub text: String,
    pub original_text: String,
    pub speaker: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ActionItem {
    pub id: String,
    pub meeting_id: String,
    pub task: String,
    pub owner: String,
    pub due: String,
    pub priority: String,
    pub done: bool,
    pub notes: String,
    pub source_time: Option<f64>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    pub id: i64,
    pub template_id: String,
    pub template_name: String,
    pub markdown: String,
    pub structured_json: String,
    pub model_name: String,
    pub created_at: f64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ChatMessage {
    pub id: i64,
    pub meeting_id: Option<String>,
    pub role: String,
    pub content: String,
    pub created_at: f64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Highlight { pub id: i64, pub time: f64, pub label: String }

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Collection { pub id: String, pub name: String }

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SearchHit { pub meeting_id: String, pub kind: String, pub r#ref: String, pub snippet: String }

/// Structured facts extracted from a transcript. Lenient: missing fields default to empty.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct Notes {
    pub purpose: String,
    pub executive_summary: String,
    pub topics: Vec<Topic>,
    pub decisions: Vec<Decision>,
    pub action_items: Vec<Action>,
    pub open_questions: Vec<String>,
    pub next_steps: Vec<String>,
    pub quotes: Vec<Quote>,
}
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct Topic { pub title: String, pub details: Vec<String> }
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct Decision { pub decision: String, pub timestamp: Option<String> }
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct Action { pub task: String, pub owner: Option<String>, pub due: Option<String>, pub priority: Option<String>, pub timestamp: Option<String> }
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct Quote { pub speaker: Option<String>, pub quote: String, pub timestamp: Option<String> }

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct MindNode { pub label: String, #[serde(default)] pub children: Vec<MindNode> }
