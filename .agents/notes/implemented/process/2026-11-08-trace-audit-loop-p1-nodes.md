# Agent Note: PA-169 确定性节点脚本化落地（scripts/audit/*）

Status: implemented

## Context

ADR 0101 把 trace 审核 loop 定为「用户唤起 + 持久状态机 + 确定性脚本骨架 + 单一审批门」。
P0（PA-168）以人工等价命令驱动 r1-dim6 各阶段并已推进到 `await_approval` 停点；缺口是
机械可查的节点仍散在 `docs/audit/rounds/r1-dim6/tools/` 的人工脚本里，没有 CAS 状态机、
没有棘轮、没有冻结/写域检查、没有唯一裁决者，导致「假 PASS」只能靠人盯。

## Decision

1. 新建 `scripts/audit/`，把 ADR 0101 §确定性节点表的 C1–C8 全部落成脚本：
   `lib.sh`（退出码词表 + 回执格式 + corpus 聚合哈希）、`state.py`（loop-state 唯一读写：
   CAS/lease/freeze）、`replay.sh`、`fetch.sh`、`assertions/dim6.sh`、
   `analyze_replay_isolation.py`、`ratchet.sh` + `ratchet_add.py`、
   `freeze-check.sh` + `scope_check.py`、`build-install.sh`、`verdict.sh`。
2. **统一退出码词表**：0 pass / 1 fail / 2 证据缺失（UNKNOWN） / 3 环境不可用 / 4 冻结漂移。
   `UNKNOWN ≠ PASS` 由脚本层强制，不依赖 Agent 自觉。
3. **verdict 是唯一裁决者**：顺序为 freeze-check → 本维度 assert → 棘轮全集 →
   pass 才写棘轮并落回执。任何一步非零都不产 pass；freeze 漂移（4）与写域越界（1）
   优先于断言结论，防止「阈值被改松后判绿」。
4. **冻结集扩展**：`assertion_hashes` 除 P0 人工脚本外，纳入 `scripts/audit/**`
   （前缀 `scripts_` / `scripts_assertions_`），使批准后偷改 verdict/assert 实现可被检出。
5. **重放臂不可变**：`replay.sh` 的臂目录带 `--label` 后缀，已存在 `sessions.tsv` 直接拒绝
   重跑——这是 r1-dim6 首轮「同名重跑覆盖旧证据」事故的直接立法。
6. **build-install 的平台诚实**：本机（Linux）无 pwsh，`npm run verify` 必然在
   `cargo:check:shared` 处失败；脚本自动切换为显式等价序列并输出
   `verify_mode=linux-fallback`，任一步非零即整体红（换跑法 ≠ 放宽标准）。
7. 交付 `scripts/audit/selftest.sh`：26 条夹具断言覆盖各节点红相/绿相，`npm run audit:selftest`
   一条命令可复现，作为「机械可查的承诺配非零退出命令」的落点。

## Alternatives considered

- **只写文档 + 人工等价命令（维持 P0 形态）**：零成本，但「假 PASS」与「同名覆盖证据」
  两类事故无法机械拦截；P0 已实测两起（003 回执漂移、base 臂被覆盖）。否决。
- **把所有判断节点也脚本化（含 audit/propose 由脚本生成）**：需要内嵌 LLM 判断，
  等于重造 harness 判断层，且无法审计「根因是否成立」。否决：判断留 Agent，机械留脚本。
- **棘轮集直接写进 `npm run verify`**：会让每次前端改动都跑一遍 Langfuse 取证（需凭据、
  需集群可达），在 CI/离线环境必然 UNKNOWN 而阻断无关改动。本轮只提供
  `npm run audit:ratchet` 独立入口；接线 `verify` 留待 patrol 阶段评估。
- **fetch 直接用 curl/宿主网络访问 Langfuse**：本机到 Langfuse 无直连路由，必须走
  observability skill 的 `in_probe_stdin` 探针通道；自建通道会绕开既有凭据纪律。否决。
- **replay 在主仓直接跑（不建 worktree）**：污染工作区且初始状态不确定，破坏归因。否决。

## Receipts

- `docs/audit/rounds/r1-dim6/receipts/009-p1-nodes-implement-script.json`：节点落地。
- `.../010-p1-selftest-implement-script.json`：`bash scripts/audit/selftest.sh` exit=0，pass=26 fail=0。
- `.../011-p1-replay-smoke-verify-script.json`：`replay.sh` 3 会话 × t01 冒烟 exit=0。
- `.../012-p1-fetch-smoke-verify-script.json`：`fetch.sh` 单会话取证 exit=0（trace 0244fec6…）。
- `.../013-p1-fetch-unknown-verify-script.json`：抽凭据后 fetch exit=3（UNKNOWN ≠ PASS）。
- 复核命令：`bash scripts/audit/selftest.sh`（唯一入口，退出码即结论）。

## 落地后修正（同轮内发现的三个自指/污染缺陷）

跑通 r1-dim6 真臂时暴露并已修复，均配机器证据：

1. **写域基线语义混用**：`baseline_commit`（重放臂固定提交）被当作写域 diff 起点，
   导致主线上介于两者之间的无关提交（他人 ADR/模块拆分 34 个文件）被算成本轮越界。
   修正：新增 `impl_base_commit`（实施开始时的 HEAD）专用于写域检查，缺省时回退旧字段
   并打印偏严提示。证据：`freeze-check.sh` 由 34 violations → 0。
2. **冻结集自指**：`state.py`/`freeze-check.sh` 等**检查器自身**也在冻结集里，任何一次
   修 bug 都立刻变成「冻结漂移」假红。修正：冻结集只含**判定语义**文件
   （`assertions/*`、`analyze_replay_isolation.py`、`fetch/replay/ratchet/verdict`），
   检查器完整性由 git 历史 + review + `selftest.sh` 承担。
3. **轮次快照缺失**：`loop-state.current` 只描述当前轮；队列推进后旧轮无法复跑裁决
   （实测复跑 exit=2 假 UNKNOWN）。修正：`state.py archive` 归档
   `rounds/<round>/round-state.json`，`freeze-check.sh` 优先读快照。

另修一处**测试污染**：replay 臂的 `*.jsonl`（10–40 MiB/会话）未忽略，使
`audit-diff.sh` 对 96 个 >2 MiB 未跟踪文件发 WARN ⇒ verdict=WARN ⇒ exit 1，
连带打红 wave47-ob2 与 v10 基线冻结门禁。`.gitignore` 加 `docs/audit/rounds/*/replay/`
（小体积 metrics/arm/sessions 索引仍跟踪）。
