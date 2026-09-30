# PA-102 canonical spec 库存规范化与仓库卫生（P1，Complexity B）

- Task ID: PA-102
- 标题: 39 份 delta 格式 canonical spec 规范化 + 生成物噪音根治 + 残余待办收口
- 状态: In Progress（§1 已完成并接 CI；§2/§3 待决策）
- 复杂度: B（跨 39 份 spec 的机械改写 + 仓库卫生 + 残余清单归档）
- 负责人: @orchestrator + implementation
- 创建时间: 2026-09-30
- 提交: 待填（§1 本轮提交）

## 背景

2026-09-30 在补齐 PA-101 收尾时，用 `npm run openspec -- validate --all --strict` 做了全库体检，发现两项长期库存问题，均非本轮引入：

1. **39 / 53 份 canonical spec 不合规**。失败原因统一为：
   > Spec must have a Purpose section. Missing required sections. Expected headers: `## Purpose` and `## Requirements`.

   即这些 `openspec/specs/*/spec.md` 保留了 **delta 格式**（`## ADDED Requirements` / `## MODIFIED Requirements` / `## REMOVED Requirements`），而 strict 校验要求 canonical 格式（`## Purpose` + `## Requirements`）。通过校验的 14 份正是当初按 canonical 重写过的那批（`app-update-check`、`workspace-sidebar-tree`、`update-security-and-release-followups` 等）。

2. **生成物噪音**。`src-tauri/gen/schemas/*.json` 每次 `tauri dev` 重新生成后都不带末尾换行，而仓库版本带换行，导致工作树恒定出现 4 个"已修改"文件，diff 内容为空（仅 `\ No newline at end of file`）。

## 目标

1. 把 39 份 delta 格式 spec 规范化为 canonical 格式，使 `validate --all --strict` 全绿。
2. 消除 `src-tauri/gen/schemas/*.json` 的恒常噪音。
3. 收敛散落在各任务卡里的残余待办（见 §3）。

## 非目标

- 不改任何 spec 的**语义内容**——只补 `## Purpose` 节、把 `## ADDED Requirements` 提升为 `## Requirements`、按需合并/退役 MODIFIED/REMOVED 节。
- 不在本卡重做功能；不重开已归档 change。

## 已完成（2026-09-30 本轮）

- `openspec/specs/update-security-and-release-followups/spec.md` 已按 canonical 格式新建（PA-101 原缺 canonical spec），`validate --strict` 通过。
- PA-101 change 已归档 `openspec/changes/archive/2026-09-25-pa101-update-followups/`，`openspec list` 返回 `No active changes found`。
- `.acl-recovery/` 已入 `.gitignore`。

## §1 canonical spec 规范化（P1）

- [x] 1.1 先做**一份样板**：选 `event-sourcing-closeout`（P0 主线、内容最重），按 canonical 格式改写并复验 `validate event-sourcing-closeout --strict` 通过。
- [x] 1.2 固化改写的判据，避免"顺手改语义"：
  - `## ADDED Requirements` → `## Requirements`；
  - 补齐 `## Purpose`（一到三句，写这份 spec 规范什么、对应哪张任务卡）；
  - `## MODIFIED Requirements`：若被改写的原要求已不在任何 spec 中，直接并入 `## Requirements`；若仍在，保留改写后的最终态文本；
  - `## REMOVED Requirements`：按仓库既有先例**降级为 Requirements 中的"SHALL be removed / 已移除"约束**（参照 `update-security-and-release-followups` 对 `cmd /c start` 的处理），不要直接删除——删除会丢失验收契约。
- [x] 1.3 批量处理剩余 38 份，按主题分批提交（建议：事件溯源批 / 工具系统批 / 会话与上下文批 / 前端与观测批 / workspace 批），每批后跑 `validate --all --strict` 记录通过数增量。
- [x] 1.4 全绿后把 `validate --all --strict` 接进 CI（当前 `.github/workflows/ci.yml` 只接了版本同步门禁），防止再次漂移。

### §1 完成记录（2026-09-30）

**诊断结论（比立卡时的假设更简单）**：38 份失败 spec 的失败原因**完全统一**——只缺 `## Purpose`。其中 30 份是 `## ADDED Requirements` 需改名 `## Requirements`，另 8 份（`provider-retry-and-backoff-boundary`、`workspace-shell-navigation`、`session-cursor-view-contract`、`skills-registry-bridge`、`history-node-management`、`model-monitor-telemetry`、`cache-hit-optimization`、`mcp-capability-bridge`）已有 `## Requirements`，只补 Purpose 即可。`third-wave-default-tool-alignment` 的 `## Scope` 节按判据并入 `## Purpose`。**立卡时设想的 MODIFIED/REMOVED 合并问题实际几乎不存在**：仅 `event-sourcing-closeout` 一份含 MODIFIED 节，已并入 Requirements 并在 Purpose 与条内注明来源。

**执行方式**：1.1 样板（`event-sourcing-closeout`）手工完成并验证；1.3 的 37 份交给 37 个并行子代理（每份一个，独立读写 + 自跑 validate 自证），统一 schema 回执。

**防语义漂移验证（关键）**：对全部 38 份做 `git show HEAD:<file>` 与工作树的语义不变量差分——逐份比对 `### Requirement:` 与 `#### Scenario:` 的**数量与规范化标题文本集合**。结果：**零漂移**，唯一差异是我在样板中对两条需求标题去掉的跨 spec 来源标注 `(from session-cursor-view-contract)` / `(from trace-event-projection)`，该来源已改记在 Purpose 与条内引用块，信息未丢失。

**行尾卫生**：`write`/子代理产出中有 4 份缺结尾换行、1 份（`workspace-shell-navigation`）含 128 处 CRLF（违反 `.gitattributes` 的 `*.md text eol=lf`），已统一为 LF + 恰好一个结尾换行并复核全绿。3.4 节记录的这一类"candidate 产出未过 EOL 卫生"是已知复发点。

**CI 门禁（1.4）**：新增 `scripts/check-openspec.ps1`（只读，解析 `Totals:` 行作判据，exit 0/1），接入：
- `.github/workflows/ci.yml`（置于 `npm ci` 之后，因依赖 OpenSpec CLI）
- `package.json` 的 `npm run verify` 链（新增 `openspec:check`）

门禁双态实测（Windows PowerShell 5.1，即 npm 实际调用者）：正向 `PASSED: 53 specs valid (0 failed)` exit 0；注入一份 delta 格式 spec 后 `失败：53 passed / 1 failed` exit 1；清理后复归 exit 0。

**过程中修掉的两个门禁自身缺陷**（都由双态实测暴露，非推测）：
1. 脚本原以无 BOM UTF-8 保存 → PS 5.1 按 ANSI 解码中文导致**语法错误**。已加 UTF-8 BOM。
2. 脚本原在 `$ErrorActionPreference='Stop'` 下调用 native 命令并 `2>&1` 捕获 → OpenSpec 向 stderr 打印进度时被提升为终止性 ErrorRecord，脚本**误判失败**。已在调用处临时降级为 `Continue`，判据只认 stdout 的 `Totals:` 行。

**§1 验收**：`npm run openspec -- validate --all --strict` → `53 passed / 0 failed`；`npm run version:check` → PASSED；两个门禁互不影响。

## §2 生成物噪音根治（P2）

- [ ] 2.1 确认 `src-tauri/gen/schemas/` 的定位：是"随 `tauri dev` 重生成的产物"还是"应随源码评审的契约文件"。若为前者，考虑 `gitignore` + 按需生成；若须保留跟踪，则在构建脚本末尾统一补 LF。
- [ ] 2.2 落地所选方案，并验证：跑一次 `tauri dev` 后 `git status` 不再出现这 4 个文件。
- [ ] 2.3 当前工作树的 4 个文件需要 `git checkout --` 恢复为 HEAD 版本（不要在提交里带上它们）。

## §3 残余待办收敛（P1）

以下条目此前散落在各任务卡的"残余/后续"节中，本卡负责登记为可排期条目（逐条决定：本轮做 / 继续 Backlog / 明确不做）：

- 事件溯源（PA-095 残余）：#2 阶段 B 行级 facet 增量 + wal 基线、squash 生产入口、`append_turn` panic `Result` 化评估。
- 工具系统（PA-076 后续）：PA-077 Windows Job Object containment、完整 `SandboxBackend`、`McpResourceSurface` 真实 McpTransport 接线、生产 resolver 接线（hostname WebFetch 由 fail-closed 转启用）、生产默认从 `LegacyCompatiblePolicyEvaluator` 迁移到保守审批。
- Workspace（PA-081 Non-Goals）：会话跨 workspace 移动、workspace 内文件浏览器。
- 工具错误保真（PA-100 残余）：多跳回合 RL 预算按跳重置、无回合级上限；浮点 `startLine` 静默 clamp。
- 前端可访问性（PA-096 残余）：disclosure `aria-expanded`、e2e 导航矩阵、embedded IPC 缓存。

## 风险与回滚

- 39 份批量改写有"顺手改语义"的风险——用 1.1 样板 + 1.2 判据 + 每批 `validate` 增量三重约束；本卡**不引入行为变更**，纯文档结构，回滚即 `git revert` 对应批次。
- `gitignore` 生成物若判断错误，会导致契约文件丢失跟踪——2.1 必须先明确产物定位再动手。

## 测试计划

- 文档：`npm run openspec -- validate --all --strict`（目标 53 passed / 0 failed）。
- 工程：`npm run version:check` 保持 PASSED；`tauri dev` 后 `git status` 干净（§2 完成后）。
- 若 1.4 落地：CI 上验证 `validate` 门禁在篡改场景下确实失败。

## 验收标准

1. ✅ `npm run openspec -- validate --all --strict` 输出 `53 passed / 0 failed`（2026-09-30 达成）。
2. ⏳ 跑一次 `tauri dev` 后 `git status` 无 `src-tauri/gen/schemas/*` 噪音（§2 未决）。
3. ⏳ §3 每条残余都有明确去向（已做 / 已立卡 / 明确不做并写理由）（§3 未决）。

## 当前进度小结（2026-09-30）

| 节 | 状态 | 说明 |
|----|------|------|
| §1 canonical spec 规范化 | ✅ 完成 | 38 份已规范化，全库 53/53 通过，CI + verify 门禁已接，零语义漂移实证 |
| §1.4 CI 门禁 | ✅ 完成 | `scripts/check-openspec.ps1`，双态实测通过 |
| §2 生成物噪音根治 | ⏳ 未决 | 需先决策 `src-tauri/gen/schemas/` 的产物定位（见 2.1），不宜单方面 gitignore |
| §3 残余待办收敛 | ⏳ 未决 | 需逐条做优先级决策，属产品/排期判断，不宜由执行方代决 |

## Resume Hint

继续前先看：

- `management/task-system/99_LOGS/2026-09-30-pa101-closeout-and-repo-convergence.md`（发现经过与本轮已做项）
- `openspec/specs/update-security-and-release-followups/spec.md`（样板：canonical 格式可参照）
- `openspec/specs/event-sourcing-closeout/spec.md`（1.1 的样板改造对象）
- `.gitattributes`（EOL 策略，PA-097/098 遗留）
