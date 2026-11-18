# Agent Note: Rust lint 棘轮工具链 pin（clippy 0.1.91 / rustc 1.91.1）

Status: implemented

工单：B245（B243 的 CI 接线回归）。影响文件：`rust-toolchain.toml`、`.github/workflows/ci.yml`。

## Problem

B243 把 clippy 棘轮门接进 Linux CI 后，首个真实 CI run（job 134479825726，commit 45121be0）
在 `npm run rust:lint` 判红：实测 4034 / 基线 3870，新增 106 条（`clippy::map_unwrap_or`
为主，另有 `useless_conversion` / `manual_is_variant_and`）。

根因是**工具链口径漂移**而非代码回退：

- 本地基线由 `rustc 1.91.1 + clippy 0.1.91` 生成，基线元数据记录 `toolchain=clippy 0.1.91`；
- CI 的 `actions-rust-lang/setup-rust-toolchain@v1` 未指定版本 ⇒ 浮在 latest stable，
  实测 `clippy 0.1.100 (0ef1bc3860 2026-11-09)`；
- 新 clippy 版本扩大了 `map_unwrap_or` 等 lint 的触发面，同一份源码多出 106 条告警，
  被棘轮门如实判成「新增债」。

门本身语义正确（fail-closed），错的是「基线按固定口径、CI 按浮动口径」这一组合：
每次上游工具链发版都会让门随机变红，且红因与代码无关。

## Decision

**在仓内 pin 工具链**：新增 `rust-toolchain.toml`（`channel = "1.91.1"`、`components = ["clippy", "rustfmt"]`），
并在 `ci.yml` 两个 `setup-rust-toolchain` 步骤显式写 `toolchain: "1.91.1"` + `components: clippy`。

- pin 的版本 = 基线生成版本，CI 与本地同口径，棘轮门的红/绿只反映代码变化；
- 升级工具链成为一次**显式变更**：改 `rust-toolchain.toml` + 跑 `npm run rust:lint:update`
  （上涨需 `--allow-growth` 并 review diff）+ 同提交更新 ADR 与 `.meta/lints/README.md`；
- 不在 CI 里做「检测到漂移自动重基线」——那会让门退化成永远绿。

## Alternatives considered

- **A. 按 CI latest（clippy 0.1.100）重基线，不 pin。** 落选：基线随上游发版反复失效，
  每次都要人工重基线 106+ 条；且不同平台 CI 缓存的 stable 版本可能不同步，绿/红不可归因。
- **B. 门在工具链版本不符时 exit 2（fail-closed）。** 落选：门已经有了（基线元数据记录
  toolchain），但如果不 pin，CI 每次发版都 exit 2，等于把「保持红」制度化；pin 才是治本。
- **C. 只 pin CI、不加 rust-toolchain.toml。** 落选：本地开发仍可能用不同工具链跑门，
  出现「本地绿 CI 红」的错位；仓内 pin 让两边单一真源。
- **D. 把 CI 降级成 continue-on-error / 仅提示。** 落选：等于放弃棘轮门，和 B243 的验收相悖。

## Verification

- `rustc --version` = 1.91.1、`cargo clippy --version` = 0.1.91（本地基线口径）；
- 推 CI 后 Linux `Clippy 棘轮门 + 门自检` 步骤复跑 `npm run rust:lint:selftest`（34/34）与
  `npm run rust:lint`（期望 exit 0，4034 vs 3870 的口径差随工具链 pin 消失）。
