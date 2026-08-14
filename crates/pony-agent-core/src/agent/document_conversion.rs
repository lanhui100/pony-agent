//! Workspace-scoped office document conversion (Firecrawl anydoc integration).
//!
//! `workspace_read_document` converts a workspace-scoped office document (Word, PowerPoint,
//! Excel, OpenDocument, RTF, EPUB, CSV, PDF) into GitHub-Flavored Markdown for the model.
//! The module mirrors the safety posture of `image_artifact`:
//!
//! - the path must canonicalize inside the workspace root (fail closed otherwise);
//! - the input file must stay under `max_input_bytes`;
//! - the converted Markdown must stay under `max_output_bytes`, with any cap surfaced as an
//!   explicit `truncated` evidence plus the full length — never a silent cut;
//! - format detection is content-based (`anydoc::Format::from_bytes`), so mislabeled files
//!   still convert; unsupported content fails closed with a diagnostic.
//!
//! The module introduces no external services and no ML models: `anydoc` is a pure-Rust local
//! converter. Scanned PDFs (no embedded text) are outside its scope and are reported as such.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::agent::tool_runtime::{PrimitiveToolHandler, PrimitiveToolHandlerRequest};

/// Default cap for the converted Markdown returned to the model. Larger than the plain-text
/// read budget because document conversion legitimately produces structured output (tables,
/// headings), while still bounding context pressure.
pub const DEFAULT_MAX_OUTPUT_BYTES: u64 = 512 * 1024;
/// Default cap for the source document bytes read from disk. Office files embed fonts, images
/// and history that dwarf their text payload, so this is generous but still bounded.
pub const DEFAULT_MAX_INPUT_BYTES: u64 = 20 * 1024 * 1024;

/// Bounds enforced while converting a workspace document.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentReadOptions {
    /// Maximum converted Markdown bytes accepted without `truncated` evidence.
    pub max_output_bytes: u64,
    /// Maximum source document bytes read from disk. Larger files are rejected up front.
    pub max_input_bytes: u64,
}

impl Default for DocumentReadOptions {
    fn default() -> Self {
        Self {
            max_output_bytes: DEFAULT_MAX_OUTPUT_BYTES,
            max_input_bytes: DEFAULT_MAX_INPUT_BYTES,
        }
    }
}

/// A bounded, workspace-scoped view of a converted document.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DocumentConversion {
    /// Canonical absolute path inside the workspace — the controlled reference.
    pub path: String,
    /// Detected document format (anydoc `Format` name, e.g. `docx`, `pdf`, `csv`).
    pub format: Option<String>,
    /// Converted GitHub-Flavored Markdown, capped at `max_output_bytes`.
    pub markdown: String,
    /// Full converted length in bytes (before capping), so callers can distinguish a bounded
    /// payload from the real size.
    pub markdown_len: u64,
    /// Source document size in bytes.
    pub input_bytes: u64,
    /// Evidence that the converted Markdown exceeded `max_output_bytes` and was capped.
    pub truncated: bool,
}

/// Resolve `path` inside `root` and produce a validated, bounded [`DocumentConversion`].
///
/// Fails closed for empty paths, paths that canonicalize outside `root`, non-file targets,
/// oversized inputs, unknown formats, and conversion errors.
pub fn read_workspace_document(
    path: &str,
    root: &Path,
    options: &DocumentReadOptions,
    authorizations: &crate::agent::path_permission::AuthorizeStore,
) -> Result<DocumentConversion, String> {
    let raw_path = path.trim();
    if raw_path.is_empty() {
        return Err("document path cannot be empty".to_string());
    }
    if options.max_output_bytes == 0 || options.max_input_bytes == 0 {
        return Err("document read byte budgets cannot be zero".to_string());
    }

    let canonical = resolve_inside_workspace(raw_path, root, authorizations)?;
    let metadata = std::fs::metadata(&canonical)
        .map_err(|error| format!("cannot read document metadata for `{raw_path}`: {error}"))?;
    if !metadata.is_file() {
        return Err(format!("document path `{raw_path}` is not a regular file"));
    }
    let input_bytes = metadata.len();
    if input_bytes > options.max_input_bytes {
        return Err(format!(
            "document `{raw_path}` is {input_bytes} bytes, exceeding the {max} byte input limit",
            max = options.max_input_bytes
        ));
    }

    let bytes = std::fs::read(&canonical)
        .map_err(|error| format!("cannot read document `{raw_path}`: {error}"))?;
    // Content-based detection first (mislabeled files still convert). Signature-less formats
    // such as CSV carry no content marker, so fall back to the path extension to name them —
    // exactly how `anydoc` expects callers to handle them.
    let named_format = anydoc::Format::from_bytes(&bytes)
        .or_else(|| anydoc::Format::from_path(&canonical));
    let format = named_format.map(|format| format_name(&format));
    let markdown = anydoc::to_markdown_bytes(&bytes, named_format).map_err(|error| {
        // Distinguish "unknown content" from "known content that failed to parse": scanned PDFs
        // and other image-only documents parse to near-empty output or fail here, which is an
        // explicit limitation (no local OCR), not a silent empty success.
        let hint = match &format {
            Some(name) => format!(
                "conversion of `{raw_path}` ({name}) failed locally: {error}. \
                 Scanned/image-only documents require OCR and are not supported by the local engine"
            ),
            None => format!(
                "unsupported or unrecognized document content in `{raw_path}`: {error}"
            ),
        };
        hint
    })?;

    let markdown_len = markdown.len() as u64;
    let truncated = markdown_len > options.max_output_bytes;
    let markdown = if truncated {
        let cap = options.max_output_bytes as usize;
        if cap == 0 {
            String::new()
        } else {
            // Cut at a char boundary to avoid splitting a UTF-8 sequence.
            let mut cut = cap;
            while cut > 0 && !markdown.is_char_boundary(cut) {
                cut -= 1;
            }
            markdown[..cut].to_string()
        }
    } else {
        markdown
    };

    Ok(DocumentConversion {
        path: canonical.to_string_lossy().into_owned(),
        format,
        markdown,
        markdown_len,
        input_bytes,
        truncated,
    })
}

/// Stable `Format` name for surface output. `anydoc::Format` does not implement `Display`, so
/// the supported variants map explicitly; unknown future variants stay `unknown`.
fn format_name(format: &anydoc::Format) -> String {
    match format {
        anydoc::Format::Doc => "doc".to_string(),
        anydoc::Format::Docx => "docx".to_string(),
        anydoc::Format::Ppt => "ppt".to_string(),
        anydoc::Format::Pptx => "pptx".to_string(),
        anydoc::Format::Excel => "excel".to_string(),
        anydoc::Format::Odt => "odt".to_string(),
        anydoc::Format::Ods => "ods".to_string(),
        anydoc::Format::Odp => "odp".to_string(),
        anydoc::Format::Rtf => "rtf".to_string(),
        anydoc::Format::Epub => "epub".to_string(),
        anydoc::Format::Csv => "csv".to_string(),
        anydoc::Format::Pdf => "pdf".to_string(),
    }
}

/// Canonicalize `raw_path` (absolute or relative to `root`) and fail closed when the result
/// escapes the workspace. PA-080: the check goes through `classify_path(Read)` so an explicit
/// authorization entry can allow an external document read; otherwise it fails closed.
fn resolve_inside_workspace(
    raw_path: &str,
    root: &Path,
    authorizations: &crate::agent::path_permission::AuthorizeStore,
) -> Result<PathBuf, String> {
    use crate::agent::path_permission::{PathPermissionChecker, PathPurpose};
    let input = PathBuf::from(raw_path);
    let candidate = if input.is_absolute() {
        input
    } else {
        root.join(raw_path)
    };
    let canonical = candidate
        .canonicalize()
        .map_err(|error| format!("cannot resolve document path `{raw_path}`: {error}"))?;
    let canonical_root = root
        .canonicalize()
        .map_err(|error| format!("cannot resolve workspace root `{}`: {error}", root.display()))?;
    if !canonical.starts_with(&canonical_root) {
        // workspace 外：授权清单命中放行，否则 `requires_authorization`（审批语义入口）。
        let checker = PathPermissionChecker::with_default();
        let tmp = root.join(".tmp");
        return match checker.classify(
            &canonical.display().to_string(),
            &canonical_root,
            &tmp,
            authorizations,
            PathPurpose::Read,
        ) {
            Ok(permission) => Ok(permission.canonical),
            Err(error) => Err(format!("{}: {}", error.code.as_str(), error.message)),
        };
    }
    Ok(canonical)
}

/// Primitive tool handler adapting [`read_workspace_document`] to the governed dispatcher.
/// The handler parses `path` (required) and optional `maxOutputBytes` from the call arguments,
/// delegating to `read_workspace_document` and returning the serialized
/// [`DocumentConversion`] on success or a structured `code: message` error on failure.
#[derive(Clone, Debug)]
pub struct ReadDocumentHandler {
    root: PathBuf,
    default_options: DocumentReadOptions,
    /// 共享授权清单（PA-080）：workspace 外文档读取需显式授权才放行。
    authorizations: Arc<crate::agent::path_permission::AuthorizeStore>,
}

impl ReadDocumentHandler {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            default_options: DocumentReadOptions::default(),
            authorizations: Arc::new(crate::agent::path_permission::AuthorizeStore::new()),
        }
    }

    /// 注入共享授权清单（PA-080）：与工具执行器共享同一 `Arc`。
    pub fn with_authorizations(
        mut self,
        authorizations: Arc<crate::agent::path_permission::AuthorizeStore>,
    ) -> Self {
        self.authorizations = authorizations;
        self
    }
}

impl PrimitiveToolHandler for ReadDocumentHandler {
    fn execute(
        &self,
        request: &PrimitiveToolHandlerRequest,
    ) -> Result<serde_json::Value, String> {
        let path = request
            .arguments
            .get("path")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .ok_or_else(|| "missing_argument: workspace_read_document 缺少必填参数 `path`".to_string())?;
        let options = DocumentReadOptions {
            max_output_bytes: request
                .arguments
                .get("maxOutputBytes")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(self.default_options.max_output_bytes),
            max_input_bytes: self.default_options.max_input_bytes,
        };
        let conversion = read_workspace_document(path, &self.root, &options, &self.authorizations)?;
        serde_json::to_value(&conversion).map_err(|e| {
            format!("handler_error: workspace_read_document serialization failed: {e}")
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::path_permission::AuthorizeStore;
    use std::fs;
    use std::io::Write;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEMP_SEQ: AtomicU64 = AtomicU64::new(0);

    fn temp_workspace() -> PathBuf {
        let seq = TEMP_SEQ.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "pony-doc-conversion-test-{}-{seq}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("sub")).expect("create test workspace");
        root
    }

    fn csv_bytes() -> Vec<u8> {
        b"name,amount\napple,3\nbanana,7\n".to_vec()
    }

    /// Minimal ZIP package with a `[Content_Types].xml` + `word/document.xml` — enough for
    /// anydoc's content-based detection and docx parsing.
    fn minimal_docx_bytes() -> Vec<u8> {
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let options =
            zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        zip.start_file("[Content_Types].xml", options).unwrap();
        zip.write_all(
            br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
  <Default Extension="xml" ContentType="application/xml"/>
  <Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>
</Types>"#,
        )
        .unwrap();
        zip.start_file("_rels/.rels", options).unwrap();
        zip.write_all(
            br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/>
</Relationships>"#,
        )
        .unwrap();
        zip.start_file("word/document.xml", options).unwrap();
        zip.write_all(
            br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:body>
    <w:p><w:r><w:t>Pony Agent doc test</w:t></w:r></w:p>
    <w:p><w:r><w:t>Hello anydoc</w:t></w:r></w:p>
  </w:body>
</w:document>"#,
        )
        .unwrap();
        zip.finish().unwrap().into_inner()
    }

    #[test]
    fn csv_converts_to_markdown_with_detected_format() {
        let root = temp_workspace();
        let file = root.join("data.csv");
        fs::write(&file, csv_bytes()).expect("write csv");

        let result = read_workspace_document("data.csv", &root, &DocumentReadOptions::default(), &AuthorizeStore::new())
            .expect("csv should convert");
        assert_eq!(result.format.as_deref(), Some("csv"));
        assert!(!result.truncated);
        assert!(result.markdown.contains("apple"), "{}", result.markdown);
        assert!(result.markdown.contains("banana"), "{}", result.markdown);
        assert_eq!(result.input_bytes, csv_bytes().len() as u64);
    }

    #[test]
    fn docx_converts_with_content_based_detection_even_when_mislabeled() {
        let root = temp_workspace();
        let file = root.join("report.dat"); // misleading extension
        fs::write(&file, minimal_docx_bytes()).expect("write docx-as-dat");

        let result = read_workspace_document("report.dat", &root, &DocumentReadOptions::default(), &AuthorizeStore::new())
            .expect("content-based detection should convert mislabeled docx");
        assert_eq!(result.format.as_deref(), Some("docx"));
        assert!(result.markdown.contains("Pony Agent doc test"), "{}", result.markdown);
    }

    #[test]
    fn rejects_paths_outside_workspace() {
        let root = temp_workspace();
        let outside = root.parent().expect("temp root has a parent").join("stray.txt");
        fs::write(&outside, "secret").expect("write outside file");

        let error = read_workspace_document(
            &format!("..\\{}", outside.file_name().unwrap().to_string_lossy()),
            &root,
            &DocumentReadOptions::default(),
            &AuthorizeStore::new(),
        )
        .expect_err("escape must fail closed");
        assert!(
            error.contains("outside the workspace")
                || error.contains("denied")
                || error.contains("requires_authorization"),
            "{error}"
        );
        let absolute = read_workspace_document(
            &outside.to_string_lossy(),
            &root,
            &DocumentReadOptions::default(),
            &AuthorizeStore::new(),
        )
        .expect_err("absolute path outside must fail closed");
        assert!(
            absolute.contains("outside the workspace")
                || absolute.contains("denied")
                || absolute.contains("requires_authorization"),
            "{absolute}"
        );
    }

    #[test]
    fn rejects_missing_and_directory_targets() {
        let root = temp_workspace();
        let missing = read_workspace_document("nope.docx", &root, &DocumentReadOptions::default(), &AuthorizeStore::new())
            .expect_err("missing file must fail");
        assert!(missing.contains("cannot resolve"), "{missing}");
        let directory = read_workspace_document("sub", &root, &DocumentReadOptions::default(), &AuthorizeStore::new())
            .expect_err("directory must fail");
        assert!(directory.contains("not a regular file"), "{directory}");
    }

    #[test]
    fn rejects_oversized_input_before_conversion() {
        let root = temp_workspace();
        let file = root.join("big.csv");
        fs::write(&file, csv_bytes()).expect("write csv");
        let options = DocumentReadOptions {
            max_input_bytes: 4,
            ..Default::default()
        };
        let error = read_workspace_document("big.csv", &root, &options, &AuthorizeStore::new())
            .expect_err("oversized input must be rejected");
        assert!(error.contains("input limit"), "{error}");
    }

    #[test]
    fn truncates_output_with_evidence() {
        let root = temp_workspace();
        let file = root.join("data.csv");
        fs::write(&file, csv_bytes()).expect("write csv");
        let options = DocumentReadOptions {
            max_output_bytes: 10,
            ..Default::default()
        };
        let result = read_workspace_document("data.csv", &root, &options, &AuthorizeStore::new())
            .expect("conversion with tiny budget still succeeds");
        assert!(result.truncated);
        assert!(result.markdown_len > 10);
        assert!(result.markdown.len() <= 10);
        assert!(result.markdown.is_char_boundary(result.markdown.len()));
    }

    #[test]
    fn rejects_unsupported_content_with_diagnostic() {
        let root = temp_workspace();
        let file = root.join("junk.bin");
        fs::write(&file, b"not a document at all").expect("write junk");
        let error = read_workspace_document("junk.bin", &root, &DocumentReadOptions::default(), &AuthorizeStore::new())
            .expect_err("unknown content must fail");
        assert!(error.contains("unsupported"), "{error}");
    }

    #[test]
    fn handler_parses_arguments_and_returns_structured_json() {
        let root = temp_workspace();
        let file = root.join("data.csv");
        fs::write(&file, csv_bytes()).expect("write csv");
        let handler = ReadDocumentHandler::new(root.clone());
        let request = PrimitiveToolHandlerRequest {
            descriptor_id: "builtin:workspace_read_document".to_string(),
            arguments: serde_json::json!({ "path": "data.csv" }),
            session_id: None,
            workspace_root: None,
        };
        let value = handler.execute(&request).expect("handler should convert");
        assert_eq!(value["format"], serde_json::json!("csv"));
        assert_eq!(value["truncated"], serde_json::json!(false));
        assert!(value["markdown"].as_str().unwrap().contains("apple"));

        let missing = PrimitiveToolHandlerRequest {
            descriptor_id: "builtin:workspace_read_document".to_string(),
            arguments: serde_json::json!({}),
            session_id: None,
            workspace_root: None,
        };
        let error = handler.execute(&missing).expect_err("missing path must fail");
        assert!(error.contains("missing_argument"), "{error}");
    }
}
