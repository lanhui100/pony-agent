# 0087 CLI 自更新：`pony update` 走 GitHub Releases + 强制 sha256 + POSIX 原子替换

Status: implemented

## 背景

`pony`（终端 TUI 二进制的入口）此前只有**只读**的 `/update` 斜杠命令：它调 GitHub
`releases/latest` 拿 tag，比较版本后打印「源码开发环境请 `git pull && npm run cli:build`、
桌面端请在设置里一键升级」（`crates/pony-agent-core/src/bin/tui.rs` 的 `perform_update_check`）。
即：**检查有了，升级没有人替用户做**。用户拿到的是一句提示，实际升级仍靠手工构建或换装桌面端。

同时，发布侧的真实状态与「能升级」这件事并不匹配：

- `.github/workflows/release.yml` 只有 `windows-latest` 一个 job，产出 Windows NSIS 安装器 + Tauri
  签名更新清单 `latest.json`（该清单只有 `windows-x86_64` 一个平台项，用途是**桌面端**一键升级）；
- GitHub Release `v0.1.182` 里确实存在 `pony-tui_0.1.155_linux-x86_64` 与它的 `.sha256`，但**仓库内
  没有任何 workflow 产出它们**——那是人工上传的，且文件名里的版本（core 版本 0.1.155）落后于 tag
  （桌面版本 0.1.182）。

于是「`pony update`」这个需求实际包含两件事：**客户端会装**，以及**发布侧稳定地有得装**。

## 决策

1. **命令面**：`pony update [--check] [--force] [--pin <ver>] [--json]`。
   默认「检查 → 下载 → 校验 → 安装 → 回验」；`--check` 只检查；`--force` 允许降级/同版本重装；
   `--json` 让 stdout 只输出一行 JSON 收据（供脚本消费）。退出码：`0` 成功（含「已是最新」）、
   `1` 校验失败（含回验失败/回验超时）、`2` 用法错误（含拒绝降级）、`3` 网络/HTTP/清单/无可用平台资产失败、
   `4` 安装失败。
   `/update` 斜杠命令保持**只读**语义（它面向「整体发布/桌面端」用 tag 口径），文案追加一行
   「本机升级（CLI 二进制）：`pony update`」——不在交互界面里悄悄替换用户的可执行文件。
   两条命令**刻意不共享版本口径**：`/update` 回答「有没有新发布」，`pony update` 回答「CLI 二进制能不能升」
   （core 版本线）；强行合并会让其中一个说假话。
2. **更新源唯一**：GitHub Releases（仓库公开，可匿名访问）。发现流程 =
   `GET /repos/{owner}/{repo}/releases?per_page=20` → 过滤 draft/prerelease → 按 `published_at` 降序取
   **第一个含 `pony-cli-latest.json` 资产**的 release。**版本以清单里的 `version` 为准，绝不使用 `tag_name`**
   ——上面那组「tag 0.1.182 / CLI 0.1.155」的实测错位就是这条规则的由来。
3. **清单由 CI 生成，不由人手写**：`pony-cli-latest.json`
   = `{"schema":"pony-cli-manifest","version":"<core 版本>","generated_at":...,"platforms":{"linux-x86_64":{asset,sha256,size}}}`。
   新增 ubuntu job：`cargo build --release --bin pony` → 用**构建出来的二进制自己报的版本**做一致性校验
   （必须等于 `crates/pony-agent-core/Cargo.toml` 的 `version`）→ 产出 `pony-tui_<ver>_linux-x86_64` +
   `.sha256` + 清单 → `gh release upload --clobber` → 收尾用 API 断言清单真的在 release 上。
   本仓是**两条独立版本线**（core / tauri-app，实测 0.1.156 vs 0.1.183，`version:check` 分两条各自 PASS），
   所以 CLI 只认 core 版本，`--pin <ver>` 也指 core 版本（按清单版本扫 release，而不是按 tag）。
4. **完整性只信 sha256**：清单里的 `sha256` 必填，长度必须 64 位 hex；下载走流式写盘并在结束时校验
   sha256 + 字节数。校验失败 ⇒ 立即中止且**不触碰原二进制**。没有「跳过校验」开关——宁可失败也不装来路不明的字节。
5. **重定向手动逐跳**：仓库既有约定是禁止 reqwest 自动跟随重定向（`agent/web_access.rs` 决策 8，≤5 跳、逐跳校验）。
   GitHub 资产实测是 `github.com/...` → **302** → `release-assets.githubusercontent.com/...`（签名 SAS URL）→ 200，
   因此 updater 用 `redirect::Policy::none()` 手动跟随 ≤5 跳，每跳校验 scheme + host，最终落点也必须在
   白名单（`api.github.com` / `github.com` / `release-assets.githubusercontent.com` / `objects.githubusercontent.com`）内。
6. **替换是原子的、可观测的**：下载到目标同目录的临时文件 → `chmod 0o755` → POSIX `rename` 覆盖（目标永不缺失）
   → 回验：跑新二进制 `--version`（**10 秒 deadline**，超时即 kill 并回滚，否则不响应的制品会让命令永久挂起）
   必须等于清单版本，否则恢复旧文件并以 `1` 退出。同目录 `<exe>.update.lock`（`create_new` + `pid`/`started`
   两行）防两个 `pony update` 并发互踩；`started` 超过 45 分钟（1.5× 最大下载预算）即判陈旧锁并接管一次，
   其余情形绝不删除他人锁。临时文件与锁在成功/失败路径都必须清理；信号强杀路径无法清理锁，靠上述陈旧判定自愈。
7. **不支持的环境明确拒绝**：Windows 上运行中的 `.exe` 无法被替换 ⇒ `UnsupportedPlatform`，指引用户用桌面端
   安装包（`Pony.Agent_<ver>_x64-setup.exe`）；目标目录不可写 ⇒ `NotWritable` 并给出 `sudo` / 换目录提示，
   **不静默改写到用户目录**。
8. **测试/镜像逃生舱 fail-closed**：只有同时设置 `PONY_UPDATE_MANIFEST_URL` 与 `PONY_UPDATE_ALLOW_INSECURE=1`
   时才放开 `http://` 与 localhost（供 e2e/镜像）；只设其一仍按生产规则拒绝。安装目标也可被
   `PONY_UPDATE_INSTALL_PATH` 覆盖，仅测试/CI 使用。

## 备选方案

- **复用桌面端 `latest.json` + Tauri updater 插件**：否决。该清单只有 `windows-x86_64` 一项、且格式绑定
  桌面端安装器（minisign 签名 + NSIS 包），CLI 拿不到 linux 二进制；把 CLI 塞进去等于让两条交付形态共享
  一份不该共享的契约。
- **沿用 `/update` 的「只提示怎么升级」**：否决。这正是本次要解决的问题本身；用户要的是命令自己完成升级。
- **自建 S3（`s3.local.ponyjob.top`）镜像作为主源**：否决为主源。该域在开发机不可解析（实测 curl 6），
  无法在本机做真实验收；且引入第二份真值（谁先发、谁后发）。保留 `PONY_UPDATE_MANIFEST_URL` 作为镜像/离线口子。
- **按 `tag_name` 发现版本、按 tag 拼下载 URL**：否决。实测 tag 0.1.182 与 CLI core 版本 0.1.155 已经错位，
  按 tag 比较会得出「已是最新」的假结论。
- **用 `git pull && cargo build` 取代二进制替换**：否决。要求用户机器上有 Rust 工具链，且升级耗时/失败面远大于下载
  一个 28MB 制品；源码路径作为开发环境提示保留在 `/update` 文案里。
- **自动跟随重定向（reqwest 默认）**：否决。与仓库既有安全约定冲突（SKILL/ADR 已锁定手动逐跳），且会把
  「跳去哪」变成不可审计的黑箱。
- **Windows 也做自更新（下载后延迟替换脚本）**：本轮否决。需要额外的辅助进程/重启编排，收益只覆盖一个当前
  不发布 CLI 产物的平台；先把 Linux 路径做对。

## 影响

- **用户可见**：`pony update` 一条命令即可把本机二进制升到最新 core 版本；`--check` 提供只读预检；`--json`
  给脚本化升级入口。`/update` 文案从「请自行构建」变为「本机升级：`pony update`」。
- **不可逆面（本变更最需要留意的部分）**：写入的是**用户自己的可执行文件**。约束：失败即不动、成功即覆盖、
  覆盖前有 sha256 与回验两道闸、并发有 lock、临时文件必清。**没有自动备份**：升级不可回滚到旧的**字节**，
  但可用 **`pony update --pin <旧版本> --force`** 装回指定版本（`--pin` 是**目标选择**、`--force` 是**放行降级**，
  两者必须同时给；前提是该版本仍在最近的 release 里带清单）——这条命令必须逐字写进发布说明，
  不把「可回滚」当成已承诺能力，也不写成半条命令。
- **CI 面**：`release.yml` 新增 ubuntu job（`needs: build-and-release`）。桌面 job 先发布、CLI job 后补传资产：
  若 CLI job 失败，会出现「本次 release 没有 CLI 清单」，此时客户端会自然落到更旧的含清单 release（不会装坏版本），
  属**优雅降级**而非隐蔽故障——job 里显式打印该风险。
- **NFR**：制品下载对 `external_call_timeout_ms=3000` 是**显式例外**（28MB 不可能 3s 下完）：控制面仍 5s +
  1 次重试，制品下载单独预算 300s（上限 1800s，非法回退 300）、连接 10s、体积上限 128MiB、**不重试**
  （避免半包累积），落在 `.dev-team/nfr-baseline-wave67.json` 的 `download_budget`。
  *（事实指针：控制面的「5s 总预算」已于 2026-11-08 由 [ADR 0095](0095-cli-update-control-plane-timeout.md)
  修订为 connect 5s / 空闲读 10s / 总预算 30s，并把 body 读取纳入重试；本句保留为制定时的记录，不作现行口径。）*
- **可观测性**：新增一条出网路径（发现 + 下载）与一个新的 CLI 子命令接口面，四维归属见 `docs/observability.yaml`
  的本条登记（含 `trace_id` 缺省豁免的显式声明）。阶段日志是 **stderr 上的单行 `[pony-update] phase=… status=…`**
   （六个阶段 ok/fail 都有），不写工作区日志文件；`--json` 时 stdout 只有一行收据，日志绝不污染 stdout。
- **验收**：红相在空桩上真红（`unimplemented!()` panic / 断言失败），绿相 `cargo test -p pony-agent-core --lib`
  全绿；另有本机 e2e（本地 mock 清单 → 下载 → 校验 → 替换 → 回验）与真实网络 `pony update --check --json`
  两条物理收据。
