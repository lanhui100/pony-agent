//! Cross-process Client SDK for Pony Agent (B085)
//! 
//! Communicates with the Pony Core ACP server over stdio via JSON-RPC 2.0.

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, Command, Stdio};
use serde_json::Value;

pub struct PonyClient {
    child: Child,
}

impl PonyClient {
    pub fn spawn(core_bin_path: &str) -> std::io::Result<Self> {
        let child = Command::new(core_bin_path)
            .arg("acp")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()?;
        Ok(Self { child })
    }

    pub fn send_request(&mut self, method: &str, params: Option<Value>) -> std::io::Result<Value> {
        let req = serde_json::json!({
            "id": 1,
            "method": method,
            "params": params,
        });

        let stdin = self.child.stdin.as_mut().ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::BrokenPipe, "Failed to open stdin")
        })?;
        writeln!(stdin, "{}", serde_json::to_string(&req)?)?;
        stdin.flush()?;

        let stdout = self.child.stdout.as_mut().ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::BrokenPipe, "Failed to open stdout")
        })?;
        let mut reader = BufReader::new(stdout);
        let mut line = String::new();
        reader.read_line(&mut line)?;

        let resp: Value = serde_json::from_str(&line)?;
        Ok(resp)
    }
}
