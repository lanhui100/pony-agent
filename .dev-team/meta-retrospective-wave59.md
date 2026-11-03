# 元复盘 · wave-59 · PA-164 Headless 交付形态补全

> 交付区间：`9706859`(红相/夹具) → `6b903fd`(契约 v5/NFR/三路审查收据) → `46cc067`(实现)
> 分支 `wave-59/headless`，隔离 worktree（主工作区 244 个未提交 WIP 文件未被触碰）
> 分级：B（跨文件 + 外部网络 IO ⇒ 机械触发强制 ≥B）

## 一、通信拓扑与信噪比

- **架构事实先于契约**：本波最大收益来自「先侦察后冻结」。两路只读 Explorer（E1 枚举 / E2 判断）在写第一行契约前推翻了原始设计的三处假设——`start_turn_stream` 同步阻塞（原设想"同线程轮询超时"不可行）、`turn:suspended` 是终态（CLI 因此死循环）、默认权限 fail-open（原以为需要自造放行判定）。**契约 v1→v5 共 5 版，其中 3 版变更由侦察与审查驱动，而非需求变更。**
- Token 效率：复用 `CARGO_TARGET_DIR` 后单次 `--lib` 37s，无重复全量构建；无重复读取。
- **信噪比问题**：三路审查合计 26 条 findings，BLOCKING 仅 4 条（15%）。Lead 的裁定成本高于发现成本，是本波最大单项开销。

## 二、门禁穿透与误杀率

- **独立 Test Agent 封堵了 2 类假绿**：① E2E 原先只 mock `Read` 工具，`read_only` 下合法 ⇒ 根本没覆盖 suspended；② `ASK_ARGS` 缺 `description` 使 `ask_user` 在参数校验期失败，`exit 6` 在任何正确实现下都不可达。
- **红→绿鉴别力有物理对照**：`headless_contract` 3/22 → 3/23 → 26/26；E2E 13/10 → 11/19 → 30/0。断言非恒真。
- **三路 Join Barrier 全 FAIL，抓到 4 个真 BLOCKING**：SSE 帧无终止符且未复用既有 `format_sse_event`；`--help` 谎报默认档位；机读模式 REPL 破坏 C2/C3；`--result-json` 可被符号链接劫持。若无独立审查，这 4 项会以「五门禁全绿」交付。
- **1 个误杀**：L3-P 的 `rc=124`（停读消费者致 stdout 背压）被判 BLOCKING。裁定降级为声明式限制并写入 NFR `forbidden_claims`（禁称"已解决"）。误杀成本 = 一次双向复证 + 契约 1 条，可接受。

## 三、分工契约与隔离有效性

- **隔离 worktree 是必需而非可选**：主工作区 244 个未提交文件中，`bin/cli.rs` 正是本波要改的文件（含他人 `/compact` WIP）。在主工作区作业则 diff 归属审计必然失败。
- **写域审计抓了一次真实越界需求**：stdout 纯净性在声明的 2 文件写域内**物理不可达**（`otel::log` 无条件写 stdout）。正确处置是显式扩权 + 记录理由（+17 行开关），而非偷偷改或降低契约。
- **Executor 三次主动上报"测试与契约矛盾"且不动手改测试**，三次均被证实为 TEST-DEFECT —— "测试只读"纪律真实落地。
- **串行化正确**：T1→T2→T3 严格 `blocked_by`；契约 amend 期间 Executor 空等，避免 v3/v4 双向返工。

## 四、元协议迭代建议（针对 dev-team skill）

1. **契约设"可兑现性"前置门**：本波撤销 `--allow-tools` / `--max-turns` / `run_headless` 三项，根因同为"先写契约、后验接线口"。建议 Phase 1 强制新增：「每个对外声明的参数必须给出**消费它的代码位置**，否则不得进入契约」。
2. **审查输出按严重度分级预算**：建议审查者先交 BLOCKING 摘要（≤5 行），ADVISORY 折叠进 JSON 由 Lead 按需拉取。
3. **`review-*.json` 机器校验元数据应由 prompt 模板预置**：本波三份 JSON 各补 3 轮字段才过 lint，属纯流程摩擦。
4. **E2E 夹具过"必填参数自检"**：`ASK_ARGS` 缺字段会让整条路径静默失效。建议 L2-AT 段落加一条：「mock 的工具调用参数必须来自真实工具 schema，不得手写猜」。

## 五、计划阶段审查收据

- 方案对抗审查：`.dev-team/review-plan-wave59.json`（verdict=FAIL，9 BLOCKING → 全部裁定并落契约 §4）
- 机械校验：`dev-review-lint.py` **PASS**
- 侦察收据：E1（枚举型）/ E2（判断型）只读报告，`path:line` 证据已回填契约 §4

## 六、实施收口阶段审查收据

- `.dev-team/review-join-a.json`（L3-A 并发/调度，verdict=FAIL → 修复并复验）
- `.dev-team/review-join-b.json`（L3-B 安全/契约，verdict=FAIL → 修复并复验）
- `.dev-team/review-l3p.json`（L3-P 生产六问，verdict=FAIL → 1 项声明式降级 + 其余修复）
- 三份均通过 `dev-review-lint.py`（schema v2）

## 七、交付门禁收据（Lead 亲跑，非转述）

| 门禁 | 命令 | 结果 |
| :-- | :-- | :-- |
| G1 | `cargo test -p pony-agent-core --test headless_contract` | 26 passed / exit 0 |
| G2 | `cargo test -p pony-agent-core --test cli_acceptance` | 5 passed / exit 0 |
| G3 | `cargo test -p pony-agent-core --lib` | 1195 passed / exit 0（与改前基线逐项一致） |
| G4 | `RUSTFLAGS="-D warnings" cargo check -p pony-agent-core` | exit 0 |
| G5 | `bash tests/acceptance/wave59/headless-cli-e2e.sh` | PASS=30 / FAIL=0 / exit 0 |
| 观测 | `audit-diff.sh --base 9706859` | `[OK]` / exit 0 |
| 观测 | `check-declaration.sh` | `结论: PASS` / exit 0 |
| 冒烟 | help / jsonl 纯净性 / SSE 帧 / REPL 禁入 / 超时上限 / 符号链接防护 | 全部实测通过 |
