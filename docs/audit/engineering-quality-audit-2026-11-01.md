# 工程质量管理审计（ponygo 元框架 + 仓库门禁实测）

- **审计对象**：`pony-agent` @ HEAD `9bf7862`（main），**工作树脏**（见 §5 并发写入告警）
- **审计时点**：2026-11-01 11:29–11:45 +0800
- **审计方式**：只读巡检。逐条实跑门禁命令并记录**机器原生 Exit Code**；不修改产品代码、不改门禁、不落盘修复
- **判定口径**：ponygo 判「有无/对不对」（形状/机制），本报告额外对「声明 vs 实况」做人工裁决（§3 P1-a/P1-b）

## 1. 结论摘要

| 维度 | 结论 | Exit |
|---|---|---|
| ponygo 治理成熟度 | **L2 达标**，M 档 14/14 命中，级自洽 PASS | 0 |
| ponygo 门禁自证（opt-in） | 1 个 `*.spec.sh` 负样本自检真跑通过 | 0 |
| ponygo 骨架完整度 | **承诺不完整**：缺 6 个骨架文件 | **1** |
| ponygo 骨架漂移 | 3 处本地适配漂移 + 6 缺口 | **1** |
| 可观测性四维（ob0/ob1/ob2） | **ob2 ≥ 目标 ob1**，无缺口，收口点在场 | 0 |
| ADR 全树格式门（1.1–1.10） | 全通过 + 11 条负样本 PASS | 0 |
| 版本四处同步 | PASS（core 0.1.139 / tauri 0.1.166） | 0 |
| 敏感信息全仓扫描 | **红**：1 处测试夹具命中 `sk-` 规则 | **1** |
| Rust 测试目标 lint 门 | PASS（`-D warnings` 下测试目标可编译） | 0 |
| 前端类型检查 | PASS | 0 |
| 前端构建 | PASS（3 条 bundler 告警） | 0 |
| 前端单测（全量） | **红**：6 failed / 1090 passed / 12 skipped | **1** |
| Rust 核心 check（`-D warnings`） | PASS | 0 |
| Rust 核心 lib 单测（`-D warnings`） | PASS：1177 passed / 0 failed | 0 |

**总判定**：治理机制层（L2）成立且门禁本体可执行；**当前工作树不可提交、不可发版**——存在 1 处 P0 门禁红（全仓 secrets）与 1 处 P1 钩子失效。

## 2. 实测凭据表（原生 Exit Code）

```
ponygo audit                          → exit 0    （M 档，命中 14/14，当前级 L2）
ponygo audit --run-specs              → exit 0    （2.3 执行判据真跑通过）
ponygo status                         → exit 1    （骨架文件缺口 6）
ponygo upgrade --check                → exit 1    （drift 3 + 缺口 6）
ponygo observe status                 → exit 0    （占位符无，收口点 error_boundary / http_client）
ponygo observe audit --json           → exit 0    （current=ob2, target=ob1, gaps=[]）
bash .meta/gates/ci-verify.sh         → exit 0    （ADR 全树 + 11 条负样本 spec 全 PASS）
node scripts/check-version-sync.mjs   → exit 0
node scripts/check-secrets.mjs --all  → exit 1    ← P0
node scripts/check-rust-test-lints.mjs→ exit 0
npx vue-tsc --noEmit                  → exit 0
npx vite build                        → exit 0    （built in 4.65s，3 告警）
npx vitest run                        → exit 1    ← 6 failed / 1090 passed / 12 skipped（113 files，3 failed）
RUSTFLAGS="-D warnings" cargo check -p pony-agent-core                          → exit 0
RUSTFLAGS="-D warnings" cargo test  -p pony-agent-core --lib                    → exit 0（1177 passed；41.54s）
RUSTFLAGS="-D warnings" cargo test  -p pony-agent-core --test *_red（未跟踪红相）→ exit 101（预期红相，未入仓）
npx openspec validate --all --strict  → 证据不足（本机未安装 openspec CLI，见 §4）
```

## 3. 缺陷清单（按因果闭合与阻断力排序）

### P0-a 全仓 secrets 门禁红（提交即红）

- **证据**：`node scripts/check-secrets.mjs --all` → exit 1，命中
  `crates/pony-agent-core/src/agent/secret_store.rs:382`
  `let secret_value = "sk-plaintext-fallback-value";`
- **性质**：测试占位符（非真实凭证），但违反本仓扫描器自带的处置建议——占位符前缀须含
  `mock|test|draft|dummy`；且该行**不在 HEAD**，属未提交工作树新增。
- **阻断力**：`.github/workflows/ci.yml` 明文执行 `node scripts/check-secrets.mjs --all`，
  一旦随提交进入远端，CI 立即红；同时本地 `npm run verify` 的 `secrets:check` 亦为红。
- **连带红**：仓库自带门禁自检 `tests/check-secrets.spec.ts:249`
  （"端到端零误报：runChecks({mode:'all'}) 必须 0 违规"）因此失败——**门禁自检如实报警，机制健康**。
- **最小动作（1 行）**：夹具改名为 `sk-mock-plaintext-fallback-value` 等合规前缀；
  禁止放宽扫描规则或加豁免（会破坏 wave-18c C4 的零误报/防过度纠正判据）。

### P1-a 中继层 pre-push 钩子实际失效（"有"是形状幻觉）

- **证据**：`git ls-files -s .githooks/` →
  `100644 .githooks/pre-push`（`pre-commit` 为 `100755`）。
  非可执行钩子被 git **静默忽略**（ponygo 自身 README 亦点名该陷阱）。
- **性质**：`ponygo audit` 的「中继层 pre-push（10秒级）：有」为**机械形状探测**，
  其免责声明已写"只做机械形状探测、不裁决价值"，本报告据此裁决为**声明与实况不符**：
  本机不存在 L2 增量类型检查层，`npm run typecheck` 实际只在 CI 与手动执行时发生。
- **最小动作**：`git update-index --chmod=+x .githooks/pre-push` 并随下一提交固化 100755；
  固化后用 `git ls-files -s` 复验位。

### P1-b 骨架不完整 → 治理承诺 exit 1

- **证据**：`ponygo status` → exit 1，缺 6：
  `.agents/notes/README.md`、`.agents/notes/AGENTS.md`、`.agents/notes/archived/AGENTS.md`、
  `.agents/skills/README.md`、`.agents/skills/governance-review/SKILL.md`、
  `.agents/skills/change-review/SKILL.md`。
- **同时**：`ponygo upgrade --check` → drift 3：`.agents/skills/write-adr/SKILL.md`、
  `.agents/skills/write-adr/verify-note.sh`、`.meta/gates/README.md`
  （三处均已**提交**于 `4375a77`，属有意的本地适配，非脏工作树漂移）。
- **最小动作**：`ponygo init --repair-skeleton`（只补缺失、不覆盖既有）补 6 文件；
  3 处 drift 二选一——逐条合入模板，或在 `.agents/notes/**/process/` 落一条适配说明
  （`alt-governance.md` 已确立"登记适配决策"的合法路径）。

### P2-a 本地绿门失真：红相产物落在默认收集面内

- **证据**：未跟踪文件 `tests/markdown-external-resource-allowlist.red.spec.ts`、
  `tests/runtime-session-event-isolation.red.spec.ts`（4 条断言红，红相预期）
  命中 `vitest.config.ts` 的 `include: ["tests/**/*.spec.ts"]`；
  另 3 个未跟踪 `crates/pony-agent-core/tests/*_red.rs` 使 `cargo test` 立即 101。
- **后果**：同一条工作树里「红相（应红）」与「绿门（应绿）」共用一套命令——
  本地 `npm run test:unit` / `cargo:test:shared` 恒红，而 CI 干净检出恒绿。
  这不是测试代码缺陷，是**门禁语义混载**，会训练团队忽略红灯。
- **最小动作**：红相统一落独立收集面（如 `tests/red/` 并在 `vitest.config.ts`
  `exclude`；Rust 侧以 `#[ignore]` 或独立 feature 承载），绿门命令不收集红相。

### P2-b 行为/契约变更未 same-commit 落文档

- **证据**：`ponygo audit` WARN「近 5 提交有代码变更但无（非-notes）文档更新」；
  人工核实 `9bf7862`、`c2fb776`（watchdog 行为对齐）、`14b99c0`（流事件按 sessionId 隔离）、
  `88fe82c`（侧边栏 pin）四个提交**零 `docs/` 变更**，而其中至少两项改变了运行时契约。
- **判据**：`docs/AGENTS.md` 写作规则 3「同提交原子性」。
- **最小动作**：把上述变更的契约面补进 `docs/architecture/`（既有
  `docs/architecture/structural-debt-proposals.md` 未跟踪稿可作为落脚点之一）。

### P3-a 构建告警 3 条（非阻断，技术债）

- `INEFFECTIVE_DYNAMIC_IMPORT` ×2：`src/stores/runtime.ts`（被 `src/lib/tauri.ts` 动态导入但被多处静态导入）、
  `src/stores/ask.ts`（被 `runtime.ts` 动态导入但被组件静态导入）→ 动态导入不产生分块收益。
- PURE 注释位置失效 ×1：纯注释写在错误位置被忽略，死代码消除不生效。
- 最小动作：单独立卡（不随本次审计夹带整改）。

### P3-b 本地 L1 层部分静默失效（Windows 专属脚本）

- **证据**：本机无 `pwsh`（`command -v pwsh` 空）；`.githooks/pre-commit` 中
  `git-guard.ps1` 与 `verify-decisions.ps1` 包在 `if command -v pwsh` 内 → **被静默跳过**。
  本地 L1 实际只剩 ponygo 注入的 `adr-verify.sh`。
- 同一形态扩散面：`package.json` 中 10 处 `powershell/pwsh` 调用（含
  `cargo:check:shared`、`cargo:test:shared`），在非 Windows 主机上 `npm run verify` 无法整体通过。
- 最小动作：为三条 `.ps1` 门禁补 bash/node 等价实现或显式声明平台边界（写入 docs，避免"看起来有"）。

## 4. 未覆盖 / 证据不足（诚实声明）

- **openspec canonical 校验**：`npx openspec validate --all --strict` 不可用（本机无该 CLI）；
  Dashboard 记载的历史 `54/54 passed` **本次未复验**。
- **E2E / Playwright**：未执行（需构建 + 浏览器矩阵，成本超本审计预算）。
- **Rust 集成测试全矩阵**：仅跑 `--lib`；未跟踪红相集成测试按预期 101，其余集成目标未逐个执行。
- **Windows 专属路径**：Job Object containment、Windows 进程树回收、签名更新链均无法在 Linux 主机取证。
- **ponygo 判据的"值不值"维度**：本报告不做价值裁决，仅给形状与实况对照（P6 归 review）。

## 5. 并发写入告警（结论有效期）

审计期间检测到**第二写入者**在同一工作树内活动：

- `crates/pony-agent-core/src/agent/mcp_client.rs` 11:30:57 修改
- `crates/pony-agent-core/src/agent/runtime/turn_persist.rs` 11:34:08 修改
- `crates/pony-agent-core/src/agent/provider/mod.rs` 11:42:38 修改
- 同机存在并发 `cargo test` 进程（`*_red` 集成测试、`streaming_client_uses_idle_read_timeout`）

因此：本报告为 **11:29–11:45 快照**；§2 表中 Rust/vitest 结论对应该时点源码。
存在 P0/P1 未闭合前不应提交；复核建议在写入者落盘收敛后重跑：

```
ponygo audit --run-specs && ponygo status && ponygo upgrade --check \
  && bash .meta/gates/ci-verify.sh && node scripts/check-secrets.mjs --all \
  && npx vitest run && RUSTFLAGS="-D warnings" cargo test -p pony-agent-core --lib
```

## 6. 处置顺序建议（最小闭包）

1. **P0-a**：夹具改名 1 行 → 同时消 `check-secrets` 扫描红 + 门禁自检 spec 红（2 红一处根因）。
2. **P1-a**：`git update-index --chmod=+x .githooks/pre-push`，恢复 L2 增量类型检查层。
3. **P2-a**：红相移出绿门收集面，恢复"本地绿 = 可提交"的信号可信度。
4. **P1-b**：`ponygo init --repair-skeleton` 补 6 文件；3 处 drift 登记适配。
5. **P2-b / P3**：立卡处理，不与上述同批提交（避免范围夹带）。
