# Agent Note: `pony update` 同号不同内容谎报「已是最新」+ 发布侧清单版本递增门禁

Status: implemented

## Context

现场（2026-11-08 开发服务器）：本机装的是 `0.1.158` 的**本地补丁版**（含 ADR 0095 超时修复），
`pony update` 稳定回「当前已是最新版本（v0.1.158）」。`gh api` 核查后确认：v0.1.185 的清单
`version=0.1.158` 是正确的（= 该 tag 的 core 版本），core 版本线确实每版递增
（v0.1.182→0.1.155 / v0.1.184→0.1.157 / v0.1.185→0.1.158）。所以问题不在「版本线不动」，
而在两处：**客户端把版本号相等当成字节相同**，以及**发布侧允许清单版本不递增的发布**。

决策正文见 ADR 0097：[`docs/decisions/0097-cli-update-same-version-content-mismatch.md`](../../../../docs/decisions/0097-cli-update-same-version-content-mismatch.md)。

## Decision

1. `crates/pony-agent-core/src/agent/updater.rs`
   - 新增 `hash_file_sha256`（只读流式哈希）与 `installed_matches_artifact`（只有「读到且摘要逐字相等」
     才算一致，缺失/不可读一律不一致 = fail-closed）；`verify_sha256` 改为复用前者，
     删除下载件的口径只留在 `verify_sha256`，**绝不套用到用户正在运行的二进制**。
   - `UpdateOutcome` 新增 `content_mismatch`；`is_same && !force` 分支先比对内容哈希：不一致则
     `updated=false, content_mismatch=true`，**不下载、不覆盖**，退出码仍为 0。
2. `crates/pony-agent-core/src/bin/tui.rs`
   - 收据渲染抽成纯函数 `format_update_outcome(outcome, json)`（三种状态唯一出文案处）；
     `--json` 增字段 `content_mismatch`；人可读文案给出 `pony update --force` 出路；
     `pony update --help` 的 `--force` 行补「同号但内容不同也必须给 --force」。
3. 发布侧 `scripts/check-cli-manifest-advances.mjs`（新）
   - `evaluateCliAdvance(current, previous)`：严格递增才通过；相等/回退/任一版本非法一律失败（fail-closed）；
     `previous` 为 `none` 时放行（首个清单）。退出码 0/1/2。
4. `.github/workflows/release.yml` 的 `build-cli-linux`
   - 新增步骤 `Assert CLI version advances over previous release manifest`：用与客户端发现同一口径
     （`published && !draft`，最多 20 条，第一个含 `pony-cli-latest.json` 的 release）取上一条清单版本，
     再跑上述脚本；放在「生成清单」之前，失败即 job 红。
   - 该 job 显式 `actions/setup-node@v4 (22)`：不赌 ubuntu runner 预装 node。
5. 测试：`updater::tests` +1（只读且 fail-closed），`cli_self_update_tests` +2（同号不同内容不下载不覆盖、
   渲染与收据字段），`tests/cli-manifest-advances.spec.ts` +5（vitest）。

## Alternatives considered

- **同号不一致时自动重装**：会静默销毁用户本机补丁版且无回滚通道 —— 落选（`--force` 才是显式授权）。
- **把同号不一致报成错误面（非 0 退出）**：本机是定制件不是失败，会破坏脚本化用法与 C2 退出码口径 —— 落选。
- **把 sha 编进 `pony --version`**：波及 `/update` 与 CI 的 core 版本链断言，远超本缺陷 —— 落选。
- **只在客户端加「内容不一致」报告，不加发布侧门禁**：治不了「只 bump tauri ⇒ 清单版本恒定 ⇒ 所有机器永远
  已是最新」这种客户端不可观测的失败 —— 落选。完整比较见 ADR 0097。
