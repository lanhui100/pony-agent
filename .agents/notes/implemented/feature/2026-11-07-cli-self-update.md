# Agent Note: `pony update` —— CLI 自更新（GitHub Releases + 强制 sha256 + 原子替换）

Status: implemented

## Context

`pony`（终端 TUI）此前只有只读的 `/update`：查出新 tag 后打印「请 `git pull && npm run cli:build`」
或「桌面端请一键升级」，**没有人替用户完成升级**。而发布侧也不具备「可被升级」的条件：
`.github/workflows/release.yml` 只跑 Windows 桌面打包，linux 的 `pony-tui_<ver>_linux-x86_64` 是人工上传的
（`v0.1.182` 里那份是 core `0.1.155`，落后于 tag），`latest.json` 又只是桌面端 Tauri 更新清单（仅 windows 一项）。
另有一个陷阱：本仓是**两条独立版本线**（core `0.1.156` / tauri-app `0.1.183`），`pony --version` 报的是 core 版本，
按 tag 比较会得出错误的「已是最新」。

完整的决策正文（背景、备选方案、不可逆面与回滚口径、NFR 例外）见 ADR 0087：
[`docs/decisions/0087-cli-self-update.md`](../../../docs/decisions/0087-cli-self-update.md)。

## Decision

1. 新增 `crates/pony-agent-core/src/agent/updater.rs`（纯函数 + 网络编排分离）：清单解析、平台资产选择、
   `plan_update`（升级/已最新/降级/平台缺失）、流式下载 + sha256 校验、同目录原子安装（`install_atomically`）、
   `UpdateError` 与退出码映射。semver 解析/比较从 `bin/tui.rs` 迁入，成为 `/update` 与 `pony update` 的单一真值。
2. 命令面：`pony update [--check] [--force] [--pin <ver>] [--json]`，退出码 `0` 成功（含已最新）/ `1` 校验失败
   （含回验失败与 10s 回验超时）/ `2` 用法错误（含拒绝降级）/ `3` 网络·HTTP·清单·无平台资产 / `4` 安装失败；
   `/update` 保持只读（tag 口径，面向整体发布），文案追加一行「本机升级（CLI 二进制）：`pony update`」。
   两条命令不共享版本口径是有意的：一个回答「有没有新发布」，一个回答「CLI 能不能升」。
3. 发现只看 **release 里的 CLI 清单**（`pony-cli-latest.json`），版本以清单 `version` 为准、**绝不用 `tag_name`**；
   `--pin` 指 core 版本，按清单版本扫 release，不按 tag 拼 URL。
4. 完整性：sha256 必填（64 hex）+ 字节数校验；下载流式写盘、超 128MiB 或超预算即中止；校验失败**不触碰原二进制**。
5. 网络：`redirect::Policy::none()` 手动跟随 ≤5 跳、逐跳校验 scheme 与 host（`api.github.com` / `github.com` /
   `release-assets.githubusercontent.com` / `objects.githubusercontent.com`）；控制面 5s、制品下载独立预算 300s。
6. 安装：同目录 lock（`create_new` + `pid`/`started`；`started` 超 45 分钟判陈旧并接管一次）→ 下载到临时名 → `chmod 0755` → `rename` 覆盖 → 跑新二进制 `--version` 回验，
   不符或 10s 内不返回即回滚并退出 `1`；临时文件与 lock 在成功/失败路径都清理。Windows 上运行中 exe 不可替换 ⇒ 明确报
   `UnsupportedPlatform` 并指引桌面端安装包。
7. 发布侧同批补齐：`release.yml` 新增 `build-cli-linux`（`needs: build-and-release`），用**构建出来的二进制自报版本**
   做一致性断言（只比 core），产出 `pony-tui_<ver>_linux-x86_64` + `.sha256` + `pony-cli-latest.json` 并上传，
   发布后用 API 断言清单真的在 release 上。

## Alternatives considered

- **复用桌面端 `latest.json` + Tauri updater**：该清单只有 windows 一项且绑定安装器格式，CLI 拿不到 linux 二进制。
- **保留「只提示怎么升级」**：这就是要修的缺陷本身。
- **S3（`s3.local.ponyjob.top`）作主源**：该域在开发机不可解析（实测 `curl: (6)`），无法本地真验，且引入第二份真值。
  仅保留 `PONY_UPDATE_MANIFEST_URL` 作为镜像/离线口子（与 `PONY_UPDATE_ALLOW_INSECURE=1` 同时设置才放开 http/localhost）。
- **按 `tag_name` 发现/拼 URL**：实测 tag 与 core 版本已错位，会得出「已是最新」的假结论。
- **自动跟随重定向**：与仓库既有安全约定冲突（`agent/web_access.rs` 决策 8：禁止自动跟随、逐跳审计）。
- **`git pull && cargo build` 代替二进制替换**：要求用户有 Rust 工具链，耗时与失败面远大于下载 28MB 制品。

## Consequences

- 用户一条命令即可升级本机二进制；`--check` 只读预检、`--json` 可供脚本消费。
- **不可逆面**：写的是用户自己的可执行文件。逐字节回滚没有承诺——失败即不动、成功即覆盖、有 sha256 与回验两道闸、
  并发有 lock；要回到旧版本用 **`pony update --pin <旧 core 版本> --force`**（`--pin` 选目标、`--force` 放行降级，
  缺一不可；前提是该版本仍在最近 release 携带清单里），这条命令必须逐字进发布说明，不能当「已支持回滚」宣传。
- **NFR 例外**：制品下载对 `external_call_timeout_ms=3000` 是显式例外（28MB 不可能 3s 下完），预算 300s/上限 1800s/
  128MiB 上限/不重试，记在 `.dev-team/nfr-baseline-wave67.json` 的 `download_budget`；控制面仍 5s + 1 次重试。
- **可观测性**：新增一条出网路径与一个 CLI 子命令接口面，四维归属落 `docs/observability.yaml`；CLI 进程无内置 tracer，
  `trace_id` 按既有 `logging_legacy_exempt` 口径豁免并显式声明。
- **顺带发现的既有缺陷（本轮不改，登记）**：`pony` 的 `ObservabilityBootstrap` 在解析参数前就 `log_json` 打一条
  `otel::init` 警告，而该 logger 走 **stdout** ⇒ `pony --version` / `pony update --json` 的「机器可读输出」
  在「OTel 已启用但缺 token」的环境里会被一行 JSON 警告占位（v0.1.184 的 CI 首次实证：版本断言把警告行当成版本）。
  CI 侧已改为按版本号模式提取（fail-closed）；**产品侧的正确修法是把该 logger 的默认 sink 改成 stderr**，
  属跨模块行为变更（TUI 的 tui-runtime.log 抓取依赖它），另立条目处理。
- **残余风险**：CLI job 失败时该 release 没有 CLI 清单 ⇒ 客户端回落到更旧的含清单 release（不会装坏版本，属优雅降级）；
  当前只发布 `linux-x86_64`，其它平台会得到 `UnsupportedPlatform`/`MissingAsset` 的明确指引。
- 顺带修复：HEAD 上 `bin/tui.rs` 里 `41e13297` merge 残留的 `mod wave43_unit_tests` 重复粘贴（5 个测试各定义 2 次 ⇒ E0428，
  bin 测试目标在 HEAD 即无法编译），删除重复的第二份、语义零改动——否则本波的 CLI 侧测试根本无法编译。
