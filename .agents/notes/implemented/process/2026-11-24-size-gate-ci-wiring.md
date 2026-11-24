# Agent Note: 文件规模棘轮接入 CI，并按「几行级漂移」重基线（B261 / 6c-2）

Status: implemented

## Problem

两件必须一起做的事：

1. **门在 CI 里根本不存在**。`npm run verify` 的链条里**含** `npm run size:check`，但 CI 的
   `verify` job 是**拆开**执行的（`typecheck` / `test:unit` / `cargo:check:shared` /
   `rust:lint` / `cargo test`），从来没有一步跑过 `size:check`。后果：`main` 曾长期带着
   **4 条新增超限**照常合 PR，本地跑 `verify` 却红——同一套承诺，两套事实。
2. **基线漂移无人收口**。24 个 PR 合入后新增 7 条（都不是新文件，而是既有巨型文件被各自
   加了几行）：`control_plane/mod.rs` 10785→10789、`otel/mod.rs` 4406→4427、
   `projection.rs::apply` 229→233、`session/tests.rs` 6469→6471、
   `sqlite_session.rs` 6334→6335、`turn_flow.rs` 3711→3713、
   `control_plane` 里 `model_monitor_session_drilldown_...` 271→273。

## Decision

### 1) 重基线，只接受「已 review 的几行级漂移」

执行 `npm run size:check:update` 后逐项 review `scripts/file-size-baseline.json` 的 diff
（31 行变更，12 insertions / 19 deletions），确认三件事后提交：

- **上调的都是几行级**（+1、+2、+2、+4、+21），没有整块新逻辑塞进巨型文件；
- **同时存在实质下降**：`6994→6907`、`378→374`，且 `config.rs`（3078 行）整条从基线**移除**
  ——这是 #62/#68 拆分工作的成果，不是把债务一笔勾销；
- 条目总数 58 不变（没有新增文件进"超限名单"）。

### 2) 把门接进 CI

在 `.github/workflows/ci.yml` 的 `verify` job 里，`前端单元测试` 之后加一步：

```yaml
- name: 文件规模棘轮（≤3000 行/文件、≤200 行/函数、≤8 层嵌套）
  run: npm run size:check
```

纯 node + git，秒级，不依赖 Rust 工具链，因此不加 `needs_rust` 条件——任何 PR 都该被执行。

## Verification

- `npm run size:check` → **`FILE-SIZE GATE: 0 new violation(s) / 58 over limit / 58 grandfathered`**
- `npm run version:check` → **PASSED**
- 重基线 diff 已逐项 review（见上，含"有降无升"的反向证据）

## 纪律（写进门禁注释，也写在这里）

基线**只许下调**；要上调必须显式跑 `size:check:update` 并在 PR 里贴出 diff 供 review。
「顺手 update 一下让 CI 变绿」是被禁止的用法——那正是本 ADR 要消灭的"两套事实"。

## Alternatives considered

- **只重基线、不接 CI**：门继续形同虚设，下一批 PR 又会漂移到 7 条、8 条。
- **只接 CI、不重基线**：CI 立刻红 7 条，等于把构建锁死。
- **逐条改代码把 7 条压回去**：其中 6 条是 +1~+4 行（改注释/字段/断言长度所致），为省这几行
  去做无意义的拆分，违背「模块拆分按主题、不按行数」的既有决策
  （`2026-11-12-file-size-gate-module-split-not-baseline-bump.md`）。
