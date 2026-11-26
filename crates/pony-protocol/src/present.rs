use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PresentFileInput {
    pub path: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PresentedFileItem {
    pub path: String,
    pub description: String,
    pub size_bytes: u64,
    pub mime_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PresentArgs {
    pub files: Vec<PresentFileInput>,
    pub cwd: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PresentResult {
    pub presented_files: Vec<PresentedFileItem>,
}

fn detect_mime_type(path: &str) -> String {
    if path.ends_with(".md") {
        "text/markdown".to_string()
    } else if path.ends_with(".png") {
        "image/png".to_string()
    } else if path.ends_with(".jpg") || path.ends_with(".jpeg") {
        "image/jpeg".to_string()
    } else if path.ends_with(".json") {
        "application/json".to_string()
    } else if path.ends_with(".pdf") {
        "application/pdf".to_string()
    } else if path.ends_with(".rs") || path.ends_with(".ts") || path.ends_with(".js") {
        "text/plain".to_string()
    } else {
        "application/octet-stream".to_string()
    }
}

pub fn present(args: PresentArgs) -> Result<PresentResult, String> {
    if args.files.is_empty() {
        return Err("No files specified for present".to_string());
    }
    if args.files.len() > 4 {
        return Err("Maximum 4 files allowed per present call".to_string());
    }

    let ws_root = if let Some(ref dir) = args.cwd {
        PathBuf::from(dir)
    } else {
        std::env::current_dir().map_err(|e| e.to_string())?
    };
    let canonical_ws = ws_root.canonicalize().map_err(|e| e.to_string())?;

    let mut presented = Vec::new();
    for input in args.files {
        if input.path.contains("..") {
            return Err(format!("Access outside workspace denied: {}", input.path));
        }

        let full_path = ws_root.join(&input.path);
        if !full_path.exists() {
            return Err(format!("File does not exist: {}", input.path));
        }

        let canonical_file = full_path.canonicalize().map_err(|e| e.to_string())?;
        if !canonical_file.starts_with(&canonical_ws) {
            return Err(format!("Access outside workspace denied: {}", input.path));
        }

        let metadata = fs::metadata(&canonical_file).map_err(|e| e.to_string())?;
        if !metadata.is_file() {
            return Err(format!("Target is not a regular file: {}", input.path));
        }

        let mime = detect_mime_type(&input.path);
        let desc = input.description.unwrap_or_else(|| input.path.clone());

        presented.push(PresentedFileItem {
            path: input.path,
            description: desc,
            size_bytes: metadata.len(),
            mime_type: mime,
        });
    }

    Ok(PresentResult {
        presented_files: presented,
    })
}
