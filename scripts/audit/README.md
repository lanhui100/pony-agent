# scripts/audit/ —— trace 审核 loop 确定性节点（PA-169）

设计正典：[ADR 0101](../../.agents/notes/archived/architecture/2026-11-01-trace-audit-loop-automation.md) §确定性节点。
本目录只做**机械可查**的部分；判断节点（audit / propose / implement）由 Agent 按
`.agents/skills/trace-audit-loop/SKILL.md` 执行。**verdict 只认退出码，禁止口头判 pass。**

## 退出码词表（全节点统一）

| 码 | 语义 | 纪律 |
| :-- | :-- | :-- |
| 0 | pass | 只有全绿才允许 |
| 1 | fail（断言/门禁红） | 阻断推进 |
| 2 | 证据缺失（UNKNOWN） | **UNKNOWN ≠ PASS**，不得折算成 0 |
| 3 | 环境不可用（缺凭据/探针不可达/缺工具链/缺 corpus） | 同上，不得退化成「空数据=全缺失」 |
| 4 | 冻结漂移（corpus/断言哈希被事后改动） | 须新提案重审 |

## 节点清单

| 文件 | 职责 | 关键纪律 |
| :-- | :-- | :-- |
| `lib.sh` | 公共库：退出码词表、回执落盘、corpus 聚合哈希 | 回执格式唯一实现处 |
| `state.py` | `loop-state.json` 唯一读写实现（CAS / lease / 冻结） | 各节点禁止自行写状态文件 |
| `replay.sh` | 双臂 scratch worktree 重放（每臂 ≥3 会话） | 臂目录不可变：同名重跑直接拒绝 |
| `fetch.sh` | 按 session id 拉 Langfuse → `metrics.json` | 凭据只走 stdin 管道；缺凭据 = 3 |
| `assertions/dim<N>.sh` | 维度阈值断言（阈值随提案批准冻结） | 超阈非零；证据缺失 2 |
| `analyze_replay_isolation.py` | A1 隔离取证（`Environment: cwd=` 真值） | 无 jsonl = 2，不是 pass |
| `ratchet.sh` + `ratchet_add.py` | 已通过维度断言全集（单调增长） | 只增不减；换脚本路径 = 拒写 |
| `freeze-check.sh` + `scope_check.py` | 冻结哈希漂移 + 提案写域越界 | 越界 harness diff = 红 |
| `build-install.sh` | `npm run verify` + cargo build + 本机安装 | 本机无 pwsh 时走显式等价序列并记录 `verify_mode` |
| `verdict.sh` | 唯一裁决者：freeze → assert → ratchet → 棘轮登记 | pass 才写棘轮 |

## 典型一轮

```bash
# 1. 双臂重放（scratch worktree，固定提交）
bash scripts/audit/replay.sh --arm base      --commit <baseline-sha> --label r1-base
bash scripts/audit/replay.sh --arm candidate --commit <candidate-sha> --label r1-cand

# 2. 取 Langfuse 指标（缺凭据 → exit 3，不判 pass）
bash scripts/audit/fetch.sh --arm-dir docs/audit/rounds/r1-dim6/replay/r1-cand \
  --out docs/audit/rounds/r1-dim6/metrics.candidate.json

# 3. 唯一裁决（写回执 + pass 时登记棘轮）
bash scripts/audit/verdict.sh --round r1-dim6 --dim 6 \
  --arm-dir docs/audit/rounds/r1-dim6/replay/r1-cand \
  --expected-cwd /tmp/audit-r1-dim6-r1-cand \
  --metrics docs/audit/rounds/r1-dim6/metrics.candidate.json

# 4. 状态机推进（CAS + lease）
python3 scripts/audit/state.py take-lease --session <session-id>
python3 scripts/audit/state.py transition --session <session-id> --to verify
```

## 平台注记

- 仓库 `npm run verify` 依赖 `pwsh`（`scripts/invoke-rust-target.ps1`）。本机（Linux dev 服务器）
  无 pwsh，`build-install.sh` 自动切换为显式等价序列（version:check / test:unit / build /
  contract:check / size:check / ext:check / cargo check），并在输出与回执里标注
  `verify_mode=linux-fallback`。换跑法不等于放宽标准：任一步非零即整体红。
- `replay.sh` 的 corpus 复制进 worktree 后只读使用（corpus 任务 `write_access: none`）。
