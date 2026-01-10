// todo.rs

use chrono::Local;
use serde::{Deserialize, Serialize};
fn default_project() -> String { "Inbox".to_string() }
#[derive(Serialize, Deserialize)]
pub struct Todo {
    pub description: String,
    pub due_date: Option<String>,
    pub created_date: String,
    pub done: bool,
    #[serde(default = "default_project")]
    pub project: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub planka_card_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub planka_list_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub planka_board_id: Option<String>,
}

impl Todo {
    pub fn new(description: String, due_date: Option<String>) -> Self {
        Self {
            description,
            due_date,
            created_date: Local::now().format("%Y-%m-%d").to_string(),
            done: false,
            project: default_project(),
            planka_card_id: None,
            planka_list_id: None,
            planka_board_id: None,
        }
    }
}
