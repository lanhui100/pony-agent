# PA-099 add-app-update-check

| 字段 | 值 |
|---|---|
| Task ID | PA-099 |
| 标题 | 配置页软件更新功能 + GitHub 发版侧栏角标提醒 |
| 状态 | Done（待用户手动验收后归档 spec 目录） |
| 复杂度 | B |
| 负责 | @orchestrator（本会话）+ 子智能体 reviewer |
| 创建时间 | 2026-08-24 |
| spec | `openspec/changes/2026-08-24-add-app-update-check/proposal.md`（v2） |
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

- 2026-08-24：实现 + 双轮对抗审核 + 门禁全绿 + 收口文档齐备。未 commit（等待用户指示）。

## Next Action

1. 用户手动验收路径：篡改缓存 tagName 为更高版本重启 → 设置入口出现角标 → 卡片可跳转真实 GitHub release 页（proposal 验收标准 3）。
2. 用户确认后：归档 openspec 变更目录至 `archive/2026-08-24-add-app-update-check/` 并按需提交。

## Resume Hint

若中断：全部工作已完成并留痕于本卡与 `openspec/changes/2026-08-24-add-app-update-check/tasks.md`；仅剩归档与提交动作。

## Follow-ups（独立任务）

- F1 Rust `open_url` 加固（URL 白名单，替换 cmd/c start）
- F2 CSP 收紧 + DOMPurify 评估
- F3 发布流水线约定确认（tag == package.json 版本）+ tauri.conf.json 版本同步修复
