use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Frame {
    pub filename: Option<String>,
    pub function: Option<String>,
    pub lineno: Option<u32>,
    pub in_app: Option<bool>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Exception {
    pub error_type: String,
    pub value: Option<String>,
    pub stacktrace: Option<Vec<Frame>>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Breadcrumb {
    pub category: String,
    pub message: String,
    pub data: Option<HashMap<String, String>>,
}

/// PonySentry ingest 载荷（Wave-4 C2：新增 `event_id` / `trace_id` 互跳字段）。
///
/// 契约：
/// - `event_id`：32 位小写十六进制（无连字符），事件唯一标识；缺省时为 `None`；
/// - `trace_id`：当前活跃 turn 的 OTel trace_id（同样 32 位小写十六进制）；
///   无活跃 trace 时必须为 `None`，**严禁**用全 0 / dummy 占位；
/// - `fingerprint`：可选分组指纹（同指纹事件在服务端聚合为同一 issue）；
/// - `culprit`：可选「肇事位置」摘要（通常取首个 in-app 栈帧）。
/// 反序列化侧必须显式保留上述字段，不得静默丢弃（`#[serde(default)]` 允许旧客户端缺省）。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct IngestPayload {
    pub platform: String,
    pub release: String,
    pub environment: String,
    pub message: Option<String>,
    pub exception: Option<Exception>,
    pub tags: Option<HashMap<String, String>>,
    pub extra: Option<serde_json::Value>,
    pub breadcrumbs: Option<Vec<Breadcrumb>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fingerprint: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub culprit: Option<String>,
}
