# Agent Note: CI 按变更面分流——纯文档/账本改动不再付 Rust 全量门

Status: implemented

## Problem

单人维护 + 多 AI 会话的仓库里，PR 的机械闸门只有一条 `verify`，而这条闸门的成本几乎全部来自
Rust 编译面：Windows runner 上 `cargo:check:shared` + clippy 棘轮 + `cargo test --no-fail-fast`
实测 **32 分 09 秒**（run 2026-11-20T01:50:44Z→02:22:53Z），合并到 main 后同一内容再跑一遍
**41 分 22 秒**（01:49:28Z→02:30:50Z）。

但本仓的高频 PR 类型不是代码：`chore(backlog)` 账本归档与 ADR 记录占了近期合并的多数
（#23 / #24 / #26 / #30 全是账本归档）。这些 PR 只改 `backlog/tasks.jsonl` 或 `.md`，
却要在 Windows runner 上完整重编一遍 Rust——32 分钟机器时间换一行 JSONL 的校验，
不能拿「保守」当理由。

## Decision

在 `.github/workflows/ci.yml` 前置一个 `changes` job，由 `scripts/ci-scope.mjs` 比对
PR base 与 HEAD 的变更路径，输出 `needs_rust`：

- 变更全落在纯文档/账本树（`docs/`、`backlog/`、`openspec/`、`.agents/notes/`、
  `.agents/skills/`、根级 `*.md`）⇒ `verify` 内部跳过 8 个 Rust 步骤，
  `core-platform-check` 整 job skip；
- 其余任何路径（含 `.github/`、`.meta/`、`scripts/`、`crates/`、`src-tauri/`、`src/`）⇒ 全量照旧。

三条设计约束（改动本机制前必须先读）：

1. **job 名与「必过」语义不动**：`verify` 仍是同一个 job、仍必然出现并 success（只是内部
   step 按需 skip）。因此远端 ruleset 里的 required check `verify` 不会因跳过而消失——
   这正是**不用 `paths-ignore`** 的原因。
2. **fail-safe 方向是「跑全量」**：base 解析不了、路径不在白名单、git 命令异常，
   一律 `needs_rust=true`。判错的两个方向代价不对称：多跑一次是几分钟机器时间，
   少跑一次是门禁静默消失。
3. **脚本恒 exit 0**：它是下游 `if` 的输入。非零退出会让 `changes` 变红 → `verify` 被 skip →
   同样是 required check 消失。异常只降级为 true，不向上抛。

`.meta/` 下的 markdown 被明确排除在「纯文档」之外（lint 表是 clippy 口径的真值源），
这条由 `scripts/ci-scope.spec.sh` 的 H 断言看守；spec 已接入 `ponygo-governance` 工作流，
与门禁本体同批执行。

## Alternatives considered

1. **在 `on.pull_request` 上加 `paths-ignore`**（最省事）。否决：GitHub 对被 `paths-ignore`
   跳过的 required check 会一直保持 pending，PR 永远无法合并。要么把 `verify` 从 required
   摘掉（等于拆门禁），要么改 `paths` 白名单反向枚举（改一处漏一处）。
2. **拆成 `verify-light` + `verify-heavy` 两 job，把 required 换成 light**。否决：要同步改远端
   ruleset（`gh api` 人工操作），门禁出现「仓库里看不到」的一半；新 check 名还需先在 CI 上
   出现过一次才能被 require，迁移窗口内合并会卡住。
3. **main 上不重跑 verify，只信 PR 的绿**。否决：squash 合并后的树是在新 main 上重放的，
   两个各自绿的 PR 合起来可能红。`main` 的绿必须是「真跑过的绿」——这是原 ADR 的明确取舍，
   不为省时间推翻。
4. **什么都不做，接受 32 分钟**。否决：账本归档是高频动作，成本线性叠加且零守护收益；
   而「门禁太慢 → 人开始绕门禁」在本仓已发生过一次（`git-guard.ps1` 在 Linux 静默失效）。

## Consequences

- 账本/文档类 PR：Rust 步骤全跳过，`verify` 只剩决策记录校验、版本同步、敏感信息扫描、
  `npm ci`、typecheck、前端单测——量级从 32min 降到几分钟。
- 代码 PR：多付一个 `changes` job（ubuntu，约 20–30s）+ `verify` 串行等待，成本可忽略。
- 新增守卫：`scripts/ci-scope.spec.sh`（11 条正负样本），随 governance 门在每次 PR 上跑。
- 未覆盖：`core-platform-check` 本来就不是 required check（原 ADR 已声明），它的 skip
  不改变合并语义；语义冲突仍靠 review。
