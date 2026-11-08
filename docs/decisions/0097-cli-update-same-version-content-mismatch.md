# ADR 0097：`pony update` 同号不同内容不得谎报「已是最新」，发布侧清单版本必须严格递增

Status: implemented

## 背景

2026-11-08 现场（开发服务器）：本机 `~/.local/bin/pony` 是 `0.1.158`（本地构建的补丁版，含 ADR 0095 的超时修复），
`pony update` 稳定回「当前已是最新版本（v0.1.158）」。核查发布侧事实（`gh api` 实测）：

| release | tag 打包的 core 版本 | 清单 `version` | CLI 资产 |
|---|---|---|---|
| v0.1.181 | — | 无清单 | 无 CLI 资产 |
| v0.1.182 | 0.1.155 | **无清单** | `pony-tui_0.1.155_linux-x86_64` |
| v0.1.184 | 0.1.157 | 0.1.157 | `pony-tui_0.1.157_linux-x86_64` |
| v0.1.185 | 0.1.158 | 0.1.158 | `pony-tui_0.1.158_linux-x86_64`（sha256 `cd7bc460…`，29865120 B） |

由此**否证**两条此前的猜测性判断：① core 版本线并非「不随 release 递增」（每版都 patch+1）；
② 清单 `version=0.1.158` 对 v0.1.185 而言是**正确**的。真正的缺陷是另外两条：

1. **客户端把「版本号相同」当成「字节相同」**：`plan_update` 用 semver 相等就判 `is_same`，
   `run_self_update` 直接回「已是最新」。于是**任何同号的非官方构建（本地 `cargo build` 装的补丁版、
   被手工替换过的二进制）永远拿不到官方同号件，也无从发现自己不是官方件**——这正是本次现场的症状：
   本地补丁版 0.1.158 与官方 0.1.158 同号，`pony update` 唯一的真话应该是「内容不一致」。
2. **发布侧允许「清单版本不递增」的发布**：`scripts/release.mjs --target tauri` /
   `bump-version.ps1 -Target tauri` 都合法，只 bump tauri 时清单 `version`（= core 版本）与上一版相同。
   客户端发现口径是「版本以清单为准」，于是所有机器永远判「已是最新」，本次 CLI 构建**永远不会被下载**，
   且客户端侧没有任何报错可看。这是无法在客户端侧观测的失败，只能靠发布侧 fail-closed 拦。

历史的 v0.1.182 「有 CLI 资产、无清单」是同一类失败的另一半形态：客户端发现按「第一个含清单的 release」
回落到更旧版本，静默、不报错（该 tag 的收尾断言是之后才加的）。

## 候选方案

- **A（采纳，客户端）同号时比对内容哈希，不一致就如实报告 + 给 `--force` 出路，不自动覆盖。**
  `installed_matches_artifact` 只读哈希（复用 `hash_file_sha256`，不删除任何文件），只有「读到且摘要逐字相等」
  才判一致；缺失/不可读一律判不一致（fail-closed）。选它的理由：既不再说谎（旧行为），也不自作主张覆盖用户
  本机定制件（`--force` 才是显式授权）。
- **B（落选）同号不一致时直接自动重装。** 语义上「update 就是对齐官方件」，但会在用户不知情时销毁其本机补丁
  （本次现场就是一个正在用的补丁版），且没有回滚通道——代价远大于多敲一次 `--force`。
- **C（落选）同号不一致时报成错误面（非 0 退出）。** 「本机是定制件」不是失败，退出码非 0 会破坏脚本化用法
  （契约 C2 的 0/1/2/3/4 口径也不该为它新增码值）。
- **D（落选）改用运行中二进制的 sha256 作版本标识（如 `0.1.158+sha`）。** 会改写 `pony --version` 与全部
  版本比对口径（含 `/update` 与 CI 的 core 版本链断言），波及面远超本缺陷。
- **E（采纳，发布侧）新增清单版本严格递增门禁（脚本 + CI 步骤，fail-closed）。**
  纯函数 `evaluateCliAdvance(current, previous)` 由 vitest 覆盖；CI 用与客户端发现同一口径回看上一条含清单的
  release，取不到（首个清单）则放行。

## 决策

1. `updater.rs`：新增只读 `hash_file_sha256`（`verify_sha256` 改为复用它，删除口径只保留给下载件）、
   `installed_matches_artifact`（只读、fail-closed）；`UpdateOutcome` 新增 `content_mismatch`；
   `is_same && !force` 分支改为先比对内容哈希，不一致则 `updated=false, content_mismatch=true` 且**不下载、不覆盖**。
2. `bin/tui.rs`：成功收据渲染抽成纯函数 `format_update_outcome`（三种状态唯一出文案处），
   `--json` 收据新增 `content_mismatch` 字段；人可读文案给出 `pony update --force` 出路。
3. 发布侧：新增 `scripts/check-cli-manifest-advances.mjs`（0 通过 / 1 未递增或版本非法 / 2 用法错误），
   `.github/workflows/release.yml` 的 `build-cli-linux` 在生成清单**之前**执行该门禁；
   该 job 显式 `actions/setup-node@v4`（不赌 runner 预装 node）。

## Alternatives considered

见「候选方案」B / C / D 与 A / E 的取舍理由（本节按规范显式命名，内容不重复）。

## 影响

- 契约矩阵 `.dev-team/contract-matrix-cli-self-update.md`：C2 补「同号 + 内容不一致」口径与新收据字段，
  C3 补递增门禁与退出码，C4 的发现顺序不变（只新增发布侧校验）。
- 机械收口：`agent::updater::tests::installed_matches_artifact_is_read_only_and_fails_closed`；
  `bin/tui` 的 `cli_self_update_tests::update_same_version_with_different_bytes_reports_mismatch_without_overwriting`
  与 `same_version_receipt_distinguishes_content_mismatch_from_up_to_date`；
  `tests/cli-manifest-advances.spec.ts`（vitest）。
- 阶段日志（`[pony-update] phase=… status=…`）的格式、出口与六个阶段名**不变**；不一致状态只经
  `--json` 收据字段与人可读文案表达（observability.yaml 同提交登记）。
- 退出码口径不变：同号不同内容仍是成功态（0）。
- 不新增出网路径、不新增依赖（node 脚本用 runner 已有 node + 仓库既有 `gh`）。
