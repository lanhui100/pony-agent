use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TurnInputImage {
    pub data_url: String,
    pub mime_type: String,
    pub name: Option<String>,
}

impl TurnInputImage {
    pub fn base64_data(&self) -> Option<&str> {
        self.data_url.split_once(',').map(|(_, payload)| payload)
    }

    pub fn payload_size_bytes(&self) -> u64 {
        self.data_url.as_bytes().len() as u64
    }
}
