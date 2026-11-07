# 契约矩阵：wave-67 `pony update` CLI 自更新（C 级）

- 用户裁决（2026-11-07）：① 更新对象 = **`pony`（TUI 二进制）**，命令 `pony update`；② **同批扩发布流水线**（ubuntu job 构建 + sha256 + 平台命名上传）；③ 更新源 = **GitHub Releases + 强制 sha256**，无 `--force` 不允许降级。
- 基线 commit：`67e656c9`（`cargo test -p pony-agent-core --lib` = 1348 passed；收据 `.dev-team/regression-baseline-wave67.json`）
- 现状事实（侦察已验）：`pony` 有 `/update` 斜杠命令但**只检查不安装**（`tui.rs:2051 perform_update_check`，5s 超时，GitHub latest API + S3 回退）；`ponycli` 无版本号无更新；`release.yml` 只构建 Windows 桌面包（`runs-on: windows-latest`），**无任何 job 产出 linux CLI 二进制**；`v0.1.182` 里的 `pony-tui_0.1.155_linux-x86_64(.sha256)` 是人工上传且落后 tag；`latest.json` 是桌面 Tauri 更新清单（仅 windows，minisign），不可复用。
- 依赖已就绪（无需新增 crate）：`reqwest 0.12`(json,native-tls,charset,stream)、`sha2 0.10`、`serde_json`、`tokio`(rt,time,net,sync)。

## C1 模块与 API：新增 `crates/pony-agent-core/src/agent/updater.rs`

平台标识（发布侧与客户端唯一真值，`#[non_exhaustive]` 风格字符串常量）：
`linux-x86_64` / `linux-aarch64` / `macos-x86_64` / `macos-aarch64` / `windows-x86_64`

```rust
pub const CLI_MANIFEST_ASSET: &str = "pony-cli-latest.json";
pub const DEFAULT_UPDATE_REPO: &str = "lanhui100/pony-agent";
pub const DEFAULT_UPDATE_API_BASE: &str = "https://api.github.com";
pub const DEFAULT_DOWNLOAD_BUDGET_SECS: u64 = 300;
pub const MAX_ALLOWED_DOWNLOAD_BUDGET_SECS: u64 = 1800;
pub const MAX_ARTIFACT_BYTES: u64 = 128 * 1024 * 1024;

pub fn current_platform() -> Option<&'static str>;

#[derive(Deserialize)] pub struct CliManifest { pub schema: String, pub version: String, #[serde(default)] pub generated_at: Option<String>, pub platforms: BTreeMap<String, PlatformAsset> }
#[derive(Deserialize)] pub struct PlatformAsset { pub asset: String, pub sha256: String, pub size: u64 }

pub fn parse_cli_manifest(raw: &str) -> Result<CliManifest, UpdateError>;      // schema 必须 == "pony-cli-manifest"；version 必须可解析 semver；sha256 必须 64 位小写 hex（大写→规范化为小写）
pub fn select_platform_asset<'a>(m: &'a CliManifest, platform: &str) -> Option<&'a PlatformAsset>;
pub fn plan_update(m: &CliManifest, platform: &str, current_version: &str, force: bool) -> Result<UpdatePlan, UpdateError>;
// UpdatePlan { pub version: String, pub asset: String, pub sha256: String, pub size: u64, pub is_downgrade: bool, pub is_same: bool }
// 规则：无该平台条目 ⇒ UnsupportedPlatform（错误串列出 manifest 已有的平台）；version 比当前旧 ⇒ 非 force 时 Downgrade；相等 ⇒ 非 force 时 AlreadyLatest（由调用方映射为「已是最新」成功态）
pub fn verify_sha256(path: &Path, expected_hex: &str) -> Result<(), UpdateError>;  // 流式 64KiB 读，禁止整文件进内存
pub fn install_atomically(downloaded: &Path, target_exe: &Path) -> Result<InstallReceipt, UpdateError>;
// 同目录临时名 `<exe>.new-<pid>`；chmod 0o755（unix）；POSIX rename 覆盖（目标永不缺失）；
// 失败路径必删临时文件；InstallReceipt { pub target: PathBuf, pub bytes: u64 }
pub fn resolve_install_path() -> Result<PathBuf, UpdateError>;  // std::env::current_exe()；测试可被 `PONY_UPDATE_INSTALL_PATH` 覆盖
pub fn parse_semver_triple(v: &str) -> Option<(u64,u64,u64)>;   // 从 tui.rs:2031 迁入（tui.rs 改为 use 引用，其既有测试保持通过）
pub fn is_newer_semver(candidate: &str, current: &str) -> bool; // 同上迁入

pub enum UpdateError { Network(String), HttpStatus(u16), Manifest(String), Signature(String),
                        NotWritable(String), Downgrade(String), UnsupportedPlatform(String),
                        MissingAsset(String), Io(String), TooLarge(u64) }
impl UpdateError { pub fn user_message(&self) -> String; pub fn exit_code(&self) -> i32; }
```

下载（内部）：`download_artifact(client, url, dest, expected)` 流式 `bytes_stream()` → 边写边累计 → 超 `MAX_ARTIFACT_BYTES` 或超 `size` 立即中止并删临时文件 → 结束后校验 sha256 与 size。**禁止整文件读入内存**。

环境变量（全部非法值回退默认，禁止静默取 0）：
| 变量 | 默认 | 校验 |
|---|---|---|
| `PONY_UPDATE_API_BASE` | `https://api.github.com` | 必须以 `https://` 开头 |
| `PONY_UPDATE_REPO` | `lanhui100/pony-agent` | 必须匹配 `^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$` |
| `PONY_UPDATE_MANIFEST_URL` | 空（=走发现流程） | 必须 `https://`；**测试/镜像用**，设置后跳过发现并使用其 API 基址做资产下载 |
| `PONY_UPDATE_DOWNLOAD_BUDGET_SECS` | 300 | 1..=1800 |
| `PONY_UPDATE_INSTALL_PATH` | 空（=current_exe） | **仅测试/CI 用**，设置后替换目标改此路径 |
| `PONY_UPDATE_ALLOW_INSECURE` | 空 | 仅当 == `1` **且** 设置了 `PONY_UPDATE_MANIFEST_URL` 时生效：放开 http/localhost（见 C4 逃生舱） |

## C2 命令面：`pony update`（`crates/pony-agent-core/src/bin/tui.rs`）

- `CliAction::Update { check: bool, force: bool, pin: Option<String>, json: bool }`，解析点：`CliAction` 枚举（tui.rs:13017-13026）+ `parse_cli_action`（:13028-13045）+ `main` 派发（:13308-13358）。
- 语义：`pony update` = 检查→下载→校验→安装→**回验**（新二进制 `--version` 必须等于 manifest.version，否则回滚失败退出）；`--check` 只检查不安装；`--force` 允许降级/重装；`--pin <ver>` 指定版本（需该 release 含 CLI 清单，找不到即 MissingAsset）；`--json` stdout 只出行 JSON 收据。
- 成功收据（人类可读）：`已更新 pony: v<old> → v<new>，sha256=<前12位>…，路径 <path>`；已最新：`当前已是最新版本（v<ver>）`。
- **退出码**：`0` 成功（含「已是最新」）；`1` 校验失败（sha256/size/回验不符）；`2` 用法错误；`3` 网络/HTTP/清单失败；`4` 安装失败（不可写/rename 失败）。与 `tui.rs` 既有 `exit` 口径共存，不改变其它子命令行为。
- `/update` 斜杠命令：改为复用 `updater` 的检查实现，文案追加一行「本机升级：pony update」（**不自动替换用户的二进制**）。

## C3 发布侧：`.github/workflows/release.yml` 新增 job

新增 `build-cli-linux`（`runs-on: ubuntu-latest`，**`needs: build-and-release`** —— 桌面 job 内部完成 publish，故 CLI job 在其后 `gh release upload --clobber`；若 CLI job 失败，发布已发生但缺 CLI 清单，客户端发现逻辑会回落到更旧的含清单 release，属**优雅降级**而非坏版本，需在 job 里显式打印该风险），步骤契约：
1. `cargo build --release --bin pony`；
2. `ver=$(./target/release/pony --version)` 解析出的版本必须 == `crates/pony-agent-core/Cargo.toml` 的 `version`（**core 版本链**，fail-closed）。**不要**与 `package.json` 比较：本仓是**两条独立版本线**（实测 `core=0.1.156`、`tauri/app=0.1.183`，`npm run version:check` 明确输出「Core version ok / Tauri version ok (4 places)」），CLI 二进制只承载 core 版本；
3. 产物命名 `pony-tui_<ver>_linux-x86_64` + `pony-tui_<ver>_linux-x86_64.sha256`（`sha256sum` 输出格式 `<hex>  <filename>`）；
4. 生成 `pony-cli-latest.json`：`{"schema":"pony-cli-manifest","version":"<ver>","generated_at":"<RFC3339 UTC>","platforms":{"linux-x86_64":{"asset":"pony-tui_<ver>_linux-x86_64","sha256":"<hex>","size":<bytes>}}}`；
5. 在 release 已发布后 `gh release upload "$tag" <4 files> --clobber`；
6. **收尾断言**：`gh api repos/$repo/releases/tags/$tag` 中必须出现 `pony-cli-latest.json` 且其 `version` == `$ver`，否则 job 失败。
- 不改动既有 Windows 桌面打包/`latest.json` 组装逻辑；`ponycli` 本次**不发布**（用户裁决 A）。

## C4 发现逻辑（客户端）

1. `GET {api_base}/repos/{repo}/releases?per_page=20`（含 `Accept: application/vnd.github+json`、`User-Agent: pony-update/<ver>`）；
2. 过滤 `draft=false && prerelease=false`，按 `published_at` 降序取**第一个** assets 含 `pony-cli-latest.json` 的 release；
3. 下载该 asset 的 `browser_download_url` → `parse_cli_manifest`（**版本以 manifest.version 为准，绝不使用 tag_name**——现状 tag `v0.1.182` 与 CLI 版本 `0.1.155` 已证明二者会错位）；
4. `--pin <ver>` = 指定 **CLI(core) 版本**：按步骤 1 的列表倒序遍历（最多 20 条），取**第一个** `pony-cli-latest.json` 中 `version == <ver>` 的 release，再按步骤 3 处理；扫完无匹配 ⇒ `MissingAsset`（错误串须提示「--pin 指 CLI 版本，不是 release tag」）。**不得**用 `releases/tags/v<ver>`（core 版本与 tag 版本是两条线，实测 core 0.1.156 vs tag v0.1.183）；
5. 资产下载 URL 必须与清单来自**同一 release** 的 `browser_download_url`（不得用 tag 拼字符串）。

**重定向（实测，2026-11-07）**：仓库既有约定是**全局禁止 reqwest 自动重定向**（`web_access.rs:18-19` 决策 8，`max_redirects` 默认 5，逐跳校验）。实测真实资产链路为
`github.com/.../releases/download/...` → **302** → `https://release-assets.githubusercontent.com/github-production-release-asset/...`（签名 SAS URL）→ **200**。
故 updater 必须：`redirect::Policy::none()` **手动**跟随 ≤5 跳，每一跳都校验 scheme + host，最终落点也必须在白名单内；超跳数 ⇒ `Network` 失败。

**信任锚 / host 白名单**：`api.github.com`、`github.com`、`release-assets.githubusercontent.com`、`objects.githubusercontent.com`（后两者为资产 CDN，前者为历史/兼容路径），scheme 必须 `https`。sha256 不符 ⇒ 立即中止且**不触碰原二进制**。

**测试/镜像逃生舱（fail-closed）**：只有**同时**设置 `PONY_UPDATE_MANIFEST_URL` 与 `PONY_UPDATE_ALLOW_INSECURE=1` 时，才允许 `http://` 与 host ∈ {`127.0.0.1`,`localhost`,`[::1]`} 与 manifest 自身的 host；只设其一 ⇒ 仍按生产规则拒绝。该组合仅测试/CI 使用，行为写入 ADR 与 observability 声明。

## C5 NFR（本波为 B/C 级全量 NFR；`.dev-team/nfr-baseline.json` 需增补 `download_budget` 字段）

| 维度 | 契约 |
|---|---|
| 控制面超时 | connect 5s / total 5s（沿用 `perform_update_check` 现有 5s 口径） |
| 下载预算 | connect 10s、总预算 300s（`PONY_UPDATE_DOWNLOAD_BUDGET_SECS` 1..=1800，非法回退 300）；**这是对 `external_call_timeout_ms=3000` 的显式例外**，理由：28MB 级二进制不可能 3s 内下完；例外只作用于「制品下载」这一条路径，控制面仍受 5s 约束 |
| 体积上限 | `MAX_ARTIFACT_BYTES = 128MiB`；超限即 `TooLarge` 并删临时文件 |
| 重试 | 清单/列表请求最多 2 次尝试（1 次重试，退避 500ms）；**制品下载不重试**（避免半包累积），失败即整轮失败 |
| 并发 | 同目录 lock 文件 `<exe>.update.lock`（`OpenOptions::create_new`）；已存在 ⇒ `NotWritable("另一个 pony update 正在运行")`；无论成败都删除（含 panic 场景用 `Drop` 守卫） |
| 资源释放 | 所有临时文件（`.new-<pid>`、`.part`、lock）在成功与失败路径都必须删除；下载失败不得留下可执行位残留 |
| 日志 | 关键阶段单行：`[pony-update] phase=<discover|manifest|download|verify|install|verify_installed> status=<ok|fail> ...`；CLI 进程无内置 tracer，trace_id 缺失按既有 `logging_legacy_exempt` 口径处理并在 observability 声明里说明 |
| 不可逆面 | 覆盖用户可执行文件属不可逆面：必须在交付说明单列「是否可回滚（保留旧版本的做法：失败即不动、成功即覆盖，用户可用 `--pin` 回退，但无自动备份）」，并在 preprod 前置验证（本机 e2e） |

## C6 安全边界

1. sha256 **强制**：清单缺 `sha256`/长度非 64 hex ⇒ `Manifest` 错误；不提供「跳过校验」开关。
2. scheme 必须 `https`；下载 host 白名单见 C4。
3. 拒绝降级（无 `--force`）、拒绝同版本重装（无 `--force`）。
4. 目标不可写 ⇒ `NotWritable` 且给出 `sudo` / 改 `PONY_UPDATE_INSTALL_PATH` / 换安装目录的具体提示；**不得**静默改为写用户目录。
5. 回验：安装后用新二进制 `--version` 复核版本；不符 ⇒ 立即用保留的旧文件恢复并 `1` 退出（POSIX 下先 `rename(target→old)` 备份再 `rename(new→target)` 仅在需要回滚时使用；正常路径用 rename 覆盖）。
6. **不支持自更新的平台**（Windows：运行中 exe 无法被替换）⇒ `UnsupportedPlatform` 明确指引「用桌面端安装包/`Pony.Agent_*_setup.exe`」。

## C7 测试契约（Test Agent 冻结；`updater.rs` 内 `#[cfg(test)] mod` + tui.rs 内 CLI 解析测试）

- 单测（真断言，禁 skip）：semver 解析/比较边界（`v1.2.3`/`1.2.3`/`1.2`/`1.2.3.4`/空/非数字）；manifest 解析（合法、schema 错、sha 非 64hex、version 非法、空 platforms）；`select_platform_asset`/`plan_update`（有更新 / 已最新 / 降级拒绝 / force 降级 / 平台缺失错误串含已有平台 / pin 语义）；`verify_sha256`（对 / 错一字节 / 文件不存在）。
- 集成（自建极简 HTTP mock：`std::net::TcpListener` 单线程、固定响应，**不得**复用 runtime 的私有 `MockHttpServer`）：
  1. 正常路径：清单→下载→校验→`install_atomically` 到临时目录 → 新文件内容与 sha 一致且具备可执行位；
  2. sha256 不匹配 ⇒ 目标文件**逐字节未变**且临时文件已清理；
  3. 清单 404 / 畸形 JSON / 平台缺失 ⇒ 对应 `UpdateError` 且无写盘；
  4. 下载被截断（Content-Length 大于实际）⇒ 失败、无写盘；
  5. lock 已存在 ⇒ `NotWritable`，且不删除他人 lock。
- L2-AT 对抗注入（确定性，禁 sleep 竞态）：超时注入（mock 半响应后停滞，断言在预算内失败）、超大体积（>128MiB 声明 ⇒ `TooLarge` 且不落盘）、并发两实例（同时 `create_new` lock，二者必有一个失败）、只读目标目录（`chmod 0555` 后断言 `NotWritable` 且不产生临时文件）。
- 端到端（必产物理收据，机器跑）：用 `PONY_UPDATE_MANIFEST_URL` + 本地 mock + `PONY_UPDATE_INSTALL_PATH=/tmp/.../pony` 跑 `pony update`，再用 `--version` 回验；另跑一次**真实 GitHub** `pony update --check --json` 并落盘 stdout（真网络收据，允许网络失败则记 `[DEGRADED]` 并说明）。

## C8 文档与治理（同提交）

- ADR：新增 `docs/decisions/0087-cli-self-update.md`（决策 + 备选 + 不可逆面 + 回滚口径），并追加 `docs/decisions/README.md` 索引。
- Note：`.agents/notes/implemented/feature/2026-11-07-cli-self-update.md`（`Status: implemented` 骨架，正文现在时）。
- `docs/observability.yaml`：追加本变更四维归属（新增网络 IO + 新增 CLI 子命令接口面；metrics 结构性 N-A 理由；tracing/logs 落点），并跑 `audit-diff.sh` / `check-declaration.sh`。
- `.dev-team/nfr-baseline.json`：增补 `download_budget`（300s/1800s 上限/128MiB/无重试）与例外理由，过 `dev-nfr-lint.py` Exit 0。

## 门禁

- **`deny-warnings` 对齐（CI 同口径，硬门禁）**：CI 用 `RUSTFLAGS=-D warnings`（`.github/workflows/ci.yml:59/87/93`），且 `crates/pony-agent-core/tests/tui_acceptance.rs:6-8` 以 `#[path] mod tui;` 把 bin 当模块引入。因此：
  `RUSTFLAGS="-D warnings" cargo check -p pony-agent-core --lib --bins` 必须 Exit 0，且 `node scripts/check-rust-test-lints.mjs` Exit 0。新增代码**不得**引入任何死代码/未使用导入（`updater` 的 pub 项在 lib 里不算 dead_code；tui.rs 私有 fn 由 `main()` 触达，且 tui.rs 已有 `#![cfg_attr(test, allow(dead_code))]`）。
- 红相：Test Agent 落盘后 `cargo test -p pony-agent-core --lib updater` 与 `--lib update` 必须真实失败（断言失败/空桩 panic，非编译错误）。
- 绿相：`cargo test -p pony-agent-core --lib` 全绿（≥1348 + 新增）；`pa109/pa110/pa112/tool_surface_singularity/wave61_team_activation/harness_optimizations_test` 全绿。
- 审查：Gate-1 方案对抗 1 路 + L3 两路（A：网络/文件/并发/原子性；B：契约/安全/回归面）+ L3-P 六问（含 NFR 例外与不可逆面）。
- 真实冒烟：本机 e2e（mock 清单全链路）+ `pony update --check --json` 真网络收据。

## AMEND-3（Lead 裁决，实施回执后）

1. **C1 增补 4 个 pub 项（接受）**：`UpdateOptions{check,force,pin}`、`UpdateOutcome{previous_version,latest_version,updated,install_path,sha256}`、`run_self_update(...)`、`UpdateError::code()`。
   理由：`pony update` 的实现在 **bin 目标**里，只能触达 `pony_agent_core` 的 `pub` 面；若把发现/下载留在私有 fn，`-D warnings` 下必成死代码。这是 C1 的一处洞，按「最保守语义」补齐。
2. **`verify_sha256` 语义收紧（接受）**：摘要不符时**当场删除待安装产物**（只删 `.part`/待装临时件，绝不碰既有二进制）。依据：冻结用例 `sha_mismatch_leaves_existing_binary_untouched_without_residue` 在只调 `verify_sha256` 后即断言「无 `.part` 残留」。
3. **回滚备份移到编排层（接受）**：`install_atomically` 保持「`rename` 直接覆盖、目标永不缺失」；`<exe>.old-<pid>` 由 `run_self_update` 在安装前 `copy` 生成、回验成功即删、失败即 `rename(.old→target)` 恢复。
   理由：C1（rename 覆盖）+ C5（临时文件必删）+ C6-5（回验失败恢复）三者只有在编排层持有备份时才同时成立。
4. **C3 产物件数勘误**：实际 3 件（`pony-tui_<ver>_linux-x86_64`、`.sha256`、`pony-cli-latest.json`），原写「4 files」为笔误。
5. **门禁数字勘误**：`--lib` 上限 = 基线 1348 + A 组 14 = **1362**（B 组 16 条落在 bin 目标，`--lib` 看不到）；bin 目标 = 54（含既有 38）。
6. **`-D warnings` 门禁的当前阻塞（外部，不属本写域）**：`crates/pony-agent-core/src/agent/team_history.rs`（另一路并行会话的协议空桩，10 处 `todo!("NotImplemented")`、参数未使用）在 HEAD 即产生 14 条 `unused variable` error。
   物理证据：`RUSTFLAGS="-D warnings" cargo check -p pony-agent-core --lib` 的 error 落点 100% 在该文件；本波两个文件的 warning 数为 **0**。处置：**不修**（不碰他人未完成实现），登记为外部阻塞，等该 owner 填实现后自然转绿；`node scripts/check-rust-test-lints.mjs` 同因失败。
