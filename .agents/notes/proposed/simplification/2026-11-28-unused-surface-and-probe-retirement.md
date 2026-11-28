# Agent Note: 未消费遗留面（死组件/探针二进制/悬空 IPC）分级清理边界裁决

Status: proposed

> **人工已批准（2026-11-28）**：方案照此执行，阻塞解除；落地进度见 `backlog:B1795831358623 / B1795831359003`。
> 本文件**仍留在 `proposed/`**直到代码在同一提交里落地——批准 ≠ 实现，提前迁 `implemented/` 即假闭环。
>
> **落地进度（一档已完成，B1795831358623）**：Rust 3 个未挂载孤立死文件（`pty/windows.rs`、`skills/watchdog.rs`、`test_utils/data_isolation.rs`）与 `examples/w61_e2e_probe.rs` 已删除，`file-size-baseline.json` 条目 56→55，`.meta/gates/no-residue.manifest` 首次启用并登记 5 个被删符号（范围限生产树 `crates/`，决策史/账本按设计保留名字）。
> **落地进度（一档前端部分已完成，B1795831359003）**：`AskPanel.vue` 与 `event-bus.ts` 已删除；ADR 0074「无墙钟过期态」契约迁到 `tests/acceptance/stage-1-ask-user.spec.ts`（生产载体 `AskUserToolCallCard.vue`），`AskPlanPanel.spec.ts` 裁剪后更名 `PlanPanel.spec.ts`，`tabular-nums-contract` 只删 C2-4 段、`workspace-width-alignment` 只删 AskPanel 用例。
> **未落地**：二档探针/示例二进制与 `non_tauri_harness` 去留、24 条悬空 IPC 收口仍待二档裁决（本 ADR 的 Proposal 第 2/3 条未获批执行范围）。

## Problem

全仓静态搜索（`rg --fixed-strings`）与代码盘点显示，多次架构拆分与功能迭代（见 [2026-11-25-build-test-layering-and-workspace-crate-split.md](../../implemented/architecture/2026-11-25-build-test-layering-and-workspace-crate-split.md) 与 [2026-11-24-test-hygiene-and-cli-lib-extraction.md](../../implemented/architecture/2026-11-24-test-hygiene-and-cli-lib-extraction.md)）后，仓内累积了四类零生产消费者的遗留代码，持续污染文件规模/Clippy 基线、增加编译与维护认知负担：

1. **Rust 未挂载孤立死文件（3 个，未参与任何编译模块树）**：
   - `crates/pony-agent-core/src/agent/pty/windows.rs`（22 行，`ConPtyDriver`/`WindowsPtyDriver` 空桩）；
   - `crates/pony-agent-core/src/skills/watchdog.rs`（25 行，`SkillWatchdog` 草稿）；
   - `crates/pony-agent-core/src/agent/test_utils/data_isolation.rs`（22 行，`PonyAgentTestContext`，其依赖 `tempfile` 在 `pony-agent-core/Cargo.toml` 均不存在）。
2. **Rust 零生产/CI 消费探针二进制与示例（8 个）**：
   - `src-tauri/src/bin/{decision_probe,direct_turn_probe,sse_turn_probe,trace_redteam_probe}.rs`（合计 1,577 行）；
   - `crates/pony-agent-core/src/bin/non_tauri_harness.rs`（198 行）；
   - `crates/pony-agent-core/examples/{perf_lazy,perf_probe,w61_e2e_probe}.rs`（合计 948 行，其中 `w61_e2e_probe.rs` 占用 `file-size-baseline.json` 一条 232 行超限配额）。
   - 对照：`crates/pony-agent-core/src/bin/conformance.rs` 为活跃门禁消费二进制，不可删。
3. **前端零生产引用组件/模块（4 个）**：
   - `src/components/AskPanel.vue`（0 生产引用，仅 4 个测试）；
   - `src/lib/event-bus.ts`（65 行，0 生产引用，仅 1 个单测）；
   - `src/components/ask/PermissionElevationCard.vue`（0 生产引用，且是 `authorize_path` 唯一前端调用点）；
   - `src/lib/runtime/acp-api.ts`（12 行，0 生产引用，B163 未接 UI）。
4. **Tauri 悬空 IPC 命令（24 条，含 `authorize_path` 共 25 条）**：`src-tauri/src/lib.rs:1660-1758` 注册的 97 条命令中，旧非流式命令（`run_turn`、`{start,continue,resume}_graph_run`）、ACP 预埋（`acp_delegate_{start,cancel}`）及 MCP/Skill/权限/诊断等 24 条在前端无任何生产调用；`acp://delegate` 事件前端零监听。

## Proposal

在人类审批后，按「零争议立删 / 迁出或收口 / 显式保留登记」三档执行清理，并同步启用 `.meta/gates/no-residue.manifest`（A8 零残留门）：

1. **一档（立删确认死代码，五件同提交）**：
   - 直接删除 Rust 3 个未挂载孤立文件；
   - 直接删除前端 `src/components/AskPanel.vue`（迁移/清理关联的 4 个测试文件）与 `src/lib/event-bus.ts`（连带删除 `tests/unit/lib/event-bus.spec.ts`）；
   - 删除 `examples/w61_e2e_probe.rs`（一次性 wave-61 证据已归档，删除后同步消减 `file-size-baseline.json`）。
2. **二档（需拍板裁决去留的探针与未接线产品面）**：
   - 将 `non_tauri_harness.rs` 迁入 `crates/pony-agent-core/tests/runtime_suite/` 作为真正的集成测试守护，或确认淘汰；
   - 裁决 `src-tauri/src/bin/*_probe.rs` 与 `examples/perf_*.rs` 是统一迁往独立 dev-tools / 删除，还是保留；
   - 裁决 `PermissionElevationCard.vue`（+ `authorize_path`）与 `acp-api.ts`（+ `acp_delegate_*` / `acp://delegate`）是按近期排期接线还是下线退场。
3. **三档（Tauri 悬空 IPC 收口）**：下线废弃的旧非流式命令（`run_turn` 等，与 `turn_sync` 取消缺陷清理协同），对确认保留的后端调试/预埋命令在 `src-tauri/tests/tauri_command_surface.rs` 显式分组注释用途，消除「看似已接入实则无人调用」的灰区。
4. **防回流机制**：每次删除提交按 `change-review` 负空间程序同步把被删符号写入 `.meta/gates/no-residue.manifest` 并落 `implemented/simplification/` ADR（含 `## Reintroduction conditions`）。

## Alternatives considered

- **方案 A：分级处置 + 启用 A8 零残留门（推荐）**。立即清除无争议死代码减小基线噪声，同时把探针与未接线产品面的去留明确交由人工拍板，不误伤手工排障手段与在途 ACP/权限功能。
- **方案 B：全量保留不动**。零删改风险，但落选：3 个未挂载死文件与 `AskPanel.vue`/`event-bus.ts` 纯属历史垃圾，持续误导新加入的 Agent 并维持 `clippy-baseline-src-tauri.json` 虚假存量。
- **方案 C：一刀切删除全部 24 条未消费 IPC 与全部探针**。最激进；但落选，因 `acp_delegate_*` 属 B163 已登记宿主面，`non_tauri_harness`、`perf_probe` 可能仍被开发者手工跑性能/契约验证，未经拍板直接删除有破坏排障工作流风险。

## Acceptance criteria

1. 人工明确拍板二档清单（4 个 `*_probe.rs`、2 个 `perf_*.rs`、`non_tauri_harness.rs`、`PermissionElevationCard.vue`、`acp-api.ts` 与 24 条悬空 IPC）的去留裁决。
2. 一档确认死代码全部按五件同提交程序删除，生产树 `rg --fixed-strings` 零残留，并启用 `.meta/gates/no-residue.manifest`。
3. 对应删除提交使 `bash .meta/gates/ci-verify.sh`、`npm run size:check`、`npm run rust:lint:tauri` 全部通过且基线净下降。

## Risks

- `src-tauri/tests/tauri_command_surface.rs` 或前端某隐蔽动态拼接字符串可能断言了部分命令名（已核查前端仅 `history.ts:107` 动态拼接 3 条分支命令），删除前必须跑全量 `rg` 与编译/测试。
- 删掉 `AskPanel.vue` 时，4 个测试文件中可能夹杂通用布局回归断言（如 `workspace-width-alignment.spec.ts`、`wave17/tabular-nums-contract.spec.ts`），需把有效断言迁往 `AskUserToolCallCard.vue` 而非整文件盲删。
