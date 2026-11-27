// image_normalization: 对齐 deepseek-harness `normalization.ts` / `encoding.ts` 的请求前图片归一化。
//
// 职责：
// 1. 探测原始图片事实（格式 / 感知宽高 / 动画 / 元数据 / 8-bit / sRGB / alpha）；
// 2. 干净的 8-bit sRGB 单帧图片在像素与体积预算内原样字节放行（零重编码）；
// 3. 否则统一剥离元数据、应用 EXIF 方向、等比缩放到像素/长边预算内，按 alpha 分流
//    重编码（无 alpha 走 JPEG [85, 75, 60] 质量阶梯；有 alpha 走 PNG 保留透明）；
// 4. 阶梯全部超字节目标时取最小产物而不报错（拒绝权在传输层/validate_turn_images）；
// 5. 二次校验输出满足单帧 8-bit sRGB/sRGBA 且无元数据，并通过 spawn_blocking + 2 路并发闸执行。

use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine;
use image::codecs::gif::GifDecoder;
use image::codecs::jpeg::{JpegDecoder, JpegEncoder};
use image::codecs::png::{CompressionType, FilterType, PngDecoder, PngEncoder};
use image::codecs::webp::WebPDecoder;
use image::metadata::Orientation;
use image::{AnimationDecoder, ColorType, DynamicImage, ImageDecoder, ImageEncoder, ImageFormat};
use std::io::Cursor;
use std::sync::Arc;
use tokio::sync::Semaphore;

use crate::agent::input::TurnInputImage;

pub(super) const DEFAULT_NORMALIZED_IMAGE_MAX_PIXELS: u64 = 2048 * 2048;
pub(super) const DEFAULT_NORMALIZED_IMAGE_MAX_DIMENSION: u32 = 8192;
pub(super) const DEFAULT_NORMALIZED_IMAGE_MAX_BYTES: usize = 4 * 1024 * 1024;
pub(super) const DEFAULT_IMAGE_COMPRESSION_CONCURRENCY: usize = 2;
pub(super) const IMAGE_ENCODING_QUALITIES: [u8; 3] = [85, 75, 60];

#[derive(Debug, Clone, Copy)]
pub(super) struct NormalizationPolicy {
    pub(super) max_pixels: u64,
    pub(super) max_dimension: u32,
    pub(super) target_bytes: usize,
}

impl Default for NormalizationPolicy {
    fn default() -> Self {
        Self {
            max_pixels: DEFAULT_NORMALIZED_IMAGE_MAX_PIXELS,
            max_dimension: DEFAULT_NORMALIZED_IMAGE_MAX_DIMENSION,
            target_bytes: DEFAULT_NORMALIZED_IMAGE_MAX_BYTES,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SampleDepth {
    EightBit,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ImageColorSpace {
    Srgb,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum FrameLayout {
    Single,
    Animated,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum MetadataState {
    Clean,
    CarriesMetadata,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum AlphaPresence {
    Opaque,
    HasAlpha,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct DetectedImage {
    pub(super) media_type: &'static str,
    pub(super) width: u32,
    pub(super) height: u32,
    pub(super) animated: bool,
    pub(super) carries_metadata: bool,
    pub(super) depth: SampleDepth,
    pub(super) space: ImageColorSpace,
    pub(super) has_alpha: bool,
    pub(super) orientation: Orientation,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct NormalizedImage {
    pub(super) data: Vec<u8>,
    pub(super) media_type: &'static str,
    pub(super) width: u32,
    pub(super) height: u32,
    pub(super) passed_through: bool,
}

pub(super) fn can_pass_through_normalization(
    detected: &DetectedImage,
    bytes_len: usize,
    policy: &NormalizationPolicy,
) -> bool {
    detected.media_type != "image/gif"
        && !detected.animated
        && !detected.carries_metadata
        && detected.depth == SampleDepth::EightBit
        && detected.space == ImageColorSpace::Srgb
        && bytes_len <= policy.target_bytes
        && u64::from(detected.width) * u64::from(detected.height) <= policy.max_pixels
        && detected.width.max(detected.height) <= policy.max_dimension
}

fn orientation_swaps_axes(orientation: Orientation) -> bool {
    matches!(
        orientation,
        Orientation::Rotate90
            | Orientation::Rotate270
            | Orientation::Rotate90FlipH
            | Orientation::Rotate270FlipH
    )
}

fn inspect_decoder_metadata<D: ImageDecoder>(
    decoder: &mut D,
) -> Result<(MetadataState, Orientation), String> {
    let has_icc = decoder
        .icc_profile()
        .map_err(|e| format!("failed to inspect icc profile: {e}"))?
        .is_some_and(|v| !v.is_empty());
    let has_exif = decoder
        .exif_metadata()
        .map_err(|e| format!("failed to inspect exif metadata: {e}"))?
        .is_some_and(|v| !v.is_empty());
    let has_xmp = decoder
        .xmp_metadata()
        .map_err(|e| format!("failed to inspect xmp metadata: {e}"))?
        .is_some_and(|v| !v.is_empty());
    let has_iptc = decoder
        .iptc_metadata()
        .map_err(|e| format!("failed to inspect iptc metadata: {e}"))?
        .is_some_and(|v| !v.is_empty());
    let orientation = decoder.orientation().unwrap_or(Orientation::NoTransforms);
    let metadata_state = if has_icc || has_exif || has_xmp || has_iptc || orientation != Orientation::NoTransforms {
        MetadataState::CarriesMetadata
    } else {
        MetadataState::Clean
    };
    Ok((metadata_state, orientation))
}

fn detect_from_parts(
    media_type: &'static str,
    raw_dimensions: (u32, u32),
    color_type: ColorType,
    frame_layout: FrameLayout,
    metadata_state: MetadataState,
    orientation: Orientation,
) -> DetectedImage {
    let (width, height) = if orientation_swaps_axes(orientation) {
        (raw_dimensions.1, raw_dimensions.0)
    } else {
        raw_dimensions
    };
    let depth = match color_type {
        ColorType::L8 | ColorType::La8 | ColorType::Rgb8 | ColorType::Rgba8 => {
            SampleDepth::EightBit
        }
        _ => SampleDepth::Other,
    };
    let space = match color_type {
        ColorType::Rgb8
        | ColorType::Rgba8
        | ColorType::Rgb16
        | ColorType::Rgba16
        | ColorType::Rgb32F
        | ColorType::Rgba32F => ImageColorSpace::Srgb,
        _ => ImageColorSpace::Other,
    };
    DetectedImage {
        media_type,
        width,
        height,
        animated: frame_layout == FrameLayout::Animated,
        carries_metadata: metadata_state == MetadataState::CarriesMetadata,
        depth,
        space,
        has_alpha: color_type.has_alpha(),
        orientation,
    }
}

pub(super) fn detect_image(bytes: &[u8]) -> Result<DetectedImage, String> {
    let format =
        image::guess_format(bytes).map_err(|e| format!("unsupported or malformed image: {e}"))?;
    match format {
        ImageFormat::Png => {
            let mut decoder = PngDecoder::new(Cursor::new(bytes))
                .map_err(|e| format!("failed to decode png header: {e}"))?;
            let dims = decoder.dimensions();
            let color_type = decoder.color_type();
            let frame_layout = if decoder.is_apng().unwrap_or(false) {
                FrameLayout::Animated
            } else {
                FrameLayout::Single
            };
            let (metadata_state, orientation) = inspect_decoder_metadata(&mut decoder)?;
            Ok(detect_from_parts(
                "image/png",
                dims,
                color_type,
                frame_layout,
                metadata_state,
                orientation,
            ))
        }
        ImageFormat::Jpeg => {
            let mut decoder = JpegDecoder::new(Cursor::new(bytes))
                .map_err(|e| format!("failed to decode jpeg header: {e}"))?;
            let dims = decoder.dimensions();
            let color_type = decoder.color_type();
            let (metadata_state, orientation) = inspect_decoder_metadata(&mut decoder)?;
            Ok(detect_from_parts(
                "image/jpeg",
                dims,
                color_type,
                FrameLayout::Single,
                metadata_state,
                orientation,
            ))
        }
        ImageFormat::WebP => {
            let mut decoder = WebPDecoder::new(Cursor::new(bytes))
                .map_err(|e| format!("failed to decode webp header: {e}"))?;
            let dims = decoder.dimensions();
            let color_type = decoder.color_type();
            let frame_layout = if decoder.has_animation() {
                FrameLayout::Animated
            } else {
                FrameLayout::Single
            };
            let (metadata_state, orientation) = inspect_decoder_metadata(&mut decoder)?;
            Ok(detect_from_parts(
                "image/webp",
                dims,
                color_type,
                frame_layout,
                metadata_state,
                orientation,
            ))
        }
        ImageFormat::Gif => {
            let mut decoder = GifDecoder::new(Cursor::new(bytes))
                .map_err(|e| format!("failed to decode gif header: {e}"))?;
            let dims = decoder.dimensions();
            let color_type = decoder.color_type();
            let (metadata_state, orientation) = inspect_decoder_metadata(&mut decoder)?;
            let frame_layout = if decoder.into_frames().take(2).count() > 1 {
                FrameLayout::Animated
            } else {
                FrameLayout::Single
            };
            Ok(detect_from_parts(
                "image/gif",
                dims,
                color_type,
                frame_layout,
                metadata_state,
                orientation,
            ))
        }
        other => Err(format!("unsupported image format: {other:?}")),
    }
}

fn decode_first_frame(bytes: &[u8], media_type: &str) -> Result<DynamicImage, String> {
    match media_type {
        "image/png" => {
            let decoder = PngDecoder::new(Cursor::new(bytes))
                .map_err(|e| format!("failed to decode png: {e}"))?;
            DynamicImage::from_decoder(decoder).map_err(|e| format!("failed to decode png: {e}"))
        }
        "image/jpeg" => {
            let decoder = JpegDecoder::new(Cursor::new(bytes))
                .map_err(|e| format!("failed to decode jpeg: {e}"))?;
            DynamicImage::from_decoder(decoder).map_err(|e| format!("failed to decode jpeg: {e}"))
        }
        "image/webp" => {
            let decoder = WebPDecoder::new(Cursor::new(bytes))
                .map_err(|e| format!("failed to decode webp: {e}"))?;
            DynamicImage::from_decoder(decoder).map_err(|e| format!("failed to decode webp: {e}"))
        }
        "image/gif" => {
            let decoder = GifDecoder::new(Cursor::new(bytes))
                .map_err(|e| format!("failed to decode gif: {e}"))?;
            DynamicImage::from_decoder(decoder).map_err(|e| format!("failed to decode gif: {e}"))
        }
        other => Err(format!("unsupported image media type: {other}")),
    }
}

pub(super) fn initial_dimensions(
    width: u32,
    height: u32,
    policy: &NormalizationPolicy,
) -> (u32, u32) {
    if width == 0 || height == 0 {
        return (width.max(1), height.max(1));
    }
    let total_pixels = u64::from(width) * u64::from(height);
    let (mut w, mut h) = if total_pixels <= policy.max_pixels || policy.max_pixels == 0 {
        (width, height)
    } else {
        let total = u128::from(total_pixels);
        let budget = u128::from(policy.max_pixels);
        let raw_w = ((u128::from(width) * u128::from(width) * budget) / total).isqrt();
        let raw_h = ((u128::from(height) * u128::from(height) * budget) / total).isqrt();
        let mut scaled_w = u32::try_from(raw_w).unwrap_or(u32::MAX).max(1);
        let mut scaled_h = u32::try_from(raw_h).unwrap_or(u32::MAX).max(1);
        while u64::from(scaled_w) * u64::from(scaled_h) > policy.max_pixels {
            if scaled_w >= scaled_h && scaled_w > 1 {
                scaled_w -= 1;
            } else if scaled_h > 1 {
                scaled_h -= 1;
            } else {
                break;
            }
        }
        (scaled_w, scaled_h)
    };

    let long_edge = w.max(h);
    if long_edge > policy.max_dimension && policy.max_dimension > 0 {
        let cap = u64::from(policy.max_dimension);
        let edge = u64::from(long_edge);
        w = u32::try_from((u64::from(w) * cap) / edge)
            .unwrap_or(1)
            .max(1);
        h = u32::try_from((u64::from(h) * cap) / edge)
            .unwrap_or(1)
            .max(1);
    }
    (w, h)
}

fn encode_jpeg_with_ladder(
    image: &DynamicImage,
    max_bytes: usize,
) -> Result<(Vec<u8>, &'static str, u32, u32), String> {
    let rgb = image.to_rgb8();
    let (width, height) = rgb.dimensions();
    let mut smallest: Option<Vec<u8>> = None;

    for quality in IMAGE_ENCODING_QUALITIES {
        let mut buf = Vec::new();
        let encoder = JpegEncoder::new_with_quality(&mut buf, quality);
        encoder
            .write_image(rgb.as_raw(), width, height, ColorType::Rgb8.into())
            .map_err(|e| format!("jpeg encode failed: {e}"))?;
        if buf.len() <= max_bytes {
            return Ok((buf, "image/jpeg", width, height));
        }
        let replace = smallest
            .as_ref()
            .is_none_or(|best| buf.len() < best.len());
        if replace {
            smallest = Some(buf);
        }
    }

    let data = smallest.ok_or_else(|| "jpeg ladder produced no output".to_string())?;
    Ok((data, "image/jpeg", width, height))
}

fn encode_png_rgba(image: &DynamicImage) -> Result<(Vec<u8>, &'static str, u32, u32), String> {
    let rgba = image.to_rgba8();
    let (width, height) = rgba.dimensions();
    let mut buf = Vec::new();
    let encoder = PngEncoder::new_with_quality(&mut buf, CompressionType::Best, FilterType::Adaptive);
    encoder
        .write_image(rgba.as_raw(), width, height, ColorType::Rgba8.into())
        .map_err(|e| format!("png encode failed: {e}"))?;
    Ok((buf, "image/png", width, height))
}

fn verify_normalized_image(
    data: Vec<u8>,
    media_type: &'static str,
    width: u32,
    height: u32,
    expected_alpha: Option<AlphaPresence>,
) -> Result<NormalizedImage, String> {
    let verified = detect_image(&data)?;
    let actual_alpha = if verified.has_alpha {
        AlphaPresence::HasAlpha
    } else {
        AlphaPresence::Opaque
    };
    let alpha_ok = expected_alpha.is_none_or(|expected| actual_alpha == expected);
    if verified.media_type != media_type
        || verified.width != width
        || verified.height != height
        || verified.animated
        || verified.carries_metadata
        || verified.depth != SampleDepth::EightBit
        || verified.space != ImageColorSpace::Srgb
        || !alpha_ok
    {
        return Err(
            "Image normalization did not produce a single-frame 8-bit sRGB image with matching metadata."
                .to_string(),
        );
    }
    Ok(NormalizedImage {
        data,
        media_type,
        width,
        height,
        passed_through: false,
    })
}

pub(super) fn normalize_image_bytes(
    bytes: &[u8],
    policy: &NormalizationPolicy,
) -> Result<NormalizedImage, String> {
    let detected = detect_image(bytes)?;
    if can_pass_through_normalization(&detected, bytes.len(), policy) {
        return Ok(NormalizedImage {
            data: bytes.to_vec(),
            media_type: detected.media_type,
            width: detected.width,
            height: detected.height,
            passed_through: true,
        });
    }

    let mut image = decode_first_frame(bytes, detected.media_type)?;
    if detected.orientation != Orientation::NoTransforms {
        image.apply_orientation(detected.orientation);
    }

    let (target_w, target_h) = initial_dimensions(image.width(), image.height(), policy);
    if image.width() != target_w || image.height() != target_h {
        image = image.resize_exact(target_w, target_h, image::imageops::FilterType::Triangle);
    }

    let (keep_alpha, expected_alpha) = if detected.media_type == "image/gif" {
        let gif_has_transparency = image.to_rgba8().pixels().any(|p| p[3] < 255);
        let alpha_choice = if gif_has_transparency {
            AlphaPresence::HasAlpha
        } else {
            AlphaPresence::Opaque
        };
        (alpha_choice, None)
    } else {
        let alpha_choice = if detected.has_alpha {
            AlphaPresence::HasAlpha
        } else {
            AlphaPresence::Opaque
        };
        (alpha_choice, Some(alpha_choice))
    };

    let (encoded, media_type, out_w, out_h) = if keep_alpha == AlphaPresence::HasAlpha {
        encode_png_rgba(&image)?
    } else {
        encode_jpeg_with_ladder(&image, policy.target_bytes)?
    };

    verify_normalized_image(encoded, media_type, out_w, out_h, expected_alpha)
}

fn normalize_single_turn_image(
    image: TurnInputImage,
    policy: NormalizationPolicy,
) -> TurnInputImage {
    let Some(base64_payload) = image.base64_data() else {
        return image;
    };
    let Ok(decoded_bytes) = BASE64_STANDARD.decode(base64_payload.trim()) else {
        tracing::warn!(
            target: "runtime",
            mime_type = %image.mime_type,
            "[pony-agent] skipping image normalization: invalid base64 payload"
        );
        return image;
    };
    match normalize_image_bytes(&decoded_bytes, &policy) {
        Ok(normalized) if normalized.passed_through => image,
        Ok(normalized) => {
            let encoded = BASE64_STANDARD.encode(&normalized.data);
            TurnInputImage {
                data_url: format!("data:{};base64,{}", normalized.media_type, encoded),
                mime_type: normalized.media_type.to_string(),
                name: image.name,
            }
        }
        Err(error) => {
            tracing::warn!(
                target: "runtime",
                mime_type = %image.mime_type,
                error = %error,
                "[pony-agent] image normalization skipped (fail-soft pass-through)"
            );
            image
        }
    }
}

pub(super) async fn normalize_turn_images_async(
    images: Vec<TurnInputImage>,
    policy: NormalizationPolicy,
) -> Vec<TurnInputImage> {
    if images.is_empty() {
        return images;
    }

    let semaphore = Arc::new(Semaphore::new(DEFAULT_IMAGE_COMPRESSION_CONCURRENCY));
    let mut handles = Vec::with_capacity(images.len());
    for image in images {
        let permit = Arc::clone(&semaphore);
        handles.push(tokio::spawn(async move {
            let _guard = permit.acquire_owned().await.ok();
            let fallback = image.clone();
            tokio::task::spawn_blocking(move || normalize_single_turn_image(image, policy))
                .await
                .unwrap_or(fallback)
        }));
    }

    let mut normalized = Vec::with_capacity(handles.len());
    for handle in handles {
        if let Ok(image) = handle.await {
            normalized.push(image);
        }
    }
    normalized
}

pub(super) fn normalize_turn_images(images: Vec<TurnInputImage>) -> Vec<TurnInputImage> {
    if images.is_empty() {
        return images;
    }
    crate::agent::runtime_helper::block_on(normalize_turn_images_async(
        images,
        NormalizationPolicy::default(),
    ))
}
