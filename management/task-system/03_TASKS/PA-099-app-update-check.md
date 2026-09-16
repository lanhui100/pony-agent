# PA-099 add-app-update-check

| 字段 | 值 |
|---|---|
| Task ID | PA-099 |
| 标题 | 配置页软件更新功能 + GitHub 发版侧栏角标提醒 |
| 状态 | Done（实现已落地并推送；用户手动验收未执行记档，见 Next Action） |
| 复杂度 | B |
| 负责 | @orchestrator（本会话）+ 子智能体 reviewer |
| 创建时间 | 2026-08-24 |
| 提交 | 实现随 `c684d38` 落地（随附落地 PA-099，与 ADR 0013 共享壳层文件按单提交原子回滚策略一并落地）；规范归档随 `b7ac018` 搬运（archive 下文档搬运 + `specs/app-update-check/spec.md`）；两者均已在 origin/main |
| spec | `openspec/changes/archive/2026-08-24-add-app-update-check/proposal.md`（v2，已归档） |
| review 记录 | `openspec/changes/2026-08-24-add-app-update-check/reviews.md`（spec 双审 + 代码双审采纳表均已回填） |
| ADR | `docs/decisions/0012-app-update-check-via-github-releases.md`（implemented，已过 Status 机械校验） |

## 背景

应用无更新感知能力；GitHub 仓库 `lanhui100/pony-agent` 以 `vX.Y.Z` tag 发版。需要：配置页提供"软件更新"区块，且当存在比当前版本新的 release 时，侧栏设置入口图标出现角标。

## 目标（已达成）

1. ✅ 配置页（通用 tab）新增软件更新卡片：当前版本 chip、手动检查、五态呈现、发布页跳转、自动检查开关。
2. ✅ 有新发版时，侧栏"设置"两个入口图标显示 amber 静态角标（v2 修订：从 4 入口收敛，避免 rose 失败语义冲突）。
3. ✅ 纯前端实现（fetch GitHub API），浏览器预览模式同样可用；零新增依赖、零 Rust 改动。

## 范围 / 非目标

见 proposal.md 的 Scope / Non-Goals。tauri.conf.json 版本失同步修复登记为 F3 前置跟进。

## 测试证据

- 全量 vitest：**30 文件 / 504 passed / 10 skipped / 0 failed**（代码双审修复后净增 11 用例）
  - 新增：update-check 39 例、update-store 18 例、ConfigGeneralSectionUpdate 9 例
  - 扩展：HomeSessionSidebar +4 例（角标显隐矩阵）、App.spec 启动计数同步 6→7
- `npm run typecheck`（vue-tsc --noEmit）：通过
- ui-guard 覆盖率：branches 79.04% —— **存量失败取证**：HEAD 基线（stash 后复测）78.95% 同样低于 80% 阈值，拖累源为本次未改动的 HomeWorkspace.vue (75.76%)；本改动使总分支覆盖率 +0.09pp
- 沙箱运行方式备注：`npx vitest run --configLoader native --pool threads`

## 审核闭环

1. Spec 双审（安全边界 / 架构一致性）：均"有条件通过" → proposal v2 修订（构造式 URL、缓存反规范化、超时幂等、角标收敛等 20 条裁决）。
2. 代码双审（正确性回归 / 边界安全落实）：均"有条件通过"，无 P0/P1 → 必修项 P2（json() 超时作用域缺口）+ 全部建议项与缺失测试已修复复验。
3. 完整采纳表见 reviews.md；残余风险（隐私 opt-out 依赖 localStorage 健康、企业限流常态失效、TLS 拦截文本级污染）已在 reviews.md 与任务卡登记知悉。

## 当前状态

- 2026-08-24：实现 + 双轮对抗审核 + 门禁全绿 + 收口文档齐备。实现已随 `c684d38` 提交并推送；OpenSpec change 已归档（`b7ac018` 搬运）。
- 2026-09-16（文档收敛）：修正 commit 归因（实现 `c684d38` vs 规范搬运 `b7ac018`）与 spec 路径（活跃目录→archive）。

## Next Action

1. （可选记档）用户手动验收路径：篡改缓存 tagName 为更高版本重启 → 设置入口出现角标 → 卡片可跳转真实 GitHub release 页（proposal 验收标准 3）。**验收未执行不阻塞 Done**：自动化门禁（vitest 504 + typecheck + 双审）已全绿；手动验收仅覆盖"真实 GitHub release 页跳转"一项外部交互。若后续验收发现问题，另立任务卡承接，不回灌本卡。

## Resume Hint

若中断：全部工作已完成并留痕于本卡与 `openspec/changes/2026-08-24-add-app-update-check/tasks.md`；仅剩归档与提交动作。

## Follow-ups（独立任务）

- F1 Rust `open_url` 加固（URL 白名单，替换 cmd/c start）
- F2 CSP 收紧 + DOMPurify 评估
- F3 发布流水线约定确认（tag == package.json 版本）+ tauri.conf.json 版本同步修复
