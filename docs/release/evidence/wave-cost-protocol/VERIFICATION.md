# wave-cost-protocol（B027）交付验证收据

- 分级：**A 级（标准路径）**，测试层级 **L2（契约级）**；判级依据：单模块簇（cost / turn_flow / turn_persist / budget），无并发/鉴权/Schema 变更
- 基线：`09c0d926` → 交付 commit：`47c30578`（`[decl-change-approved]`）
- 环境：Linux x86_64 / rustc 1.91.1 / node v22.21.0

## 1. 机器门禁（全部真实 Exit 0）

| 门禁 | 结果 |
| --- | --- |
| `cargo check -p pony-agent-core --all-targets` | Exit 0；warning 恒为基线 2 条（`mcp_channel_closure`），无新增 |
| `--test cost_protocol_parity`（新增，冻结） | 9 passed / 0 failed |
| `--test wave33_platform_cost_test`（迁移） | 1 passed / 0 failed |
| `--test wave5_cost_test` | 9 passed / 0 failed |
| `--test wave6_observability_polish_test`（K1.1 冻结） | 4 passed / 0 failed |
| `--test metrics_snapshot_contract` / `tui_metrics_snapshot` / `ts_contract_snapshot` | 10 / 12 / 1 passed |
| `--test observability_reporting_wave53_test` | 7 passed / 0 failed |
| `--lib agent::cost::` / `--lib agent::budget::` | 8 / 4 passed |
| `npm run typecheck` | Exit 0 |
| `npm run contract:check` | 通过（`TurnStreamEvent 38/38`） |
| `audit-diff.sh --base HEAD --decl-approved` | Exit 0（无越界实现、收口点完整） |
| `check-declaration.sh` | Exit 0（四维 PASS） |

## 2. 红相 → 绿相

- 红相（实现前，实测 E0432/E0061）：`cost_protocol_parity` 编译失败（`normalized_uncached_input_tokens` / `estimate_call_cost_usd` 未定义、`record_usage` 缺第 6 参）；`wave33_platform_cost_test` 同步红（签名未迁移）。
- 绿相（`47c30578`）：上述全绿，且 5 个既有成本/观测测试文件断言**逐字未变**。

## 3. 审查收据（均 `dev-review-lint` Exit 0）

| 收据 | 结论 |
| --- | --- |
| `.dev-team/review-join-cost-l3a.json` | **PASS**（0 BLOCKING / 5 ADVISORY，计价正确性/口径） |
| `.dev-team/review-join-cost-l3b.json` | **PASS**（0 BLOCKING / 4 ADVISORY，跨面一致/回归半径） |

审查确认：C1 单一出口、C2 归一唯一实现（`saturating_sub` 仅 1 处）、C3 wire≡落库（同传 `persisted.*`）、C4 边界成立（`cached>input` 时 uncached 饱和 0）、C5 未知不伪造；`wave33` 迁移仅签名+注释（数值字面量与 HEAD 逐字相同）。

## 4. 本轮修复的审查发现（文档级）

- `cost.rs` 模块头仍写固定 OpenAI 公式 ⇒ 改为「按协议归一 + 指向唯一实现」。
- `observability.yaml` 称 span/budget 也「收敛到同一出口函数」⇒ 改为「同一计价原语（函数不同、数值等价）」。

## 5. 残余（已登记 backlog，不在本波）

- **B036**（P1）：成本聚合粒度三面并存 —— 机器反例：`output=0` 被过滤时 0.1575 vs 0.3075；取整累积 0.000040 vs 0.000039；openai `cache>input` 0.004125 vs 0.003125。
- **B033**：cache 写入（`cache_creation`）无价格维度 ⇒ anthropic 缓存写入成本 100% 未计价。
- **B035**：熔断阈值 `>` vs `>=` 三处不一致（本波未触碰，未放大）。
- **B034**：治理汇总前端零调用点（B027 侦察发现）。

## 6. 基线披露（非本波引入）

`cargo test -p pony-agent-core --lib` 在**干净 HEAD 副本**（`git worktree add` 于 `09c0d926`，独立 target）同样 1 failed（`runtime/tests.rs:6184` 中断检查点文案 vs `CANCELLED_TURN_MESSAGE`）⇒ 既有基线红测，**不得**以「lib 全绿」作收尾凭证。

## 7. 工作区状态

`git status --porcelain` 仅剩 2 个本次之前既有的未跟踪 `.dev-team/retro/pack-session-*`（他次会话产物）；无后台进程/临时 worktree 残留（`/tmp/b027-head*` 已清理）。
