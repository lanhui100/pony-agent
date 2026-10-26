//! 构建期环境变量的**变更感知**（`option_env!` 的必需配套）。
//!
//! ## 为什么需要这个文件
//!
//! `option_env!("X")` 在编译期把变量值烧进二进制，但 **cargo 默认不追踪 `option_env!` 读的
//! 环境变量**：只要源码没变，增量构建会直接复用上次的目标产物 —— 变量改了也不会重新编译。
//!
//! 实测事故（2026-10-27）：给 `cargo build --release` 传入 `PONY_OTEL_BUILD_TOKEN` 后构建
//! 成功、退出码 0，但产物里**没有该 token**（`strings | grep -c '^otl_'` 为 0）——
//! 因为 `otel/mod.rs` 未改动，cargo 复用了旧对象文件。若在发布流水线上发生，
//! 结果是「新版本发布了、token 却仍是旧的/空的」，且**构建日志完全看不出来**。
//!
//! 这里用 `cargo:rerun-if-env-changed` 修掉同一类风险，两个构建期凭证都在列：
//! - `PONY_OTEL_BUILD_TOKEN`（ADR 0049：OTel 准入 token）
//! - `PONYSENTRY_BUILD_CLIENT_TOKEN`（ADR 0037：PonySentry client token）

fn main() {
    // 构建期凭证变更必须触发重编（否则增量/缓存构建会烧进旧值）。
    println!("cargo:rerun-if-env-changed=PONY_OTEL_BUILD_TOKEN");
    println!("cargo:rerun-if-env-changed=PONYSENTRY_BUILD_CLIENT_TOKEN");

    // 其它构建期标识：同样参与 option_env!，一并纳入变更感知。
    println!("cargo:rerun-if-env-changed=PONY_RELEASE_BUILD");

    // 本文件自身变更也要触发。
    println!("cargo:rerun-if-changed=build.rs");
}
