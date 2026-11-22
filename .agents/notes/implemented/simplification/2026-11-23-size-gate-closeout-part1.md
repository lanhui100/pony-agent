# Agent Note: size 棘轮收口第一步——`config.rs` 与 `process.rs::start_inner`

Status: implemented

## Problem

`npm run size:check` 在 `main` 上红 4 条（缺口判据见 ADR `2026-11-23-size-gate-ci-gap`）。
其中两条与在途 PR 无文件交叠、可立即收口：

1. `crates/pony-agent-core/src/agent/config.rs`：3085 行，基线允许 ≤3078。
2. `crates/pony-agent-core/src/agent/process.rs:1303` 的 `start_inner`：280 行，基线允许 ≤278。

另两条（`provider/tests.rs` 3057、`runtime/tests.rs` 13913）正被 B139 的 `upstream_status`
字段补齐触碰，按窗口纪律留待那批 PR 合入后再做。

## Decision

1. **`config.rs`：把「能力目录与模型能力声明」整块抽成子模块**
   `agent/config/capabilities.rs`（113 行：`CapabilityCatalogEntry` / `ProviderReasoningEffort` /
   `ProviderCapabilityPreset` / `ProviderModelCapabilities` / `CAPABILITY_CATALOG` /
   `undeclared_model_capabilities`）。父模块用 `pub use` 保持原公开路径
   （`agent::config::ProviderModelCapabilities` 等不变），内部分项降到 `pub(crate)`（含字段，
   因为 `config.rs` 里仍有查表逻辑要读它们）。结果：`config.rs` 3085 → 2977 行。
2. **`process.rs`：把「空 program 前置校验」抽成 `ensure_program_present(request)`**
   —— 3 行断言变 1 行调用，`start_inner` 280 → 277 行；新函数带理由注释
   （空串 spawn 跨平台行为不一致，属调用方 bug 而非上游错误）。
3. **不做的事**：不调高 `scripts/file-size-baseline.json`，不压缩既有注释/断言凑行数。
   棘轮的意义是让文件变小，而不是让基线变大。

## Verification

- `bash scripts/cargo-lane.sh check -p pony-agent-core --all-targets` → 无 error。
- `npm run size:check` → `2 new violation(s)`（原 4 条），并出现 `1 shrunken baseline entr(ies)`。
- `bash scripts/cargo-lane.sh test -p pony-agent-core --lib` → 全绿（见 PR 描述）。

## Alternatives considered

- **调高基线两条（3078→3085、278→280）**：一行即可让门变绿，但 B249 的语义正是「重锚后把抽取
  落地」，调基线等于宣布棘轮失效；且这两条是最近才长出来的，没有历史包袱。
- **把 `ProviderProtocolEndpoint` 一起抽进 capabilities 模块**：它依赖 `ProviderProtocol` 与更多
  `default_*` helper，会让新模块反向依赖父模块私有项，耦合面变大；本轮只抽自包含的纯声明块。
- **彻底拆 `config.rs`（注册表 IO / 存储 / 视图分家）**：更彻底，但要同时移动
  `ProviderRegistryStore` 的 impl 与文件读写路径，回归面远超「让棘轮回绿」，应另立 ADR。
- **只抽 `config.rs`、`process.rs` 留待下轮**：只差 2 行，抽一个小 helper 成本极低，没理由分批。

## Reintroduction conditions

- **把 `capabilities.rs` 的内容搬回 `config.rs`**：仅当该父文件被整体拆分/替换时（例如注册表 IO 独立成
  `config/registry.rs`），随那次拆分方案一并迁移；不得因为「少一个 `mod` 声明」把 113 行纯声明塞回父文件。
- **把 `ensure_program_present` 内联回 `start_inner`**：仅当 `start_inner` 因其它重构整体变短
  （实测 ≤270 行）且该断言重新成为唯一前置检查时；在那之前它由函数行数棘轮守着。
