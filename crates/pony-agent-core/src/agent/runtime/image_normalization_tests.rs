use super::image_normalization::{
    detect_image, normalize_image_bytes, normalize_turn_images, NormalizationPolicy,
};
use super::trace_timeline::validate_turn_images;
use crate::agent::input::TurnInputImage;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine;
use image::codecs::gif::{GifEncoder, Repeat};
use image::codecs::jpeg::JpegEncoder;
use image::codecs::png::PngEncoder;
use image::{Delay, Frame, ImageEncoder, RgbImage, Rgba, RgbaImage};
use std::io::Cursor;

fn encode_clean_rgb_png(width: u32, height: u32) -> Vec<u8> {
    let mut img = RgbImage::new(width, height);
    for (x, y, pixel) in img.enumerate_pixels_mut() {
        *pixel = image::Rgb([
            ((x * 17 + y * 31) % 256) as u8,
            ((x * 43 + y * 13) % 256) as u8,
            ((x * 7 + y * 59) % 256) as u8,
        ]);
    }
    let mut out = Vec::new();
    PngEncoder::new(&mut out)
        .write_image(img.as_raw(), width, height, image::ColorType::Rgb8.into())
        .expect("encode clean rgb png");
    out
}

fn encode_rgba_png_with_alpha(width: u32, height: u32) -> Vec<u8> {
    let mut img = RgbaImage::new(width, height);
    for (x, y, pixel) in img.enumerate_pixels_mut() {
        let alpha = if (x + y) % 2 == 0 { 128 } else { 255 };
        *pixel = Rgba([(x % 256) as u8, (y % 256) as u8, 200, alpha]);
    }
    let mut out = Vec::new();
    PngEncoder::new(&mut out)
        .write_image(img.as_raw(), width, height, image::ColorType::Rgba8.into())
        .expect("encode rgba png");
    out
}

fn encode_noisy_jpeg(width: u32, height: u32, quality: u8) -> Vec<u8> {
    let mut img = RgbImage::new(width, height);
    let mut state: u32 = 0x1234_5678;
    for pixel in img.pixels_mut() {
        state = state.wrapping_mul(1664525).wrapping_add(1013904223);
        let r = (state >> 24) as u8;
        state = state.wrapping_mul(1664525).wrapping_add(1013904223);
        let g = (state >> 24) as u8;
        state = state.wrapping_mul(1664525).wrapping_add(1013904223);
        let b = (state >> 24) as u8;
        *pixel = image::Rgb([r, g, b]);
    }
    let mut out = Vec::new();
    JpegEncoder::new_with_quality(&mut out, quality)
        .write_image(img.as_raw(), width, height, image::ColorType::Rgb8.into())
        .expect("encode noisy jpeg");
    out
}

fn inject_jpeg_exif_app1(jpeg: &[u8]) -> Vec<u8> {
    assert!(jpeg.len() >= 2 && jpeg[0] == 0xFF && jpeg[1] == 0xD8);
    let exif_payload = b"Exif\0\0MM\0*\0\0\0\x08\0\x01\x01\x12\0\x03\0\0\0\x01\0\x06\0\0\0\0\0\0";
    let seg_len = (exif_payload.len() + 2) as u16;
    let mut out = Vec::with_capacity(jpeg.len() + exif_payload.len() + 4);
    out.extend_from_slice(&jpeg[..2]);
    out.push(0xFF);
    out.push(0xE1);
    out.extend_from_slice(&seg_len.to_be_bytes());
    out.extend_from_slice(exif_payload);
    out.extend_from_slice(&jpeg[2..]);
    out
}

fn encode_animated_gif(width: u32, height: u32) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut encoder = GifEncoder::new(&mut out);
        encoder.set_repeat(Repeat::Infinite).expect("set repeat");
        let f1 = RgbaImage::from_pixel(width, height, Rgba([255, 0, 0, 255]));
        let f2 = RgbaImage::from_pixel(width, height, Rgba([0, 0, 255, 255]));
        encoder
            .encode_frames([
                Frame::from_parts(f1, 0, 0, Delay::from_numer_denom_ms(100, 1)),
                Frame::from_parts(f2, 0, 0, Delay::from_numer_denom_ms(100, 1)),
            ])
            .expect("encode animated gif");
    }
    out
}

#[test]
fn clean_small_image_passes_through_with_identical_bytes() {
    let raw = encode_clean_rgb_png(64, 48);
    let normalized = normalize_image_bytes(&raw, &NormalizationPolicy::default())
        .expect("clean small png should normalize");

    assert!(normalized.passed_through);
    assert_eq!(normalized.media_type, "image/png");
    assert_eq!(normalized.data, raw);
}

#[test]
fn oversized_pixel_image_downscales_within_budget_preserving_aspect_ratio() {
    let raw = encode_clean_rgb_png(400, 200);
    let policy = NormalizationPolicy {
        max_pixels: 20_000,
        max_dimension: 8192,
        target_bytes: 4 * 1024 * 1024,
    };
    let normalized = normalize_image_bytes(&raw, &policy).expect("oversized pixel image");

    assert!(!normalized.passed_through);
    assert!(u64::from(normalized.width) * u64::from(normalized.height) <= policy.max_pixels);
    let ratio = f64::from(normalized.width) / f64::from(normalized.height);
    assert!((ratio - 2.0).abs() < 0.05, "aspect ratio drifted: {ratio}");
}

#[test]
fn oversized_long_edge_is_capped_by_max_dimension() {
    let raw = encode_clean_rgb_png(500, 40);
    let policy = NormalizationPolicy {
        max_pixels: 2048 * 2048,
        max_dimension: 200,
        target_bytes: 4 * 1024 * 1024,
    };
    let normalized = normalize_image_bytes(&raw, &policy).expect("long-edge cap");

    assert!(!normalized.passed_through);
    assert!(normalized.width.max(normalized.height) <= 200);
    assert_eq!(normalized.width, 200);
    assert_eq!(normalized.height, 16);
}

#[test]
fn quality_ladder_returns_smallest_output_when_tiny_byte_target_exhausted() {
    let raw = encode_noisy_jpeg(256, 256, 95);
    let policy = NormalizationPolicy {
        max_pixels: 2048 * 2048,
        max_dimension: 8192,
        target_bytes: 1024,
    };
    let normalized =
        normalize_image_bytes(&raw, &policy).expect("exhausted ladder must return smallest");

    assert!(!normalized.passed_through);
    assert_eq!(normalized.media_type, "image/jpeg");
    assert!(
        normalized.data.len() < raw.len(),
        "expected ladder q60 ({}) < original q95 ({})",
        normalized.data.len(),
        raw.len()
    );
    assert!(normalized.data.len() > 1024);
}

#[test]
fn animated_gif_reencodes_to_single_frame() {
    let raw = encode_animated_gif(32, 32);
    let before = detect_image(&raw).expect("detect gif");
    assert!(before.animated);

    let normalized = normalize_image_bytes(&raw, &NormalizationPolicy::default())
        .expect("animated gif should normalize");
    assert!(!normalized.passed_through);
    assert_ne!(normalized.media_type, "image/gif");

    let after = detect_image(&normalized.data).expect("detect normalized gif output");
    assert!(!after.animated);
    assert_eq!(after.width, 32);
    assert_eq!(after.height, 32);
}

#[test]
fn alpha_png_preserves_transparency_after_reencode() {
    let raw = encode_rgba_png_with_alpha(120, 120);
    let policy = NormalizationPolicy {
        max_pixels: 64 * 64,
        max_dimension: 8192,
        target_bytes: 4 * 1024 * 1024,
    };
    let normalized = normalize_image_bytes(&raw, &policy).expect("alpha png reencode");

    assert!(!normalized.passed_through);
    assert_eq!(normalized.media_type, "image/png");
    let after = detect_image(&normalized.data).expect("detect normalized png");
    assert!(after.has_alpha);
    let decoded = image::load(Cursor::new(&normalized.data), image::ImageFormat::Png)
        .expect("load normalized png")
        .to_rgba8();
    assert!(decoded.pixels().any(|p| p[3] < 255));
}

#[test]
fn jpeg_with_exif_metadata_strips_metadata_and_applies_orientation() {
    let base_jpeg = encode_noisy_jpeg(80, 40, 90);
    let with_exif = inject_jpeg_exif_app1(&base_jpeg);
    let before = detect_image(&with_exif).expect("detect exif jpeg");
    assert!(before.carries_metadata);
    // EXIF orientation 6 (Rotate90) swaps perceived 80x40 -> 40x80.
    assert_eq!((before.width, before.height), (40, 80));

    let normalized = normalize_image_bytes(&with_exif, &NormalizationPolicy::default())
        .expect("strip exif jpeg");
    assert!(!normalized.passed_through);
    let after = detect_image(&normalized.data).expect("detect stripped jpeg");
    assert!(!after.carries_metadata);
    assert_eq!((after.width, after.height), (40, 80));
}

#[test]
fn turn_with_five_images_passes_normalization_and_validation() {
    let _rt_guard = crate::agent::runtime_helper::TestRuntimeGuard::new();
    let png = encode_clean_rgb_png(32, 32);
    let encoded = BASE64_STANDARD.encode(&png);
    let images: Vec<TurnInputImage> = (0..5)
        .map(|i| TurnInputImage {
            data_url: format!("data:image/png;base64,{encoded}"),
            mime_type: "image/png".to_string(),
            name: Some(format!("img-{i}.png")),
        })
        .collect();

    let normalized = normalize_turn_images(images);
    assert_eq!(normalized.len(), 5);
    validate_turn_images(&normalized).expect("5 images must pass turn validation");
}
