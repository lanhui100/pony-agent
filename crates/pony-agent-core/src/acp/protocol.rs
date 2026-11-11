use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub struct Request {
    pub id: serde_json::Value,
    pub method: String,
    pub params: Option<serde_json::Value>,
}
