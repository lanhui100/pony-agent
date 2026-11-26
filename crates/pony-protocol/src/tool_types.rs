use crate::tool_contract::ToolError;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub call_id: Option<String>,
    pub name: String,
    pub arguments: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan: Option<ToolPlan>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    pub tool_name: String,
    pub status: String,
    pub output: String,
    pub duration_ms: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ToolExecutionStatus {
    Ok,
    Error,
    Cancelled,
}

impl ToolExecutionStatus {
    pub fn from_legacy_status(status: &str) -> Self {
        match status {
            "ok" => Self::Ok,
            "aborted" | "cancelled" => Self::Cancelled,
            _ => Self::Error,
        }
    }

    pub fn as_legacy_status(&self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Error => "error",
            Self::Cancelled => "aborted",
        }
    }
}


#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ToolControlKind {
    WaitingUser,
    ApprovalRequired,
    WaitingHost,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ToolControlOutcome {
    pub kind: ToolControlKind,
    pub request_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolOutcome {
    pub execution_status: ToolExecutionStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub control_outcome: Option<ToolControlOutcome>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<ToolResult>,
}

impl ToolOutcome {
    pub fn from_legacy_result(result: ToolResult) -> Self {
        Self {
            execution_status: ToolExecutionStatus::from_legacy_status(&result.status),
            control_outcome: None,
            result: Some(result),
        }
    }

    pub fn pending(control_outcome: ToolControlOutcome) -> Self {
        Self {
            execution_status: ToolExecutionStatus::Ok,
            control_outcome: Some(control_outcome),
            result: None,
        }
    }

    pub fn into_legacy_result(self, tool_name: impl Into<String>) -> ToolResult {
        self.result.unwrap_or_else(|| ToolResult {
            tool_name: tool_name.into(),
            // A pending control request is not a provider-consumable tool result. Until the
            // runtime consumes it and resumes the original call, legacy callers must fail closed.
            status: "error".to_string(),
            output: json!({
                "ok": false,
                "error": {
                    "code": "control_outcome_pending",
                    "kind": "control_outcome_pending",
                    "message": "工具调用尚处于控制请求状态，尚未产生可供 provider 消费的终态结果。"
                }
            })
            .to_string(),
            duration_ms: 0,
        })
    }
}


#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolPlanStep {
    pub name: String,
    pub arguments: Value,
    pub summary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolPlan {
    pub kind: String,
    pub summary: String,
    #[serde(default)]
    pub parallel: bool,
    #[serde(default)]
    pub continue_on_error: bool,
    pub steps: Vec<ToolPlanStep>,
}


pub fn error_result(tool_name: &str, code: &str, message: String, hint: Option<String>) -> ToolResult {
    ToolResult {
        tool_name: tool_name.to_string(),
        status: "error".to_string(),
        output: json_string(json!({
            "ok": false,
            "tool": tool_name,
            "error": {
                // Stage1 双写（A2）：kind 与 code 并存且值一致。
                "code": code,
                "kind": code,
                "message": message,
                "hint": hint,
            },
            "summary": {
                "text": message
            }
        })),
        duration_ms: 0,
    }
}

pub fn tool_error_from_output(status: &str, parsed: &Value) -> Option<ToolError> {
    if status == "ok" {
        return None;
    }

    let error = parsed.get("error")?;
    // B5：纯字符串 error 形状保文本：message 含原文，kind 落 unknown 标记
    //（此前丢文本回默认句；码形 A4 要求 kind 保持 ^[a-z][a-z0-9_]{1,63}$，
    // 故 kind 取 "unknown" 而非嵌入原文）。
    if let Some(raw) = error.as_str() {
        return Some(ToolError {
            kind: "unknown".to_string(),
            message: if raw.is_empty() {
                "Tool execution failed.".to_string()
            } else {
                raw.to_string()
            },
            details: None,
            retryable: None,
            source: None,
        });
    }
    Some(ToolError {
        kind: error
            // B4：冲突时 code 获胜（code 优先，唯一行为变更）。
            .get("code")
            .or_else(|| error.get("kind"))
            .and_then(Value::as_str)
            .unwrap_or("tool_error")
            .to_string(),
        message: error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("Tool execution failed.")
            .to_string(),
        details: error.get("details").cloned(),
        retryable: error.get("retryable").and_then(Value::as_bool),
        source: error
            .get("source")
            .and_then(Value::as_str)
            .map(ToString::to_string),
    })
}

pub fn aborted_result(tool_name: &str, code: &str, message: String) -> ToolResult {
    ToolResult {
        tool_name: tool_name.to_string(),
        status: "aborted".to_string(),
        output: json_string(json!({
            "ok": false,
            "tool": tool_name,
            "status": "aborted",
            "error": {
                // Stage1 双写：kind 与 code 并存且值一致。
                "code": code,
                "kind": code,
                "message": message,
                "hint": Option::<String>::None,
            }
        })),
        duration_ms: 0,
    }
}

/// wave-9 P0-2：与 [`error_result`] 同构的错误结果，额外把 `evidence` 对象的键**并入载荷根级**。
///
/// `error_result` 既有语义保持不变（不含证据键）；本 helper 只服务于「错误必须携带已采集物理
/// 证据」的场景（run_command 超时）。`evidence` 的键名由调用方保证与成功路径一致（禁止别名）。
pub fn error_result_with_evidence(
    tool_name: &str,
    code: &str,
    message: String,
    hint: Option<String>,
    evidence: Value,
) -> ToolResult {
    let mut payload = json!({
        "ok": false,
        "tool": tool_name,
        "error": {
            // Stage1 双写（A2）：kind 与 code 并存且值一致。
            "code": code,
            "kind": code,
            "message": message,
            "hint": hint,
        },
        "summary": {
            "text": message
        }
    });
    if let Some(root) = payload.as_object_mut() {
        if let Some(evidence) = evidence.as_object() {
            for (key, value) in evidence {
                root.insert(key.clone(), value.clone());
            }
        }
    }
    ToolResult {
        tool_name: tool_name.to_string(),
        status: "error".to_string(),
        output: json_string(payload),
        duration_ms: 0,
    }
}

pub fn json_string(value: Value) -> String {
    serde_json::to_string_pretty(&value).unwrap_or_else(|_| value.to_string())
}

pub fn parse_tool_output(output: &str) -> Value {
    serde_json::from_str::<Value>(output).unwrap_or_else(|_| Value::String(output.to_string()))
}
