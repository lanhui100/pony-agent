# Agent Note: CLI 发布平台矩阵与清单单点装配

Status: implemented

配套决策：[ADR 0108](../../../docs/decisions/0108-cli-release-platform-matrix.md)（接替
[ADR 0087](../../../docs/decisions/0087-cli-self-update.md) 的 C3 发布侧形态，客户端决定不变）；
契约修订：`.dev-team/contract-matrix-cli-self-update.md` §AMEND-5；同类门禁先例：
[ADR 0097](../../../docs/decisions/0097-cli-update-same-version-content-mismatch.md) 的清单递增闸。

## Decision

1. 发布平台矩阵唯一真值源 `scripts/cli-platforms.mjs`：`linux-x86_64`（必需）/ `linux-aarch64` /
   `macos-aarch64`（尽力）；`windows-x86_64`、`macos-x86_64` 登记在 `NOT_PUBLISHED_PLATFORMS` 并写理由。
2. `.github/workflows/release.yml`：`build-cli-linux` → `build-cli`（平台矩阵，
   `continue-on-error: ${{ !matrix.required }}`）+ `publish-cli`（下载 artifact → 装配清单 → 单点上传 → 断言）。
3. `scripts/build-cli-manifest.mjs` 取代 job 内联 python：清单本体 + `<asset>.sha256` 旁证由同一实现产出
   （流式 sha256、必需平台缺制品即非零退出、资产名/版本/hex/size 全部 fail-closed）。
4. `scripts/check-cli-release-contract.mjs`（`npm run contract:cli`，已插进 `npm run verify`）对齐
   workflow ↔ 真值源 ↔ 客户端词表 ↔ 冻结资产名；回归用例 `tests/cli-release-contract.spec.ts`（16 条）。
5. 文档收口：`updater.rs::current_platform` 注释区分「协议词表 vs 发布集合」；
   `crates/pony-agent-core/README.md` 增「两个二进制入口」表（`ponycli` 不出制品成为成文事实）。

## 为什么

契约矩阵 C1 把 5 个平台标识写成「发布侧与客户端唯一真值」，而 C3/`release.yml` 只产出
`linux-x86_64`。两侧各自"合规"，缺口落在用户身上：macOS / ARM 机器执行 `pony update` 必然
`UnsupportedPlatform`，且**没有任何命令会红**——这类"承诺与交付不一致"只有变成机器对齐才不会再漂。
同时，清单装配长在 YAML 内联 python 里意味着本机无法验证，而矩阵化后各 job 自写清单会造成
同一份 `pony-cli-latest.json` 互相 `--clobber`（最终只剩最后完成的平台），比现状更糟。

## Alternatives considered

- **收窄客户端词表到 `linux-x86_64`**：落选。用「少承诺」掩盖「少交付」，还会推翻 ADR 0087 C1 已冻结的 API 面。
- **单 job 交叉编译（`cross` / zigbuild）**：落选。`native-tls` / `keyring` / `libdbus-sys` / `rusqlite-bundled`
  的 C 依赖在交叉目标上需要成体系 sysroot，本机不可验证、失败率高；原生 runner 更接近用户机器。
- **人工上传 mac/arm 二进制补空洞**：落选。ADR 0087 已裁决「清单由 CI 生成，不由人手写」，
  `v0.1.182` 人工上传件落后 tag 就是下场。
- **各平台 job 各自生成并上传清单**：落选，理由见「为什么」第 3 点。
- **顺手把清单改名成 `pony-tui-latest.json` 以消除命名歧义**：落选（P1）。该字面量已硬编码在存量二进制里，
  改名 = 老客户端静默断更；改为在 ADR/契约里钉死术语，门禁盯住常量不被改。
- **为 `ponycli` 拆独立 crate 甩掉 TUI 依赖**：落选（P3）。cargo 无 per-`[[bin]]` 依赖，收益只有体积，
  代价是新增一层跨 crate 契约。

## 后果

- 客户端代码零改动（`platforms` 本就是查表）；macOS(Apple Silicon) 与 Linux(aarch64) 用户从「必然失败」变为可升级。
- 新增约束：以后加发布平台必须同批改 `cli-platforms.mjs` 与 `release.yml`，只改一处 `npm run verify` 会红。
- **未验证项（靠下一次真实发版）**：`ubuntu-24.04-arm` / `macos-latest` 对本仓依赖树的可编译性，以及
  `-D warnings` 下 macOS 是否引入平台专属告警。失败是显式局部的：该平台 job 红但不阻断发版，
  `publish-cli` 打印 `[WARN] 本轮缺失平台…`；必需平台失败则整轮红。
- 本机收据：`node scripts/check-cli-release-contract.mjs` Exit 0；`npx vitest run tests/cli-release-contract.spec.ts`
  16 passed；`python3 -c "import yaml,…"` 解析新 workflow 得 jobs `['build-and-release','build-cli','publish-cli']`。
