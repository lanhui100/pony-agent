# Wave43 task-1 依赖预演收据（OTel 家族 vs 既有 tokio/reqwest 共仓共存）

> 执行者：protocol-agent（L2-P）
> 隔离目录：`/tmp/otel-dep-probe-nyPs4L`（`mktemp -d /tmp/otel-dep-probe-XXXXXX`），**未写入本仓 target**。
> 本机：`cargo 1.91.1` / `rustc 1.91.1`。

## 0. 结论（先给答案）

| 项 | 结论 |
| :--- | :--- |
| 锁定版本（三件套一致） | `opentelemetry = 0.30.0` / `opentelemetry_sdk = 0.30.0` / `opentelemetry-otlp = 0.30.0`（`opentelemetry-proto = 0.30.0`、`opentelemetry-http = 0.30.0` 同族） |
| 与仓库既有版本共存 | `reqwest 0.12.28` / `tokio 1.52.3` / `hyper 1.10.1` / `hyper-util 0.1.20` **零重复实例**，`cargo check` Exit **0** |
| prost | **引入** `prost 0.13.5`（`http-proto` 必需，protobuf 编码） |
| tonic | **也被引入** `tonic 0.13.1`（**与计划预期不符**，见 §4；仅 `codegen` feature，无 `channel`/`tls`，不编译 gRPC 传输层） |
| 是否需回退 | 不需要。0.30.x 一次通过，无版本冲突、无回退 |
| 新增传递依赖 `tracing` | 是，`tracing 0.1.44`（与仓库 `Cargo.lock` 现有版本同号；仅库依赖，不引入 `tracing-subscriber`，不违反 D1） |

---

## 1. 隔离 crate 依赖清单（原文）

`/tmp/otel-dep-probe-nyPs4L/Cargo.toml`：

```toml
[package]
name = "otel_probe"
version = "0.1.0"
edition = "2021"

[dependencies]
# --- OTel 三件套（目标 0.30.x，default-features = false）---
opentelemetry = { version = "=0.30.0", default-features = false, features = ["trace"] }
opentelemetry_sdk = { version = "=0.30.0", default-features = false, features = ["trace", "rt-tokio"] }
opentelemetry-otlp = { version = "=0.30.0", default-features = false, features = ["http-proto", "reqwest-client", "trace"] }

# --- 与 pony-agent-core 现有声明逐字对齐（仅用于共存验证）---
reqwest = { version = "0.12", default-features = false, features = ["json", "native-tls", "charset", "stream"] }
tokio = { version = "1", default-features = false, features = ["rt", "rt-multi-thread", "time", "net", "sync"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
chrono = { version = "0.4", default-features = false, features = ["clock", "std"] }
uuid = { version = "1", features = ["v4", "serde"] }
```

`src/main.rs` 不是空跑：真实引用 `SpanExporter::builder().with_http().with_endpoint().with_timeout()`、`SdkTracerProvider::builder().with_batch_exporter().with_sampler(Sampler::ParentBased(TraceIdRatioBased))`、`Resource::builder().with_service_name()`、`Tracer::start()`、`SpanContext::trace_id()`、`provider.shutdown()`，确保 feature 组合下 API 符号真实存在（而非仅"解析得动"）。

## 2. 命令原文与 exit code

### 2.1 首轮（未 pin，取 crates.io 最新 1.x 兼容版本）

```console
$ cd /tmp/otel-dep-probe-nyPs4L && cargo check --all-targets
    Checking opentelemetry_sdk v0.30.0
    Checking opentelemetry-proto v0.30.0
    Checking opentelemetry-http v0.30.0
    Checking reqwest v0.12.28
    Checking opentelemetry-otlp v0.30.0
    Checking otel_probe v0.1.0 (/tmp/otel-dep-probe-nyPs4L)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 56.85s
EXIT_CODE=0
```

### 2.2 对齐仓库 `Cargo.lock` 的精确版本（关键证据：不是"最新能过"，而是"仓库锁定的版本能过"）

```console
$ cargo update -p tokio --precise 1.52.3
 Downgrading tokio v1.54.1 -> v1.52.3
$ cargo update -p hyper --precise 1.10.1
 Downgrading hyper v1.12.0 -> v1.10.1
$ cargo update -p hyper-util --precise 0.1.20
 Downgrading hyper-util v0.1.21 -> v0.1.20
 Removing base64 v0.23.1

$ cargo check --all-targets
    Checking hyper-tls v0.6.0
    Checking reqwest v0.12.28
    Checking opentelemetry-http v0.30.0
    Checking opentelemetry-otlp v0.30.0
    Checking otel_probe v0.1.0 (/tmp/otel-dep-probe-nyPs4L)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 18.09s
EXIT_CODE=0
```

### 2.3 单实例证明（`cargo tree -d` 无 tokio/hyper/reqwest/opentelemetry 重复）

```console
$ cargo tree -d | grep -E "^(tokio|hyper|hyper-util|reqwest|opentelemetry) v"
NO_DUPLICATE(tokio/hyper/reqwest/opentelemetry)
EXIT=0
```
（`cargo tree -d` 只报 base64 / getrandom / syn 等与本议题无关的重复；`base64` 双版本仅 `2.2.1` vs `2.3.1` 且来自 hyper-util 内部，repo `Cargo.lock` 亦已如此。）

### 2.4 最终解析版本（probe `Cargo.lock`，pkg_count=169）

```console
hyper                  1.10.1
hyper-util             0.1.20
opentelemetry          0.30.0
opentelemetry-http     0.30.0
opentelemetry-otlp     0.30.0
opentelemetry-proto    0.30.0
opentelemetry_sdk      0.30.0
prost                  0.13.5
reqwest                0.12.28
tokio                  1.52.3
tonic                  0.13.1
tracing                0.1.44
tracing-core           0.1.36
```

## 3. prost / tonic 溯源（manifest 原文 + tree 反查）

### 3.1 `opentelemetry-otlp 0.30.0` 的 `http-proto` feature 定义（原文摘录，registry 缓存）

```toml
http-proto = [
    "prost",
    "opentelemetry-http",
    "opentelemetry-proto/gen-tonic-messages",
    "http",
    "trace",
    "metrics",
]
```

### 3.2 `opentelemetry-proto 0.30.0` 的 `gen-tonic-messages`（原文摘录）

```toml
gen-tonic-messages = [
    "tonic",
    "prost",
]
gen-tonic = [
    "gen-tonic-messages",
    "tonic/channel",
]
```

### 3.3 `cargo tree -i tonic`（真实反查）

```console
$ cargo tree -i tonic
tonic v0.13.1
└── opentelemetry-proto v0.30.0
    └── opentelemetry-otlp v0.30.0
        └── otel_probe v0.1.0 (/tmp/otel-dep-probe-nyPs4L)
```

`cargo tree -i prost`：`prost 0.13.5` 被 `opentelemetry-otlp`、`opentelemetry-proto`、`tonic` 三方共同依赖。

### 3.4 `tonic` 实际启用的 feature（真实反查）

```console
$ cargo tree -e features -i tonic
tonic v0.13.1
├── tonic feature "codegen"
│   └── opentelemetry-proto v0.30.0
│       └── opentelemetry-otlp v0.30.0
│           ├── opentelemetry-otlp feature "http"
│           │   └── opentelemetry-otlp feature "http-proto"
...
```

**结论**：`http-proto` 走 `gen-tonic-messages` ⇒ `tonic` 作为 **prost 消息类型的 codegen 宿主** 被编入，但**未**启用 `tonic/channel` / `tonic/tls`，因此不编译 gRPC 传输栈。`default-features = false` 无法规避 `tonic`；若必须零 tonic，只能放弃 http-proto 走 `http-json`（同样需要 prost + gen-tonic-messages，仍无法规避），**无解，接受现状**。

## 4. 与计划的偏差（需 Lead 知晓）

| # | 计划表述 | 实测 | 影响 |
| :--- | :--- | :--- | :--- |
| P1 | "记录是否**不**引入 tonic（grpc 才需要）" | tonic 0.13.1 **被引入**（仅 codegen feature） | 二进制体积增加（无传输层代码）；`Cargo.lock` 新增 tonic 条目。属预期外但可接受 |
| P2 | 计划 §3 D1 写 "无 `tracing`" | 实测 `tracing 0.1.44` 成为传递库依赖（`hyper-util` 与 `opentelemetry 0.30` 无条件引用） | 版本与仓库 `Cargo.lock` 现有 `tracing 0.1.44` 一致，**不新增版本、不引入 subscriber**；D1"不引入 tracing 生态"仍成立 |
| P3 | task 描述仅列 `http-proto`/`reqwest-client`/`trace` | `http-proto` 会**连带开启** `opentelemetry-otlp/metrics` 与 `logs` 之外 metrics 路径（`opentelemetry/metrics`、`opentelemetry_sdk/metrics`） | 编译面略大于"只做 tracing"预期；功能上不导出 metrics，仅多编译 sdk metrics 模块 |
| P4 | 计划未提 `rt-tokio` | `opentelemetry_sdk` 需显式 `features = ["rt-tokio"]` 才能用 `with_batch_exporter`（BatchSpanProcessor 依赖 tokio runtime） | 依赖声明需加 `rt-tokio`，task-2 已据此落盘 |

## 5. 复现命令（任何人可重跑）

```console
$ D=$(mktemp -d /tmp/otel-dep-probe-XXXXXX) && cd "$D" && cargo init --name otel_probe --edition 2021
#   写入 §1 的 Cargo.toml + src/main.rs
$ cargo update -p tokio --precise 1.52.3
$ cargo update -p hyper --precise 1.10.1
$ cargo update -p hyper-util --precise 0.1.20
$ cargo check --all-targets ; echo "EXIT=$?"
EXIT=0
```

## 6. 未验证项（诚实标注）

- 未在**本仓 workspace** 内跑 `cargo check`（task-1 明令禁止修改本仓）。真实进仓后的 lock 变更（新增 5 个 opentelemetry* + prost + tonic，hyper-util 是否被 workspace 其他成员抬高）由 task-2 的 `cargo check --manifest-path src-tauri/Cargo.toml --target-dir target-check` 提供收据。
- 未做**运行时**导出验证（无 collector 可达）；导出超时/失败语义属 task-2 之后的 B2/L2-AT 范围。

## 7. 补录：进仓后 workspace 级解析收据（task-2 期间补测，闭合 §6 第 1 条未验证项）

`crates/pony-agent-core/Cargo.toml` 已按 §1 声明三件套（`=0.30.0` 精确锁步）：

```console
$ cargo check --manifest-path src-tauri/Cargo.toml --target-dir target-check
    Checking pony-agent-core v0.1.128 (/home/dm/pony-agent/crates/pony-agent-core)
    Checking pony-agent v0.1.155 (/home/dm/pony-agent/src-tauri)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 1m 43s
CARGO_CHECK_EXIT=0
```

仓库根 `Cargo.lock` 解析结果（与隔离 probe 完全一致，`hyper-util` 未被抬高）：

```console
hyper                  1.10.1
hyper-util             0.1.20
opentelemetry          0.30.0
opentelemetry-http     0.30.0
opentelemetry-otlp     0.30.0
opentelemetry-proto    0.30.0
opentelemetry_sdk      0.30.0
prost                  0.13.5
reqwest                0.12.28   （0.13.4 为 HEAD 既有，由 tauri 侧引入，本次未新增重复实例）
tokio                  1.52.3
tonic                  0.13.1
```

lock 增量（`git diff Cargo.lock | grep '^+name = '`）仅 15 个新包：
`opentelemetry` `opentelemetry-http` `opentelemetry-otlp` `opentelemetry-proto` `opentelemetry_sdk`
`prost` `prost-derive` `tonic` `pin-project` `pin-project-internal` `rand` `rand_chacha` `rand_core`
`tokio-stream` `ppv-lite86`。**零** tokio/hyper/reqwest 版本抖动。
