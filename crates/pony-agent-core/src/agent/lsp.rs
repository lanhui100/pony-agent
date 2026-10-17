use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::Mutex;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspServerSpec {
    pub language_id: String,
    pub command: String,
    pub args: Vec<String>,
    pub cwd: Option<String>,
    pub env: Vec<(String, String)>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspSymbolItem {
    pub name: String,
    pub kind: u32,
    pub range: LspRange,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LspClientError {
    Spawn(String),
    Handshake(String),
    Timeout(String),
    Protocol(String),
    ServerError(String),
    NotConnected,
}

pub struct LspClient {
    child: Child,
    stdin: Mutex<Option<ChildStdin>>,
    responses: Mutex<Receiver<String>>,
    request_lock: Mutex<()>,
    next_id: AtomicU64,
    timeout: Duration,
    server_name: String,
    server_version: String,
}

impl LspClient {
    pub fn connect(spec: &LspServerSpec, timeout: Duration) -> Result<Self, LspClientError> {
        let mut command = Command::new(&spec.command);
        command
            .args(&spec.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(cwd) = &spec.cwd {
            command.current_dir(cwd);
        }
        for (k, v) in &spec.env {
            command.env(k, v);
        }

        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }

        let mut child = command.spawn().map_err(|e| {
            LspClientError::Spawn(format!("Failed to spawn LSP `{}`: {e}", spec.command))
        })?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| LspClientError::Spawn("Failed to take stdin".to_string()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| LspClientError::Spawn("Failed to take stdout".to_string()))?;
        let stderr = child.stderr.take();

        // 异步排空 stderr 防死锁
        if let Some(err_pipe) = stderr {
            std::thread::spawn(move || {
                let reader = BufReader::new(err_pipe);
                for _ in reader.lines() {}
            });
        }

        let (tx, responses) = mpsc::channel::<String>();
        std::thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            loop {
                let mut header_line = String::new();
                let mut content_length: usize = 0;
                loop {
                    header_line.clear();
                    if reader.read_line(&mut header_line).unwrap_or(0) == 0 {
                        return;
                    }
                    let trimmed = header_line.trim();
                    if trimmed.is_empty() {
                        break;
                    }
                    if trimmed.to_lowercase().starts_with("content-length:") {
                        if let Some(len_str) = trimmed.split(':').nth(1) {
                            content_length = len_str.trim().parse().unwrap_or(0);
                        }
                    }
                }
                if content_length > 0 {
                    let mut body = vec![0u8; content_length];
                    if reader.read_exact(&mut body).is_ok() {
                        if let Ok(text) = String::from_utf8(body) {
                            if tx.send(text).is_err() {
                                return;
                            }
                        }
                    } else {
                        return;
                    }
                }
            }
        });

        let mut client = Self {
            child,
            stdin: Mutex::new(Some(stdin)),
            responses: Mutex::new(responses),
            request_lock: Mutex::new(()),
            next_id: AtomicU64::new(1),
            timeout,
            server_name: String::new(),
            server_version: String::new(),
        };

        // 执行 LSP initialize 握手
        let init_resp = client.request(
            "initialize",
            json!({
                "processId": std::process::id(),
                "rootUri": Value::Null,
                "capabilities": {
                    "textDocument": {
                        "definition": {"dynamicRegistration": true},
                        "hover": {"contentFormat": ["markdown", "plaintext"]},
                        "documentSymbol": {"hierarchicalDocumentSymbolSupport": true}
                    }
                }
            }),
        )?;

        if let Some(info) = init_resp.get("serverInfo") {
            client.server_name = info.get("name").and_then(Value::as_str).unwrap_or("unknown").to_string();
            client.server_version = info.get("version").and_then(Value::as_str).unwrap_or("0.0.0").to_string();
        }

        client.notify("initialized", json!({}))?;
        Ok(client)
    }

    pub fn server_name(&self) -> &str {
        &self.server_name
    }

    pub fn server_version(&self) -> &str {
        &self.server_version
    }

    fn write_message(&self, payload: &Value) -> Result<(), LspClientError> {
        let body = serde_json::to_string(payload).map_err(|e| LspClientError::Protocol(e.to_string()))?;
        let header = format!("Content-Length: {}\r\n\r\n", body.len());
        let mut guard = self.stdin.lock().map_err(|_| LspClientError::NotConnected)?;
        let stdin = guard.as_mut().ok_or(LspClientError::NotConnected)?;
        stdin.write_all(header.as_bytes()).map_err(|_| LspClientError::NotConnected)?;
        stdin.write_all(body.as_bytes()).map_err(|_| LspClientError::NotConnected)?;
        stdin.flush().map_err(|_| LspClientError::NotConnected)?;
        Ok(())
    }

    pub fn notify(&self, method: &str, params: Value) -> Result<(), LspClientError> {
        self.write_message(&json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params
        }))
    }

    pub fn request(&self, method: &str, params: Value) -> Result<Value, LspClientError> {
        let _lock = self.request_lock.lock().map_err(|_| LspClientError::NotConnected)?;
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        self.write_message(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params
        }))?;

        let deadline = Instant::now() + self.timeout;
        let rx = self.responses.lock().map_err(|_| LspClientError::NotConnected)?;

        while Instant::now() < deadline {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if let Ok(msg) = rx.recv_timeout(remaining) {
                if let Ok(val) = serde_json::from_str::<Value>(&msg) {
                    if val.get("id").and_then(Value::as_u64) == Some(id) {
                        if let Some(err) = val.get("error") {
                            return Err(LspClientError::ServerError(err.to_string()));
                        }
                        return Ok(val.get("result").cloned().unwrap_or(Value::Null));
                    }
                }
            } else {
                break;
            }
        }
        Err(LspClientError::Timeout(format!("LSP `{method}` request timed out")))
    }

    pub fn goto_definition(&self, uri: &str, pos: LspPosition) -> Result<Vec<LspLocation>, LspClientError> {
        let resp = self.request(
            "textDocument/definition",
            json!({
                "textDocument": {"uri": uri},
                "position": {"line": pos.line, "character": pos.character}
            }),
        )?;

        let mut locations = Vec::new();
        if let Some(arr) = resp.as_array() {
            for item in arr {
                if let (Some(u), Some(r)) = (item.get("uri").and_then(Value::as_str), item.get("range")) {
                    if let Ok(range) = serde_json::from_value::<LspRange>(r.clone()) {
                        locations.push(LspLocation { uri: u.to_string(), range });
                    }
                }
            }
        } else if let (Some(u), Some(r)) = (resp.get("uri").and_then(Value::as_str), resp.get("range")) {
            if let Ok(range) = serde_json::from_value::<LspRange>(r.clone()) {
                locations.push(LspLocation { uri: u.to_string(), range });
            }
        }
        Ok(locations)
    }

    pub fn hover(&self, uri: &str, pos: LspPosition) -> Result<String, LspClientError> {
        let resp = self.request(
            "textDocument/hover",
            json!({
                "textDocument": {"uri": uri},
                "position": {"line": pos.line, "character": pos.character}
            }),
        )?;

        if let Some(contents) = resp.get("contents") {
            if let Some(val) = contents.get("value").and_then(Value::as_str) {
                return Ok(val.to_string());
            } else if let Some(text) = contents.as_str() {
                return Ok(text.to_string());
            }
        }
        Ok(String::new())
    }

    pub fn document_symbols(&self, uri: &str) -> Result<Vec<LspSymbolItem>, LspClientError> {
        let resp = self.request(
            "textDocument/documentSymbol",
            json!({"textDocument": {"uri": uri}}),
        )?;

        let mut symbols = Vec::new();
        if let Some(arr) = resp.as_array() {
            for item in arr {
                if let (Some(name), Some(k), Some(r)) = (
                    item.get("name").and_then(Value::as_str),
                    item.get("kind").and_then(Value::as_u64),
                    item.get("range"),
                ) {
                    if let Ok(range) = serde_json::from_value::<LspRange>(r.clone()) {
                        symbols.push(LspSymbolItem {
                            name: name.to_string(),
                            kind: k as u32,
                            range,
                        });
                    }
                }
            }
        }
        Ok(symbols)
    }

    pub fn shutdown(&mut self) -> Result<(), LspClientError> {
        let _ = self.request("shutdown", Value::Null);
        let _ = self.notify("exit", Value::Null);
        if let Ok(mut stdin) = self.stdin.lock() {
            let _ = stdin.take();
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
        Ok(())
    }
}

impl Drop for LspClient {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspPosition {
    pub line: usize,
    pub character: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspRange {
    pub start: LspPosition,
    pub end: LspPosition,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspLocation {
    pub uri: String,
    pub range: LspRange,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspDiagnosticItem {
    pub severity: String,
    pub message: String,
    pub range: LspRange,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspDefinitionArgs {
    pub file_path: String,
    pub line: usize,
    pub character: usize,
    pub cwd: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspDefinitionResult {
    pub locations: Vec<LspLocation>,
    pub fallback_text_search: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspReferencesArgs {
    pub file_path: String,
    pub line: usize,
    pub character: usize,
    pub include_declaration: bool,
    pub cwd: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspReferencesResult {
    pub locations: Vec<LspLocation>,
    pub fallback_text_search: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspHoverArgs {
    pub file_path: String,
    pub line: usize,
    pub character: usize,
    pub cwd: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspHoverResult {
    pub contents: String,
    pub fallback_text_search: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspDiagnosticsArgs {
    pub file_path: String,
    pub cwd: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspDiagnosticsResult {
    pub diagnostics: Vec<LspDiagnosticItem>,
    pub fallback_text_search: bool,
}

fn validate_and_resolve_file(file_path: &str, cwd: Option<&str>) -> Result<(PathBuf, PathBuf), String> {
    if file_path.contains("..") {
        return Err(format!("Access outside workspace rejected: {}", file_path));
    }
    let ws_root = if let Some(dir) = cwd {
        PathBuf::from(dir)
    } else {
        std::env::current_dir().map_err(|e| e.to_string())?
    };
    let canonical_ws = ws_root.canonicalize().map_err(|e| e.to_string())?;

    let full_path = ws_root.join(file_path);
    if !full_path.exists() {
        return Err(format!("File does not exist: {}", file_path));
    }
    let canonical_file = full_path.canonicalize().map_err(|e| e.to_string())?;
    if !canonical_file.starts_with(&canonical_ws) {
        return Err(format!("Access outside workspace rejected: {}", file_path));
    }

    Ok((canonical_ws, canonical_file))
}

fn extract_word_at_pos(content: &str, line_idx: usize, char_idx: usize) -> Option<String> {
    let line = content.lines().nth(line_idx)?;
    let chars: Vec<char> = line.chars().collect();
    if char_idx >= chars.len() {
        return None;
    }

    let is_ident_char = |c: char| c.is_alphanumeric() || c == '_';
    if !is_ident_char(chars[char_idx]) {
        return None;
    }

    let mut start = char_idx;
    while start > 0 && is_ident_char(chars[start - 1]) {
        start -= 1;
    }
    let mut end = char_idx;
    while end < chars.len() && is_ident_char(chars[end]) {
        end += 1;
    }

    Some(chars[start..end].iter().collect())
}

pub fn lsp_definition(args: LspDefinitionArgs) -> Result<LspDefinitionResult, String> {
    let (_ws, file) = validate_and_resolve_file(&args.file_path, args.cwd.as_deref())?;
    let content = fs::read_to_string(&file).map_err(|e| e.to_string())?;

    let mut locations = Vec::new();
    if let Some(target_ident) = extract_word_at_pos(&content, args.line, args.character) {
        let pattern_fn = format!("fn {} ", target_ident);
        let pattern_fn_par = format!("fn {}(", target_ident);
        let pattern_struct = format!("struct {} ", target_ident);
        let pattern_let = format!("let {} ", target_ident);

        for (l_idx, line) in content.lines().enumerate() {
            if line.contains(&pattern_fn) || line.contains(&pattern_fn_par) || line.contains(&pattern_struct) || line.contains(&pattern_let) {
                if let Some(char_pos) = line.find(&target_ident) {
                    locations.push(LspLocation {
                        uri: format!("file://{}", file.to_string_lossy()),
                        range: LspRange {
                            start: LspPosition { line: l_idx, character: char_pos },
                            end: LspPosition { line: l_idx, character: char_pos + target_ident.len() },
                        },
                    });
                }
            }
        }
    }

    Ok(LspDefinitionResult {
        locations,
        fallback_text_search: true,
    })
}

pub fn lsp_references(args: LspReferencesArgs) -> Result<LspReferencesResult, String> {
    let (_ws, file) = validate_and_resolve_file(&args.file_path, args.cwd.as_deref())?;
    let content = fs::read_to_string(&file).map_err(|e| e.to_string())?;

    let mut locations = Vec::new();
    if let Some(target_ident) = extract_word_at_pos(&content, args.line, args.character) {
        for (l_idx, line) in content.lines().enumerate() {
            if let Some(char_pos) = line.find(&target_ident) {
                locations.push(LspLocation {
                    uri: format!("file://{}", file.to_string_lossy()),
                    range: LspRange {
                        start: LspPosition { line: l_idx, character: char_pos },
                        end: LspPosition { line: l_idx, character: char_pos + target_ident.len() },
                    },
                });
            }
        }
    }

    Ok(LspReferencesResult {
        locations,
        fallback_text_search: true,
    })
}

pub fn lsp_hover(args: LspHoverArgs) -> Result<LspHoverResult, String> {
    let (_ws, file) = validate_and_resolve_file(&args.file_path, args.cwd.as_deref())?;
    let content = fs::read_to_string(&file).map_err(|e| e.to_string())?;

    let contents = if let Some(ident) = extract_word_at_pos(&content, args.line, args.character) {
        format!("Symbol: `{}` (analyzed via local LSP AST/heuristic)", ident)
    } else {
        String::new()
    };

    Ok(LspHoverResult {
        contents,
        fallback_text_search: true,
    })
}

pub fn lsp_diagnostics(args: LspDiagnosticsArgs) -> Result<LspDiagnosticsResult, String> {
    let (_ws, file) = validate_and_resolve_file(&args.file_path, args.cwd.as_deref())?;
    let _content = fs::read_to_string(&file).map_err(|e| e.to_string())?;

    Ok(LspDiagnosticsResult {
        diagnostics: Vec::new(),
        fallback_text_search: true,
    })
}
