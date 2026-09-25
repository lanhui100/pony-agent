# 实施计划：PA-101 与工程基线收尾

## 阶段 1: 隔离与归档核心层未暂存变更

**目标**: 梳理并验证 `crates/pony-agent-core` 的在途测试与补丁（包含 `CorruptedEventTombstone` 墓碑回归对齐、T2-A `startLine` 校验门、T2-B 退避观测报告），使其全量测试通过并形成独立提交，彻底净化工作树，避免污染 PA-101。
**成功标准**: `cargo test -p pony-agent-core --lib` 962+ 测试全绿；`crates/` 无残留脏修改。
**测试**: `cargo test -p pony-agent-core --lib`
**状态**: 已完成 (commit `f4970a0`)

## 阶段 2: 修复 F2 红队测试与完成 DOMPurify 评估

**目标**: 修复 `tests/markdown-sanitize.redteam.spec.ts` 第 9 行的 JSDoc 提前闭合语法错误；运行红队测试矩阵；根据测试结果评估手写 `sanitizeMarkdownHtml` vs `DOMPurify` 并给出明确结论（采用或保留）。
**成功标准**: `tests/markdown-sanitize.redteam.spec.ts` 正常通过/运行并产出书面评估结论。
**测试**: `npx vitest run tests/markdown-sanitize.redteam.spec.ts`
**状态**: 已完成 (红队 29 项全绿，产出 DOMPurify 采纳结论)

## 阶段 3: 落实 F3 版本对齐与发版流水线约定

**目标**: 盘点已发布产物与 tag，修复 `tauri.conf.json` 中版本号（0.1.0 -> 0.1.91）；改造 `scripts/bump-version.ps1` 纳入 `tauri.conf.json` 键级替换；新增 `scripts/check-version-sync.ps1` 并验证双态 exit code；更新 ADR 0012 增补发版约定。
**成功标准**: 四处版本一致，`check-version-sync.ps1` 退出码为 0，篡改时非 0；`bump-version.ps1 -DryRun` 正常打印 4 处变更。
**测试**: `pwsh -NoProfile -File scripts/check-version-sync.ps1`，`pwsh -NoProfile -File scripts/bump-version.ps1 -DryRun`
**状态**: 进行中

## 阶段 4: PA-101 全套门禁与规范收口

**目标**: 运行全套门禁（Rust 白名单矩阵、cmd 残留 grep 门禁、前端 vitest、vue-tsc、build）；更新 `openspec/changes/pa101-update-followups/` 与 canonical specs；收口归档并更新任务看板。
**成功标准**: `cargo test`、`npm run verify`、`git grep "cmd.*/c.*start"` 为空，任务卡标为 Done。
**测试**: `npm run verify`, `git diff` 审查
**状态**: 未开始
