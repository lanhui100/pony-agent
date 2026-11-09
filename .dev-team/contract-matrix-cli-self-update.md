# 契约矩阵：wave-67 `pony update` CLI 自更新（C 级）

- 用户裁决（2026-11-07）：① 更新对象 = **`pony`（TUI 二进制）**，命令 `pony update`；② **同批扩发布流水线**（ubuntu job 构建 + sha256 + 平台命名上传）；③ 更新源 = **GitHub Releases + 强制 sha256**，无 `--force` 不允许降级。
- 基线 commit：`67e656c9`（`cargo test -p pony-agent-core --lib` = 1348 passed；收据 `.dev-team/regression-baseline-wave67.json`）
- 现状事实（侦察已验）：`pony` 有 `/update` 斜杠命令但**只检查不安装**（`tui.rs:2051 perform_update_check`，5s 超时，GitHub latest API + S3 回退）；`ponycli` 无版本号无更新；`release.yml` 只构建 Windows 桌面包（`runs-on: windows-latest`），**无任何 job 产出 linux CLI 二进制**；`v0.1.182` 里的 `pony-tui_0.1.155_linux-x86_64(.sha256)` 是人工上传且落后 tag；`latest.json` 是桌面 Tauri 更新清单（仅 windows，minisign），不可复用。
- 依赖已就绪（无需新增 crate）：`reqwest 0.12`(json,native-tls,charset,stream)、`sha2 0.10`、`serde_json`、`tokio`(rt,time,net,sync)。

## C1 模块与 API：新增 `crates/pony-agent-core/src/agent/updater.rs`

平台标识（**客户端协议词表**唯一真值，`#[non_exhaustive]` 风格字符串常量）：
`linux-x86_64` / `linux-aarch64` / `macos-x86_64` / `macos-aarch64` / `windows-x86_64`
〔口径修订见 §AMEND-5：本清单是「客户端认识什么」，**不是**「发布侧发什么」；后者唯一真值源
`scripts/cli-platforms.mjs`，两侧对齐由 `npm run contract:cli` 机器校验〕

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
- **同号不同内容口径（2026-11-08，[ADR 0097](../docs/decisions/0097-cli-update-same-version-content-mismatch.md)）**：`manifest.version` 与当前版本**相等**时，必须再比对「本机目标文件的 sha256」与清单声明（`installed_matches_artifact`：只读、只有「读到且逐字相等」才算一致，缺失/不可读一律不一致 = fail-closed）：
  - 一致 ⇒ `当前已是最新版本（v<ver>）`，退出码 0；
  - 不一致（本地构建/补丁版、被替换过、或目标缺失）⇒ **不下载、不覆盖**，输出「本机 v<ver> 与官方发布件内容不一致…；如需覆盖为官方发布件：`pony update --force`」，退出码仍为 0；
  - `--json` 收据新增字段 `content_mismatch: bool`，判定必须读该字段，**不得**靠中文字面量。
  「版本号相同 ⇒ 同一份二进制」是被明令禁止的假设（实测：本地补丁版 0.1.158 与官方 0.1.158 同号，旧实现谎报「已是最新」）。
- **退出码**：`0` 成功（含「已是最新」）；`1` 校验失败（sha256/size/回验不符）；`2` 用法错误；`3` 网络/HTTP/清单失败；`4` 安装失败（不可写/rename 失败）。与 `tui.rs` 既有 `exit` 口径共存，不改变其它子命令行为。
- `/update` 斜杠命令：**保持既有 tag 口径的只读检查实现不变**（`perform_update_check` 零改动），仅升级指引追加一行「本机升级（CLI 二进制）：`pony update`」（**不自动替换用户的二进制**）。〔本条已被收口时裁决取代：见 §AMEND-4 第 5 条——`/update` 答「有没有新发布」，`pony update` 答「CLI 能否升」，强行复用会让其中一个说假话。〕

## C3 发布侧：`.github/workflows/release.yml` 新增 job

〔本节步骤 1–6（单 job、单平台、job 内联生成清单）已被 §AMEND-5 修订：改为 `build-cli` 平台矩阵 +
`publish-cli` 单点装配。步骤 7 的版本递增门禁不变，位置随装配步骤一起前移到 `publish-cli`。〕

新增 `build-cli-linux`（`runs-on: ubuntu-latest`，**`needs: build-and-release`** —— 桌面 job 内部完成 publish，故 CLI job 在其后 `gh release upload --clobber`；若 CLI job 失败，发布已发生但缺 CLI 清单，客户端发现逻辑会回落到更旧的含清单 release，属**优雅降级**而非坏版本，需在 job 里显式打印该风险），步骤契约：
1. `cargo build --release --bin pony`；
2. `ver=$(./target/release/pony --version)` 解析出的版本必须 == `crates/pony-agent-core/Cargo.toml` 的 `version`（**core 版本链**，fail-closed）。**不要**与 `package.json` 比较：本仓是**两条独立版本线**（实测 `core=0.1.156`、`tauri/app=0.1.183`，`npm run version:check` 明确输出「Core version ok / Tauri version ok (4 places)」），CLI 二进制只承载 core 版本；
3. 产物命名 `pony-tui_<ver>_linux-x86_64` + `pony-tui_<ver>_linux-x86_64.sha256`（`sha256sum` 输出格式 `<hex>  <filename>`）；
4. 生成 `pony-cli-latest.json`：`{"schema":"pony-cli-manifest","version":"<ver>","generated_at":"<RFC3339 UTC>","platforms":{"linux-x86_64":{"asset":"pony-tui_<ver>_linux-x86_64","sha256":"<hex>","size":<bytes>}}}`；
5. 在 release 已发布后 `gh release upload "$tag" <4 files> --clobber`；
6. **收尾断言**：`gh api repos/$repo/releases/tags/$tag` 中必须出现 `pony-cli-latest.json` 且其 `version` == `$ver`，否则 job 失败。
7. **清单版本递增门禁（2026-11-08，[ADR 0097](../docs/decisions/0097-cli-update-same-version-content-mismatch.md)）**：在**生成清单之前**，用与客户端发现同一口径回看（`published && !draft`，最多 20 条，第一个含 `pony-cli-latest.json` 的 release）取上游 `version`，跑 `node scripts/check-cli-manifest-advances.mjs --current "$PONY_CLI_VERSION" --previous "$prev"`；未**严格递增**（相等或回退）⇒ job 失败。理由：清单 `version` = 该 tag 的 core 版本，而 `release.mjs --target tauri` 这类只 bump tauri 的发版会让清单版本恒定 ⇒ 所有客户端永远判「已是最新」，本次 CLI 构建永远不会被下载，且**客户端侧没有任何报错可看**。上游无历史清单（首个清单）时放行。该 job 需 `actions/setup-node@v4`（node 22），不赌 runner 预装。
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
| 控制面超时 | connect 5s / **空闲读 10s** / 总预算 30s（2026-11-08 由 [ADR 0095](../docs/decisions/0095-cli-update-control-plane-timeout.md) 修订；旧口径 connect 5s / total 5s 会把代理链路上 175KB 的 releases 列表判成故障） |
| 下载预算 | connect 10s、总预算 300s（`PONY_UPDATE_DOWNLOAD_BUDGET_SECS` 1..=1800，非法回退 300）；**这是对 `external_call_timeout_ms=3000` 的显式例外**，理由：28MB 级二进制不可能 3s 内下完；例外只作用于「制品下载」这一条路径，控制面仍受 30s 总预算约束 |
| 体积上限 | `MAX_ARTIFACT_BYTES = 128MiB`；超限即 `TooLarge` 并删临时文件 |
| 重试 | 清单/列表请求最多 2 次尝试（1 次重试，退避 500ms），**覆盖网络错误、body 读取失败与 5xx**（4xx 直接判 HTTP 状态，不重试）；**制品下载不重试**（避免半包累积），失败即整轮失败 |
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

- 单测（真断言，禁 skip）：semver 解析/比较边界（`v1.2.3`/`1.2.3`/`1.2`/`1.2.3.4`/空/非数字）；manifest 解析（合法、schema 错、sha 非 64hex、version 非法、空 platforms）；`select_platform_asset`/`plan_update`（有更新 / 已最新 / 降级拒绝 / force 降级 / 平台缺失错误串含已有平台 / pin 语义）；`verify_sha256`（对 / 错一字节 / 文件不存在）；`installed_matches_artifact`（一致 / 同号不同字节 / 大小写摘要 / 文件缺失，且**只读不删**——删除口径只属于下载件）。
- 同号回归（ADR 0097）：`cli_self_update_tests::update_same_version_with_different_bytes_reports_mismatch_without_overwriting`（不下载、不覆盖、退出码 0）与 `same_version_receipt_distinguishes_content_mismatch_from_up_to_date`（渲染与 `--json` 的 `content_mismatch`）；发布侧门禁 `tests/cli-manifest-advances.spec.ts`（`npm run test:unit` 覆盖：递增 / 相等 / 回退 / 上游非法 / 首个清单）。
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

## AMEND-4（Lead 裁决，三路审查回执后）

审查结论：L3-A PASS(3 ADVISORY) / L3-B FAIL(2 BLOCKING + 4 ADVISORY) / L3-P FAIL(2 BLOCKING + 4 ADVISORY)。逐条裁决：

### 必修（已派单，Lead 亲验 BLOCKING）
1. **回归**：semver 迁移撤掉 `tui::parse_semver_triple` / `tui::is_newer_semver` 的 pub 面，导致既有验收 target
   `crates/pony-agent-core/tests/slash_commands_acceptance.rs:795-802`（T6 `/update` 契约）不可编译
   （Lead 实测 `--no-run` EXIT=101，E0425×3 + E0603×4）。⇒ 恢复 `pub use` 再导出，目标必须回绿。
2. **回验无超时**：`verify_installed` 用 `Command::output()` 零 deadline（L3-P 实测 25s 挂起、exit 124、锁残留）⇒ 加 10s deadline（`try_wait` 轮询 + kill），超时回滚并 `1` 退出。
3. **锁无陈旧判定**：kill 后 lock 永久残留、后续每次 `exit 4` 误报并发 ⇒ lock 写 `pid`/`started`，`started` 超 45min（1.5× 最大预算）判陈旧并**接管一次**；未过期不删。
4. **回滚口径不可执行**：`--pin` 不隐含 force，`--pin <旧版本>` 会被降级闸拦（L3-P 实测 exit 2）⇒ **保留安全闸**，把 ADR/note/错误文案统一为 `--pin <ver> --force`。

### 口径修正（Lead 裁决，非代码缺陷）
5. **`/update` 不复用 `pony update` 的版本口径**：`/update` 面向整体发布（tag 口径），`pony update` 面向 CLI 二进制（core 口径）；强行合并会让一个说假话。改为「保持只读 + 文案追加一行『本机升级（CLI 二进制）：pony update』」，ADR/note/C2 已同步。
6. **退出码表补登记**：`UnsupportedPlatform` 归 `3`（平台无可用资产），不改代码，改文档。
7. **`build-cli-linux` 的 `-D warnings`**：该 job 与 verify 作业同用 `actions-rust-lang/setup-rust-toolchain@v1`，该 action 默认注入 `RUSTFLAGS=-D warnings`（见 `scripts/check-rust-test-lints.mjs` 头注），故 advisory「未设 deny-warnings」**不成立**，不新增 env。
8. **超限用例区分力**：L3-B 变异证明 `update_rejects_oversized_artifact_without_writing` 对 `MAX_ARTIFACT_BYTES` 分支零区分力 ⇒ 由 Test Agent 补 `error.code == "too_large"` 断言。
9. **不入本波（登记后续条）**：`<exe>.part` 无 RAII 守卫（kill 后残留半包，危害低、下次覆盖）；`pony update --help`/`.env.example` 缺 6 个 `PONY_UPDATE_*` 自检入口（当前走 `Invalid` 退出 2 并给出取值口径）。

## AMEND-5（Lead 裁决，2026-11-09：发布平台与承诺对齐 — [ADR 0108](../docs/decisions/0108-cli-release-platform-matrix.md)）

1. **C1 的 5 项标识定性为「客户端协议词表」，不再是「发布侧与客户端唯一真值」**。原句把两个概念写成一处，
   导致的现场故障是：客户端认 5 个平台、`release.yml` 只发 `linux-x86_64`，macOS/ARM 用户每次
   `pony update` 都吃 `UnsupportedPlatform`，且没有任何命令会红。发布集合的真值源改为
   `scripts/cli-platforms.mjs`（本轮 `linux-x86_64` 必需 + `linux-aarch64` / `macos-aarch64` 尽力；
   `windows-x86_64`、`macos-x86_64` 登记为刻意不发布并附结构性理由）。
2. **C3 改两段式**：`build-cli`（`strategy.matrix.include`，`continue-on-error: ${{ !matrix.required }}`，
   只做构建 + core 版本链断言 + 上传 workflow artifact）与 `publish-cli`（单点下载全部 artifact →
   `node scripts/build-cli-manifest.mjs` 装配清单与 `<asset>.sha256` 旁证 → 一次 `gh release upload --clobber` →
   收尾断言）。清单装配自此不再长在 YAML 内联 python 里，摘要口径不依赖 runner 的 `sha256sum`/`shasum` 差异；
   多平台各自上传同一份清单的 `--clobber` 竞态也一并消灭。
3. **新增机器门禁**：`npm run contract:cli`（`node scripts/check-cli-release-contract.mjs`），插在 `npm run verify`
   的 `contract:check` 之后。校验 workflow 矩阵 ↔ 真值源 ↔ 客户端 `current_platform()` 词表 ↔ 冻结清单资产名四处；
   配套回归用例 `tests/cli-release-contract.spec.ts`（16 条，含变异用例）。
4. **C3-3 产物命名扩为 `pony-tui_<ver>_<platform>`**，`platform` 取发布集合；清单 `platforms` 的每个 key
   都必须在 release 上真的存在同名资产（收尾断言逐平台校验，防「清单有、资产无」）。
5. **清单资产名冻结（P1 裁定）**：`pony-cli-latest.json` **不改名** —— 该字面量已硬编码在存量已发布二进制内，
   改名 = 老客户端静默断更。确有需求时必须同批上传新旧两份清单并保留旧名到覆盖周期结束，且走新 ADR。
   术语口径：`cli` 指**命令行形态**（交付物是 TUI 二进制 `pony`），与 `ponycli` 那个 bin 无关。
6. **C3 尾句「`ponycli` 本次不发布」升格为长期事实**，家载体 `crates/pony-agent-core/README.md`
   「两个二进制入口」表（含 stdout 契约差异与「不拆 crate：cargo 无 per-`[[bin]]` 依赖」一句）。
7. **验收边界如实登记（本机验不到、靠下一次真实发版）**：`ubuntu-24.04-arm` 与 `macos-latest` 对本仓依赖树的
   可编译性、以及 `-D warnings` 下 macOS 是否引入平台专属告警，只能在下一个 `v*` tag 的 run 上验收。
   失败表现是显式且局部的：该平台 job 红但不阻断发版，`publish-cli` 在 stderr 打印缺失平台，
   用户侧得到 `UnsupportedPlatform` + 清单已有平台清单；必需平台失败则整轮红。
