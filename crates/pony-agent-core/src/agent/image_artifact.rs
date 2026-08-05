//! Workspace-scoped image reading (phase-7 task 7.1).
//!
//! `view_image` reads an image path that must resolve inside the workspace and returns a
//! **reference-based** [`ImageArtifact`]: the controlled reference is the canonical on-disk
//! path, and raw bytes are carried only as an optional, capped payload. The module never
//! decodes pixels and introduces no image-crate dependency; it validates the extension and
//! MIME from magic bytes, parses header dimensions for common formats, enforces metadata
//! limits (max width/height and max bytes), and surfaces any truncation as explicit
//! `truncated` evidence instead of silently reading the full file.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::agent::tool_runtime::{PrimitiveToolHandler, PrimitiveToolHandlerRequest};

/// Bounded metadata read used for header sniffing and dimension parsing. Kept independent of
/// `max_bytes` so a tiny byte budget cannot make an otherwise valid image unreadable.
const HEADER_SNIFF_BYTES: usize = 8 * 1024;

/// Bounds enforced while viewing a workspace image.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageReadOptions {
    /// Maximum image width accepted without `truncated` evidence.
    pub max_width: u64,
    /// Maximum image height accepted without `truncated` evidence.
    pub max_height: u64,
    /// Maximum on-disk bytes the artifact will carry. Files larger than this are read up to
    /// the cap and marked `truncated`.
    pub max_bytes: u64,
    /// When `false` the artifact carries only the controlled reference plus metadata and never
    /// embeds file bytes; the caller (e.g. a host adapter) decides when to encode them.
    pub include_bytes: bool,
}

impl Default for ImageReadOptions {
    fn default() -> Self {
        Self {
            max_width: 8_192,
            max_height: 8_192,
            max_bytes: 2 * 1024 * 1024,
            include_bytes: false,
        }
    }
}

/// A bounded, reference-based view of a workspace image.
///
/// `bytes` is `Some` only when the image is within the dimension limits *and* the caller
/// requested bytes; its length is capped at `max_bytes`. `bytes_len` is always the full
/// on-disk length so callers can distinguish a bounded payload from the real size.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ImageArtifact {
    /// Canonical absolute path inside the workspace — the controlled reference.
    pub path: String,
    /// Optional capped raw bytes (decoding is left to the caller/provider adapter).
    pub bytes: Option<Vec<u8>>,
    pub width: u64,
    pub height: u64,
    pub mime_type: String,
    /// Full on-disk byte length.
    pub bytes_len: u64,
    /// Evidence that this artifact is a bounded view: the image exceeds `max_width`/`max_height`
    /// (metadata-only, no bytes) or the file exceeds `max_bytes` (payload capped).
    pub truncated: bool,
}

/// Resolve `path` inside `root` and produce a validated, bounded [`ImageArtifact`].
///
/// Fails closed for paths that canonicalize outside `root`, for non-image extensions or
/// MIME mismatches, for malformed headers, and for missing/non-file targets.
pub fn view_workspace_image(
    path: &str,
    root: &Path,
    options: &ImageReadOptions,
) -> Result<ImageArtifact, String> {
    let raw_path = path.trim();
    if raw_path.is_empty() {
        return Err("image path cannot be empty".to_string());
    }
    if options.max_bytes == 0 {
        return Err("image read byte budget cannot be zero".to_string());
    }

    let canonical = resolve_inside_workspace(raw_path, root)?;
    let metadata = std::fs::metadata(&canonical)
        .map_err(|error| format!("cannot read image metadata for `{raw_path}`: {error}"))?;
    if !metadata.is_file() {
        return Err(format!("image path `{raw_path}` is not a regular file"));
    }
    let bytes_len = metadata.len();
    let declared_mime = mime_type_for_path(&canonical)?;

    let header = read_bounded(&canonical, HEADER_SNIFF_BYTES)?;
    let (sniffed_mime, width, height) = sniff_image(&header)?;
    if sniffed_mime != declared_mime {
        return Err(format!(
            "image path `{raw_path}` declares `{declared_mime}` but its content is `{sniffed_mime}`"
        ));
    }

    let dimension_truncated = width > options.max_width || height > options.max_height;
    let byte_truncated = bytes_len > options.max_bytes;
    let truncated = dimension_truncated || byte_truncated;

    // Oversized-by-dimension images are metadata-only: there is no value in carrying raw bytes
    // for a canvas the model/provider must not receive.
    let bytes = if dimension_truncated {
        None
    } else if options.include_bytes {
        Some(read_bounded(&canonical, options.max_bytes as usize)?)
    } else {
        None
    };

    Ok(ImageArtifact {
        path: canonical.to_string_lossy().into_owned(),
        bytes,
        width,
        height,
        mime_type: declared_mime.to_string(),
        bytes_len,
        truncated,
    })
}

/// Canonicalize `raw_path` (absolute or relative to `root`) and fail closed when the result
/// escapes the workspace.
fn resolve_inside_workspace(raw_path: &str, root: &Path) -> Result<PathBuf, String> {
    let input = PathBuf::from(raw_path);
    let candidate = if input.is_absolute() {
        input
    } else {
        root.join(raw_path)
    };
    let canonical = candidate
        .canonicalize()
        .map_err(|error| format!("cannot resolve image path `{raw_path}`: {error}"))?;
    let canonical_root = root
        .canonicalize()
        .map_err(|error| format!("cannot resolve workspace root `{}`: {error}", root.display()))?;
    if !canonical.starts_with(&canonical_root) {
        return Err(format!(
            "image path `{raw_path}` resolves outside the workspace and is denied"
        ));
    }
    Ok(canonical)
}

fn mime_type_for_path(path: &Path) -> Result<&'static str, String> {
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    match extension.as_str() {
        "png" => Ok("image/png"),
        "jpg" | "jpeg" => Ok("image/jpeg"),
        "gif" => Ok("image/gif"),
        "webp" => Ok("image/webp"),
        "bmp" => Ok("image/bmp"),
        _ => Err(format!(
            "unsupported image extension `{extension}` for `{}`",
            path.display()
        )),
    }
}

fn read_bounded(path: &Path, limit: usize) -> Result<Vec<u8>, String> {
    use std::io::Read;
    let file = std::fs::File::open(path)
        .map_err(|error| format!("cannot open image `{}`: {error}", path.display()))?;
    let mut reader = file.take(limit as u64);
    let mut buffer = Vec::with_capacity(limit);
    reader
        .read_to_end(&mut buffer)
        .map_err(|error| format!("cannot read image `{}`: {error}", path.display()))?;
    Ok(buffer)
}

/// Sniff the format from magic bytes and parse header dimensions without decoding pixels.
fn sniff_image(header: &[u8]) -> Result<(&'static str, u64, u64), String> {
    if header.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]) {
        return png_dimensions(header).map(|(width, height)| ("image/png", width, height));
    }
    if header.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return jpeg_dimensions(header).map(|(width, height)| ("image/jpeg", width, height));
    }
    if header.starts_with(b"GIF8") {
        return gif_dimensions(header).map(|(width, height)| ("image/gif", width, height));
    }
    if header.starts_with(b"RIFF") && header.len() >= 12 && &header[8..12] == b"WEBP" {
        return webp_dimensions(header).map(|(width, height)| ("image/webp", width, height));
    }
    if header.starts_with(b"BM") {
        return bmp_dimensions(header).map(|(width, height)| ("image/bmp", width, height));
    }
    Err("file content is not a supported image (unknown magic bytes)".to_string())
}

fn png_dimensions(header: &[u8]) -> Result<(u64, u64), String> {
    if header.len() < 24 || &header[12..16] != b"IHDR" {
        return Err("malformed PNG header (missing IHDR)".to_string());
    }
    let width = u32::from_be_bytes([header[16], header[17], header[18], header[19]]) as u64;
    let height = u32::from_be_bytes([header[20], header[21], header[22], header[23]]) as u64;
    if width == 0 || height == 0 {
        return Err("malformed PNG header (zero dimension)".to_string());
    }
    Ok((width, height))
}

fn jpeg_dimensions(header: &[u8]) -> Result<(u64, u64), String> {
    let mut position = 2usize;
    while position + 3 < header.len() {
        if header[position] != 0xFF {
            position += 1;
            continue;
        }
        let mut marker_index = position + 1;
        while marker_index < header.len() && header[marker_index] == 0xFF {
            marker_index += 1;
        }
        if marker_index >= header.len() {
            break;
        }
        let marker = header[marker_index];
        marker_index += 1;

        // SOF0..SOF15 define the frame dimensions (DHT=0xC4, JPG=0xC8, DAC=0xCC do not).
        if (0xC0..=0xCF).contains(&marker) && !matches!(marker, 0xC4 | 0xC8 | 0xCC) {
            if marker_index + 7 > header.len() {
                return Err("malformed JPEG header (truncated SOF segment)".to_string());
            }
            let height =
                u16::from_be_bytes([header[marker_index + 3], header[marker_index + 4]]) as u64;
            let width =
                u16::from_be_bytes([header[marker_index + 5], header[marker_index + 6]]) as u64;
            if width == 0 || height == 0 {
                return Err("malformed JPEG header (zero dimension)".to_string());
            }
            return Ok((width, height));
        }

        // End of image or start of scan: no more frame headers to find.
        if marker == 0xD9 || marker == 0xDA {
            break;
        }
        // Standalone markers carry no length field.
        if (0xD0..=0xD7).contains(&marker) || marker == 0x01 {
            position = marker_index;
            continue;
        }
        if marker_index + 2 > header.len() {
            break;
        }
        let segment_length =
            u16::from_be_bytes([header[marker_index], header[marker_index + 1]]) as usize;
        // `segment_length` counts from the length field onward (it includes the two length
        // bytes but not the marker byte), so the next marker starts at `marker_index + length`.
        position = marker_index + segment_length;
    }
    Err("malformed JPEG header (SOF marker not found)".to_string())
}

fn gif_dimensions(header: &[u8]) -> Result<(u64, u64), String> {
    if header.len() < 10 {
        return Err("malformed GIF header (truncated)".to_string());
    }
    let width = u16::from_le_bytes([header[6], header[7]]) as u64;
    let height = u16::from_le_bytes([header[8], header[9]]) as u64;
    if width == 0 || height == 0 {
        return Err("malformed GIF header (zero dimension)".to_string());
    }
    Ok((width, height))
}

fn webp_dimensions(header: &[u8]) -> Result<(u64, u64), String> {
    if header.len() < 30 {
        return Err("malformed WebP header (truncated)".to_string());
    }
    match &header[12..16] {
        b"VP8X" => {
            let width = 1 + u32::from_le_bytes([header[24], header[25], header[26], 0]) as u64;
            let height = 1 + u32::from_le_bytes([header[27], header[28], header[29], 0]) as u64;
            if width == 0 || height == 0 {
                return Err("malformed WebP VP8X header (zero dimension)".to_string());
            }
            Ok((width, height))
        }
        b"VP8L" => {
            if header[20] != 0x2F {
                return Err("malformed WebP lossless header (missing signature)".to_string());
            }
            let bits = u32::from_le_bytes([header[21], header[22], header[23], header[24]]);
            let width = (bits & 0x3FFF) as u64 + 1;
            let height = ((bits >> 14) & 0x3FFF) as u64 + 1;
            Ok((width, height))
        }
        b"VP8 " => {
            if header[20] & 0x01 != 0 {
                return Err(
                    "cannot determine dimensions for a non-keyframe WebP lossy image".to_string()
                );
            }
            if &header[23..26] != &[0x9D, 0x01, 0x2A] {
                return Err("malformed WebP lossy header (missing sync code)".to_string());
            }
            let width = (u16::from_le_bytes([header[26], header[27]]) & 0x3FFF) as u64;
            let height = (u16::from_le_bytes([header[28], header[29]]) & 0x3FFF) as u64;
            if width == 0 || height == 0 {
                return Err("malformed WebP lossy header (zero dimension)".to_string());
            }
            Ok((width, height))
        }
        _ => Err("malformed WebP header (unknown chunk type)".to_string()),
    }
}

fn bmp_dimensions(header: &[u8]) -> Result<(u64, u64), String> {
    if header.len() < 26 {
        return Err("malformed BMP header (truncated)".to_string());
    }
    let width = u32::from_le_bytes([header[18], header[19], header[20], header[21]]) as i64;
    let height = u32::from_le_bytes([header[22], header[23], header[24], header[25]]) as i64;
    if width <= 0 || height == 0 {
        return Err("malformed BMP header (invalid dimension)".to_string());
    }
    Ok((width as u64, height.unsigned_abs()))
}

/// Primitive tool handler adapting [`view_workspace_image`] to the governed dispatcher (PA-076 P2-6).
/// The handler parses `path` (required) and optional `includeBytes`/`maxWidth`/`maxHeight`/`maxBytes`
/// overrides from the call arguments and delegates to `view_workspace_image`, returning the serialized
/// [`ImageArtifact`] on success or a structured `code: message` error on failure.
#[derive(Clone, Debug)]
pub struct ViewImageHandler {
    root: std::path::PathBuf,
    default_options: ImageReadOptions,
}

impl ViewImageHandler {
    pub fn new(root: std::path::PathBuf) -> Self {
        Self {
            root,
            default_options: ImageReadOptions::default(),
        }
    }
}

impl PrimitiveToolHandler for ViewImageHandler {
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
            .ok_or_else(|| "missing_argument: view_image 缺少必填参数 `path`".to_string())?;
        let options = ImageReadOptions {
            include_bytes: request
                .arguments
                .get("includeBytes")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(self.default_options.include_bytes),
            max_width: request
                .arguments
                .get("maxWidth")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(self.default_options.max_width),
            max_height: request
                .arguments
                .get("maxHeight")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(self.default_options.max_height),
            max_bytes: request
                .arguments
                .get("maxBytes")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(self.default_options.max_bytes),
        };
        let artifact = view_workspace_image(path, &self.root, &options)?;
        serde_json::to_value(&artifact)
            .map_err(|e| format!("handler_error: view_image artifact serialization failed: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TempWorkspace {
        root: PathBuf,
    }

    impl TempWorkspace {
        fn new(label: &str) -> Self {
            let root = std::env::temp_dir().join(format!(
                "pony-agent-image-artifact-{label}-{}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(&root).expect("temp workspace should create");
            Self { root }
        }

        fn path(&self) -> &Path {
            &self.root
        }

        fn write(&self, name: &str, bytes: &[u8]) -> PathBuf {
            let path = self.root.join(name);
            std::fs::write(&path, bytes).expect("fixture file should write");
            path
        }
    }

    impl Drop for TempWorkspace {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    fn write_png(path: &Path, width: u32, height: u32) {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]);
        bytes.extend_from_slice(&[0, 0, 0, 13]);
        bytes.extend_from_slice(b"IHDR");
        bytes.extend_from_slice(&width.to_be_bytes());
        bytes.extend_from_slice(&height.to_be_bytes());
        bytes.extend_from_slice(&[8, 6, 0, 0, 0]);
        bytes.extend_from_slice(&[0, 0, 0, 0]); // CRC placeholder (parser does not validate)
        bytes.extend_from_slice(&[0, 0, 0, 0]);
        bytes.extend_from_slice(b"IEND");
        std::fs::write(path, bytes).expect("png fixture should write");
    }

    fn write_jpeg(path: &Path, width: u16, height: u16) {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&[0xFF, 0xD8, 0xFF, 0xE0]);
        bytes.extend_from_slice(&[0x00, 0x10]);
        bytes.extend_from_slice(b"JFIF\x00");
        bytes.extend_from_slice(&[0x01, 0x01, 0x00, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00]);
        bytes.extend_from_slice(&[0xFF, 0xC0, 0x00, 0x11, 0x08]);
        bytes.extend_from_slice(&height.to_be_bytes());
        bytes.extend_from_slice(&width.to_be_bytes());
        bytes.extend_from_slice(&[0x03, 0x01, 0x11, 0x00, 0x02, 0x11, 0x01, 0x03, 0x11, 0x01]);
        bytes.extend_from_slice(&[0xFF, 0xD9]);
        std::fs::write(path, bytes).expect("jpeg fixture should write");
    }

    fn write_gif(path: &Path, width: u16, height: u16) {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"GIF89a");
        bytes.extend_from_slice(&width.to_le_bytes());
        bytes.extend_from_slice(&height.to_le_bytes());
        bytes.extend_from_slice(&[0x00, 0x00, 0x00]);
        std::fs::write(path, bytes).expect("gif fixture should write");
    }

    fn write_bmp(path: &Path, width: u32, height: u32) {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"BM");
        bytes.extend_from_slice(&[0, 0, 0, 0]);
        bytes.extend_from_slice(&[0, 0, 0, 0]);
        bytes.extend_from_slice(&[54, 0, 0, 0]);
        bytes.extend_from_slice(&[40, 0, 0, 0]);
        bytes.extend_from_slice(&width.to_le_bytes());
        bytes.extend_from_slice(&height.to_le_bytes());
        bytes.extend_from_slice(&[1, 0]);
        bytes.extend_from_slice(&[24, 0]);
        std::fs::write(path, bytes).expect("bmp fixture should write");
    }

    fn write_webp(path: &Path, width: u32, height: u32) {
        // Minimal VP8X WebP: RIFF + WEBP + VP8X chunk with canvas size.
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&[0x20, 0, 0, 0]);
        bytes.extend_from_slice(b"WEBP");
        bytes.extend_from_slice(b"VP8X");
        bytes.extend_from_slice(&[10, 0, 0, 0]);
        bytes.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]); // flags + reserved
        let width_enc = [(width - 1) as u8, ((width - 1) >> 8) as u8, ((width - 1) >> 16) as u8];
        let height_enc = [
            (height - 1) as u8,
            ((height - 1) >> 8) as u8,
            ((height - 1) >> 16) as u8,
        ];
        bytes.extend_from_slice(&width_enc);
        bytes.extend_from_slice(&height_enc);
        std::fs::write(path, bytes).expect("webp fixture should write");
    }

    #[test]
    fn valid_png_returns_artifact_with_size_and_mime() {
        let workspace = TempWorkspace::new("png");
        let path = workspace.write("photo.png", &[]);
        write_png(&path, 320, 240);

        // Default is reference-based: include_bytes=false (design Decision 11).
        let artifact =
            view_workspace_image("photo.png", workspace.path(), &ImageReadOptions::default())
                .expect("workspace png should be viewable");
        assert_eq!(artifact.width, 320);
        assert_eq!(artifact.height, 240);
        assert_eq!(artifact.mime_type, "image/png");
        assert_eq!(artifact.bytes_len, std::fs::metadata(&path).unwrap().len());
        assert!(!artifact.truncated);
        assert!(
            artifact.bytes.is_none(),
            "default options produce a reference-based artifact (include_bytes=false)"
        );

        // With include_bytes=true, the bytes are present and truncated only when limits exceeded.
        let artifact = view_workspace_image(
            "photo.png",
            workspace.path(),
            &ImageReadOptions {
                include_bytes: true,
                ..ImageReadOptions::default()
            },
        )
        .expect("workspace png with bytes should be viewable");
        assert!(artifact.bytes.is_some());
        let bytes = artifact.bytes.expect("include_bytes=true embeds bytes");
        assert!(bytes.starts_with(&[0x89, b'P', b'N', b'G']));
    }

    #[test]
    fn valid_jpeg_gif_bmp_and_webp_are_supported() {
        let workspace = TempWorkspace::new("formats");
        let jpeg = workspace.write("photo.jpg", &[]);
        write_jpeg(&jpeg, 640, 480);
        let gif = workspace.write("anim.gif", &[]);
        write_gif(&gif, 12, 34);
        let bmp = workspace.write("raw.bmp", &[]);
        write_bmp(&bmp, 56, 78);
        let webp = workspace.write("modern.webp", &[]);
        write_webp(&webp, 90, 120);

        let options = ImageReadOptions::default();
        let jpeg_artifact =
            view_workspace_image("photo.jpg", workspace.path(), &options).expect("jpeg viewable");
        assert_eq!((jpeg_artifact.width, jpeg_artifact.height), (640, 480));
        assert_eq!(jpeg_artifact.mime_type, "image/jpeg");

        let gif_artifact =
            view_workspace_image("anim.gif", workspace.path(), &options).expect("gif viewable");
        assert_eq!((gif_artifact.width, gif_artifact.height), (12, 34));
        assert_eq!(gif_artifact.mime_type, "image/gif");

        let bmp_artifact =
            view_workspace_image("raw.bmp", workspace.path(), &options).expect("bmp viewable");
        assert_eq!((bmp_artifact.width, bmp_artifact.height), (56, 78));
        assert_eq!(bmp_artifact.mime_type, "image/bmp");

        let webp_artifact =
            view_workspace_image("modern.webp", workspace.path(), &options).expect("webp viewable");
        assert_eq!((webp_artifact.width, webp_artifact.height), (90, 120));
        assert_eq!(webp_artifact.mime_type, "image/webp");
    }

    #[test]
    fn outside_workspace_paths_are_denied() {
        let parent = std::env::temp_dir().join(format!(
            "pony-agent-image-artifact-outside-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&parent);
        let workspace_dir = parent.join("workspace");
        std::fs::create_dir_all(&workspace_dir).expect("workspace dir should create");
        let escape = parent.join("escape.png");
        write_png(&escape, 10, 10);
        let options = ImageReadOptions::default();

        let absolute_error = view_workspace_image(
            &escape.to_string_lossy(),
            &workspace_dir,
            &options,
        )
        .expect_err("absolute path outside the workspace must be denied");
        assert!(absolute_error.contains("outside the workspace"), "{absolute_error}");

        let relative_error =
            view_workspace_image("../escape.png", &workspace_dir, &options)
                .expect_err("relative escape outside the workspace must be denied");
        assert!(relative_error.contains("outside the workspace"), "{relative_error}");

        let _ = std::fs::remove_dir_all(&parent);
    }

    #[test]
    fn over_bytes_marks_truncated_with_capped_payload() {
        let workspace = TempWorkspace::new("overbytes");
        let path = workspace.write("big.png", &[]);
        write_png(&path, 64, 64);
        let full_len = std::fs::metadata(&path).unwrap().len();
        assert!(full_len > 16);

        let options = ImageReadOptions {
            max_bytes: 16,
            include_bytes: true,
            ..ImageReadOptions::default()
        };
        let artifact =
            view_workspace_image("big.png", workspace.path(), &options).expect("read should succeed");
        assert!(artifact.truncated, "byte overflow must surface truncated evidence");
        assert_eq!(artifact.bytes_len, full_len);
        let bytes = artifact.bytes.expect("capped payload should be present when include_bytes is true");
        assert_eq!(bytes.len(), 16);
    }

    #[test]
    fn oversized_dimensions_mark_truncated_without_payload() {
        let workspace = TempWorkspace::new("oversize");
        workspace.write("huge.png", &[]);
        write_png(&workspace.path().join("huge.png"), 100_000, 100_000);

        let options = ImageReadOptions::default();
        let artifact =
            view_workspace_image("huge.png", workspace.path(), &options).expect("read should succeed");
        assert!(artifact.truncated, "dimension overflow must surface truncated evidence");
        assert_eq!(artifact.width, 100_000);
        assert_eq!(artifact.bytes, None, "oversized images must not embed bytes");
    }

    #[test]
    fn reference_only_mode_omits_bytes() {
        let workspace = TempWorkspace::new("reference");
        workspace.write("thumb.png", &[]);
        write_png(&workspace.path().join("thumb.png"), 8, 8);

        let options = ImageReadOptions {
            include_bytes: false,
            ..ImageReadOptions::default()
        };
        let artifact =
            view_workspace_image("thumb.png", workspace.path(), &options).expect("read should succeed");
        assert!(!artifact.truncated);
        assert_eq!(artifact.bytes, None);
        assert_eq!(artifact.width, 8);
    }

    #[test]
    fn non_image_extension_is_rejected() {
        let workspace = TempWorkspace::new("ext");
        let path = workspace.write("notes.txt", &[]);
        write_png(&path, 4, 4);

        let error =
            view_workspace_image("notes.txt", workspace.path(), &ImageReadOptions::default())
                .expect_err("non-image extension must be rejected");
        assert!(error.contains("unsupported image extension"), "{error}");
    }

    #[test]
    fn mime_mismatch_is_rejected() {
        let workspace = TempWorkspace::new("mime");
        let path = workspace.write("actually_jpeg.png", &[]);
        write_jpeg(&path, 16, 16);

        let error =
            view_workspace_image("actually_jpeg.png", workspace.path(), &ImageReadOptions::default())
                .expect_err("declared extension must match content MIME");
        assert!(error.contains("declares `image/png`"), "{error}");
    }

    #[test]
    fn missing_file_errors() {
        let workspace = TempWorkspace::new("missing");
        let error =
            view_workspace_image("nope.png", workspace.path(), &ImageReadOptions::default())
                .expect_err("missing file must error");
        assert!(error.contains("cannot resolve image path"), "{error}");
    }

    #[test]
    fn directory_and_empty_file_are_rejected() {
        let workspace = TempWorkspace::new("bad");
        std::fs::create_dir(workspace.path().join("folder")).expect("subdir should create");

        let dir_error =
            view_workspace_image("folder", workspace.path(), &ImageReadOptions::default())
                .expect_err("directory must be rejected");
        assert!(dir_error.contains("not a regular file"), "{dir_error}");

        workspace.write("empty.png", &[]);
        let empty_error =
            view_workspace_image("empty.png", workspace.path(), &ImageReadOptions::default())
                .expect_err("empty file must be rejected");
        assert!(empty_error.contains("not a supported image"), "{empty_error}");
    }

    #[test]
    fn include_bytes_default_is_false_and_reference_only() {
        // design Decision 11: default artifact carries only the controlled reference + metadata;
        // the host/provider adapter decides when to encode bytes.
        let workspace = TempWorkspace::new("include_bytes_default");
        write_png(&workspace.path().join("photo.png"), 8, 8);

        let artifact =
            view_workspace_image("photo.png", workspace.path(), &ImageReadOptions::default())
                .expect("reference-based default must succeed");
        assert!(!artifact.truncated);
        assert!(artifact.bytes.is_none());
        assert_eq!(artifact.width, 8);
        assert_eq!(artifact.mime_type, "image/png");
    }

    // ── ViewImageHandler ────────────────────────────────────────────────────────────────────

    fn handler_request(arguments: serde_json::Value) -> crate::agent::tool_runtime::PrimitiveToolHandlerRequest {
        crate::agent::tool_runtime::PrimitiveToolHandlerRequest {
            descriptor_id: "builtin:view_image".to_string(),
            arguments,
            session_id: None,
        }
    }

    #[test]
    fn handler_returns_reference_artifact_with_default_options() {
        let workspace = TempWorkspace::new("handler_ref");
        write_png(&workspace.path().join("photo.png"), 100, 50);

        let handler = ViewImageHandler::new(workspace.path().to_path_buf());
        let output = handler
            .execute(&handler_request(serde_json::json!({ "path": "photo.png" })))
            .expect("handler should succeed");
        assert_eq!(output["width"], 100);
        assert_eq!(output["height"], 50);
        assert_eq!(output["mimeType"], "image/png");
        assert_eq!(output["bytes"], serde_json::Value::Null);
        assert_eq!(output["truncated"], false);
    }

    #[test]
    fn handler_with_include_bytes_true_embeds_payload() {
        let workspace = TempWorkspace::new("handler_bytes");
        write_png(&workspace.path().join("photo.png"), 16, 16);

        let handler = ViewImageHandler::new(workspace.path().to_path_buf());
        let output = handler
            .execute(&handler_request(serde_json::json!({
                "path": "photo.png",
                "includeBytes": true
            })))
            .expect("handler should succeed");
        assert!(output["bytes"].is_array() || output["bytes"].is_string());
        assert!(!output["bytes"].is_null());
    }

    #[test]
    fn handler_max_bytes_override_caps_and_marks_truncated() {
        let workspace = TempWorkspace::new("handler_cap");
        write_png(&workspace.path().join("photo.png"), 64, 64);
        let full_len = std::fs::metadata(&workspace.path().join("photo.png"))
            .unwrap()
            .len();
        assert!(full_len > 16);

        let handler = ViewImageHandler::new(workspace.path().to_path_buf());
        let output = handler
            .execute(&handler_request(serde_json::json!({
                "path": "photo.png",
                "includeBytes": true,
                "maxBytes": 16
            })))
            .expect("handler should succeed with capped bytes");
        assert_eq!(output["truncated"], true);
        assert_eq!(output["bytesLen"], full_len);
        let bytes = output["bytes"].as_array().map(Vec::len).unwrap_or(0);
        assert_eq!(bytes, 16);
    }

    #[test]
    fn handler_missing_path_is_missing_argument_error() {
        let workspace = TempWorkspace::new("handler_missing");
        let handler = ViewImageHandler::new(workspace.path().to_path_buf());
        let error = handler
            .execute(&handler_request(serde_json::json!({})))
            .expect_err("missing path must fail closed");
        assert!(error.starts_with("missing_argument:"), "{error}");
    }

    #[test]
    fn handler_unknown_format_rejects_with_mime_error() {
        let workspace = TempWorkspace::new("handler_format");
        std::fs::write(workspace.path().join("notes.txt"), b"not an image\n")
            .expect("write fixture");

        let handler = ViewImageHandler::new(workspace.path().to_path_buf());
        let error = handler
            .execute(&handler_request(serde_json::json!({ "path": "notes.txt" })))
            .expect_err("non-image extension must fail closed");
        assert!(error.contains("unsupported image extension"), "{error}");
    }
}
