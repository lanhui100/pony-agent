use std::io::{self, BufRead, Write};
use std::sync::Arc;
use serde_json::Value;
use crate::agent::control_plane::HostControlPlane;
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
                        "promptCapabilities": true
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
