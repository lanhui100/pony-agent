use std::io::{self, BufRead, Write};
use std::sync::Arc;
use serde_json::Value;
use crate::agent::control_plane::HostControlPlane;
use crate::agent::ask_control::{self, answer_ask, list_pending_asks};
use crate::agent::dispatcher::ControlRequestAuthorization;
use super::protocol::{Request, Response, RpcError};

pub struct AcpServer {
    control_plane: Arc<HostControlPlane>,
}

impl AcpServer {
    pub fn new(control_plane: Arc<HostControlPlane>) -> Self {
        Self { control_plane }
    }

    pub fn run_stdio_loop(&self) -> io::Result<()> {
        let stdin = io::stdin();
        let mut stdout = io::stdout();
        let mut handle = stdin.lock();
        let mut line = String::new();

        while handle.read_line(&mut line)? > 0 {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                line.clear();
                continue;
            }

            let resp = match serde_json::from_str::<Request>(trimmed) {
                Ok(req) => self.handle_request(req),
                Err(err) => Response {
                    id: Value::Null,
                    result: None,
                    error: Some(RpcError {
                        code: -32700,
                        message: format!("Parse error: {err}"),
                        data: None,
                    }),
                },
            };

            let out_str = serde_json::to_string(&resp).unwrap_or_else(|_| "{}".to_string());
            writeln!(stdout, "{}", out_str)?;
            stdout.flush()?;
            line.clear();
        }

        Ok(())
    }

    fn handle_request(&self, req: Request) -> Response {
        match req.method.as_str() {
            "initialize" => Response {
                id: req.id,
                result: Some(serde_json::json!({
                    "protocolVersion": "0.1",
                    "agentCapabilities": {
                        "loadSession": true,
                        "promptCapabilities": true,
                        "permissionRequest": true
                    }
                })),
                error: None,
            },
            "session/new" => {
                let session_id = format!("acp-{}", uuid_simple());
                Response {
                    id: req.id,
                    result: Some(serde_json::json!({
                        "sessionId": session_id
                    })),
                    error: None,
                }
            },
            "session/cancel" => {
                Response {
                    id: req.id,
                    result: Some(serde_json::json!({ "cancelled": true })),
                    error: None,
                }
            },
            "session/request_permission" => {
                // ACP 反向权限审批请求处理：对接 ask_control 授权机制
                // 期望入参包含 request_id 与 decision (allow / deny / answer)
                if let Some(params) = req.params {
                    let request_id = params.get("requestId").and_then(|v| v.as_str()).unwrap_or_default();
                    let decision = params.get("decision").and_then(|v| v.as_str()).unwrap_or("deny");
                    
                    if request_id.is_empty() {
                        return Response {
                            id: req.id,
                            result: None,
                            error: Some(RpcError {
                                code: -32602,
                                message: "Missing requestId in params".to_string(),
                                data: None,
                            }),
                        };
                    }

                    // 检索待消费的 ask
                    let asks = list_pending_asks(&self.control_plane.ask_dispatcher);
                    if let Some(pending) = asks.iter().find(|a| a.request_id == request_id) {
                        let answer_val = if decision == "allow" {
                            Some(serde_json::json!({ "approved": true }))
                        } else {
                            Some(serde_json::json!({ "approved": false }))
                        };
                        let auth = ControlRequestAuthorization::for_request(pending, answer_val);
                        match answer_ask(&self.control_plane.ask_dispatcher, request_id, &auth) {
                            Ok(consumed) => Response {
                                id: req.id,
                                result: Some(serde_json::json!({
                                    "status": "consumed",
                                    "requestId": request_id,
                                    "state": format!("{:?}", consumed.request.state)
                                })),
                                error: None,
                            },
                            Err(e) => Response {
                                id: req.id,
                                result: None,
                                error: Some(RpcError {
                                    code: -32001,
                                    message: format!("CAS verification failed: {e}"),
                                    data: None,
                                }),
                            },
                        }
                    } else {
                        Response {
                            id: req.id,
                            result: None,
                            error: Some(RpcError {
                                code: -32004,
                                message: format!("Pending request {request_id} not found or expired"),
                                data: None,
                            }),
                        }
                    }
                } else {
                    Response {
                        id: req.id,
                        result: None,
                        error: Some(RpcError {
                            code: -32602,
                            message: "Missing params object".to_string(),
                            data: None,
                        }),
                    }
                }
            },
            _ => Response {
                id: req.id,
                result: None,
                error: Some(RpcError {
                    code: -32601,
                    message: format!("Method not found: {}", req.method),
                    data: None,
                }),
            },
        }
    }
}

fn uuid_simple() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(0)
}
