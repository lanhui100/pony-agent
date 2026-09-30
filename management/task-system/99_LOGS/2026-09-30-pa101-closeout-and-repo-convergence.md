# 2026-09-30 PA-101 收尾补齐与仓库收敛

## 本次做了什么

本轮是**流程收尾轮**，不新增功能。起因是核查项目进展时发现 PA-101（2026-09-25 已完成并提交）的收尾账没结清。

1. **环境修复**：会话开始时 shell 完全不可用（`SetNamedSecurityInfoW failed (Win32 5)`，DSH 无法为工作区配置授权）。经诊断，`D:\Documents\pony-agent` 缺少 `WRITE_OWNER`；已为当前用户补 FullControl 并复核通过。修复记录与回滚脚本在 `.acl-recovery/`。
2. **PA-101 canonical spec 补齐**：新建 `openspec/specs/update-security-and-release-followups/spec.md`，把归档 delta 的 4 条 ADDED / 2 条 MODIFIED / 1 条 REMOVED 要求按 canonical 格式（`## Purpose` + `## Requirements`）重写。此前该 change 完全没有 canonical spec。
3. **PA-101 change 归档**：`openspec/changes/pa101-update-followups/` → `openspec/changes/archive/2026-09-25-pa101-update-followups/`，并把 tasks.md 的 15 项**全部勾选**（此前 `openspec list` 显示 `0/15`），补写"收口记录"节。
4. **实测复核 F1/F2/F3**：归档前逐条验证 as-built 证据，而非直接采信任务卡描述（见下）。
5. **发现并确认一个更早的库存问题**：`openspec validate --all --strict` 报 **14 passed / 39 failed**（见"发现"节）。

## 关键改动文件

- `openspec/specs/update-security-and-release-followups/spec.md`（新建）
- `openspec/changes/archive/2026-09-25-pa101-update-followups/`（proposal / design / tasks / specs 四件套，自 `pa101-update-followups` 迁入）
- `management/task-system/00_DASHBOARD.md`
- `management/task-system/01_TASK_BOARD.md`
- `management/task-system/03_TASKS/PA-102-*.md`（新建，下一轮收敛卡）
- `.gitignore`

## 本次验证

```
npm run openspec -- validate update-security-and-release-followups --strict   # ✓ 通过
npm run openspec -- validate --all --strict                                   # 14 passed / 39 failed（见发现节）
npm run version:check                                                         # PASSED: 4 places in sync（tauri 0.1.92 / core 0.1.90）
npm run openspec:list                                                         # No active changes found.
```

F1/F2/F3 as-built 实测证据：

- `src-tauri/src/platform.rs:81` `ALLOWED_OPEN_HOSTS = ["github.com","api.github.com","exa.ai"]`；`:88 is_allowed_open_url`；`:192 ShellExecuteW` FFI 绑定；`:516` 起白名单矩阵测试。
- `src/lib/open-external-url.ts` 是唯一出口——`safeInvoke("open_url")` 全仓仅出现在该文件；`ConfigGeneralSection.vue:65/121` 两处调用点均走 `openExternalUrl`。
- 前端 `fetch(` 全仓仅 `src/lib/update-check.ts:140` 一处，符合"模型流量不走前端 fetch"架构不变量。
- `src-tauri/tauri.conf.json:28` CSP 非 null（含 `object-src 'none'`、`frame-src 'none'`）；`index.html:10` 有浏览器预览 meta CSP。
- `dompurify ^3.4.16` 在 dependencies；`tests/markdown-sanitize.redteam.spec.ts` 存在（29/29）。
- `scripts/check-version-sync.ps1` 存在并接入 `.github/workflows/ci.yml:38`；ADR `0017` 已落地并接替 0012。

## 发现：canonical spec 库存有 39 份不合规

`openspec validate --all --strict` 的失败原因统一为：

> Spec must have a Purpose section. Missing required sections. Expected headers: `## Purpose` and `## Requirements`.

即这 39 份 spec 保留了 **delta 格式**（`## ADDED Requirements` / `## MODIFIED Requirements`），而 strict 校验要求 canonical 格式。通过校验的 14 份正是当初按 canonical 重写过的（如 `app-update-check`、`workspace-sidebar-tree`、本轮新建的 `update-security-and-release-followups`）。

这不是本轮引入的回归：**PA-095 自己的 canonical spec（`event-sourcing-closeout`）也在失败名单里**，`turn-event-log`、`session-projection-layer`、`trace-event-projection` 等事件溯源主线的 spec 同样如此。属长期库存漂移，已立卡 PA-102。

## 发现：9 月三个任务缺会话日志

`PA-099`、`PA-100`、`PA-101` 均无对应 `99_LOGS/` 记录，最近日志停在 2026-08-15，与任务系统 README"每次会话结束都要补会话日志"的要求有落差。本轮补齐 PA-101；PA-099/PA-100 的历史细节已不可复原（只保留任务卡内的完成记录），不再补写虚构日志。

## 与 PA-101 任务的对照

PA-101 的**代码与门禁本身是完整的**（17 + 962 + 568 测试全绿、`cmd /c start` 零残留、四处版本一致）。欠的只是流程账：canonical spec 未同步、change 未归档、tasks.md 未勾选。本轮全部结清。

## 后续同轮进展：PA-102 §1 完成

同上会话继续推进 PA-102，§1（canonical spec 规范化 + CI 门禁）已完成：

- **诊断比立卡假设更简单**：38 份失败 spec 的原因**完全统一**——只缺 `## Purpose`。30 份需把 `## ADDED Requirements` 改名 `## Requirements`；8 份已有 `## Requirements`；`third-wave-default-tool-alignment` 的 `## Scope` 并入 `## Purpose`。立卡时设想的 MODIFIED/REMOVED 合并问题实际几乎不存在（仅 `event-sourcing-closeout` 一份含 MODIFIED）。
- **执行**：样板（`event-sourcing-closeout`）手工完成后，37 份交给 37 个并行子代理，每份独立读写并自跑 `validate` 自证。
- **防语义漂移（关键）**：对全部 38 份做 `git show HEAD:<file>` 与工作树的语义不变量差分——逐份比对 `### Requirement:` 与 `#### Scenario:` 的**数量与规范化标题文本集合**。结果**零漂移**；唯一差异是样板中两条需求标题去掉的跨 spec 来源标注，来源已改记在 Purpose 与条内引用块。
- **行尾卫生**：产出中 4 份缺结尾换行、1 份（`workspace-shell-navigation`）含 128 处 CRLF（违反 `.gitattributes` 的 `*.md text eol=lf`），已统一为 LF + 恰好一个结尾换行。此类"产出未过 EOL 卫生"是已知复发点。
- **CI 门禁**：新增 `scripts/check-openspec.ps1`（只读、解析 `Totals:` 行作判据、exit 0/1），接入 `.github/workflows/ci.yml`（置于 `npm ci` 之后）与 `package.json` 的 `npm run verify`（新增 `openspec:check`）。
- **门禁自身两个缺陷由双态实测暴露并修掉**（非推测）：① 脚本无 BOM 保存 → Windows PowerShell 5.1 按 ANSI 解码中文导致**语法错误**，已加 UTF-8 BOM；② 在 `$ErrorActionPreference='Stop'` 下 `2>&1` 捕获 native 命令 → OpenSpec 向 stderr 打印进度被提升为终止性 ErrorRecord，脚本**误判失败**，已在调用处临时降级为 `Continue` 且判据只认 stdout。
- **未决**：§2（`src-tauri/gen/schemas/` 产物定位需先决策，不宜单方面 gitignore）、§3（残余待办优先级属产品/排期判断）。

## 下一步最小动作

1. **PA-102 §2**：先决策 `src-tauri/gen/schemas/` 是"随 `tauri dev` 重生成的产物"还是"应随源码评审的契约文件"，再选 gitignore 或构建后补 LF。
2. **PA-102 §3**：逐条给残余待办定去向（已做 / 立卡 / 明确不做并写理由）。
3. 若继续 Phase 8，从 PA-102 卡 §3 登记的残余清单（Job Object containment、完整 SandboxBackend、`McpResourceSurface` 接线、事件溯源 facet 增量等）选取。

## Resume Hint

继续前先看：

- `management/task-system/03_TASKS/PA-101-update-security-and-release-followups.md`
- `management/task-system/03_TASKS/PA-102-canonical-spec-normalization-and-repo-hygiene.md`
- `openspec/specs/update-security-and-release-followups/spec.md`
- `management/task-system/00_DASHBOARD.md`
