# PA-169: Trace 审核 Loop P1 —— 确定性节点脚本化

## 状态
- **状态**: **完成**。C1–C8 脚本落地 + 四项验收全部有机器证据：
  1. 维度 6 完整 round 由 `scripts/audit/verdict.sh` 机械判 **pass（exit=0）**，dim6 已入棘轮（回执 018）；
  2. 红相证据：抽凭据 fetch exit=3（回执 013）、构造超阈 assert exit=1（`selftest.sh` 断言）、缺 metrics verdict exit=2；
  3. replay 双臂 worktree 初始提交对拍一致、主仓 `git status` 不受污染（`replay.sh` 强制 worktree + 臂目录不可变）；
  4. 棘轮纳入 dim6 后，用未修基线臂复现 **REGRESSION dim6 exit=1**（回执 019）。
  节点自测 `bash scripts/audit/selftest.sh` → exit=0，pass=26 fail=0。
- **优先级**: P1
- **Complexity**: B
- **创建日期**: 2026-11-08
- **前置依赖**: PA-168（维度 6 未过不得自动化其上）
- **决策依据**: [ADR 0101](../../../docs/decisions/0101-trace-audit-loop-automation.md)

## 需求背景

P0 以人工等价命令驱动各阶段，回执证明流程可行；本卡把机械可查的节点全部脚本化，Agent 只保留 audit/propose/implement 判断节点。

## 交付契约

- **C1 replay**：`scripts/audit/replay.sh`——scratch worktree 双臂（baseline/candidate 各固定提交），每臂 ≥3 会话，`ponycli --output-format jsonl --result-json --permission-mode --timeout-secs`；corpus 只读约束；进程失败/退出码异常 = 红。
- **C2 fetch**：`scripts/audit/fetch.sh`——复用 check-langfuse.sh 通道纪律，按审计标签拉会话 → metrics.json；缺凭据/不可达 = 3（UNKNOWN ≠ PASS）。
- **C3 assert**：`scripts/audit/assertions/dim<N>.sh`——阈值读自批准提案冻结值，超阈非零。
- **C4 ratchet**：`scripts/audit/ratchet.sh`——已通过维度断言全集单调增长（ADR 0090 纪律），任一红阻断。
- **C5 build-install**：`scripts/audit/build-install.sh`——npm run verify + cargo build + 本机安装替换，非零 = 红。
- **C6 verdict**：`scripts/audit/verdict.sh`——assert + ratchet + 前后对照，唯一裁决者。
- **C7 归因**：`audit.round / dimension / arm` 标记注入既有 session/release 归因键；属性面不足时经 ADR 0068 契约版本化通道扩展，不造私有 schema。
- **C8 冻结检查**：corpus_hash / assertion_hashes 漂移、提案写域外 harness diff → 红。

## 非目标

- 不做整轮自动编排与 patrol（PA-170）。
- 不重写 harness 工具循环/治理/trace 出口。

## 验收标准（物理收据）

1. 维度 6 首个完整 round 由脚本机械判 pass（verdict.sh 退出码 + 回执）。
2. 红相证据：构造 UNKNOWN（抽凭据）时 fetch 退 3 且 verdict 不 pass；构造超阈时 assert 非零。
3. replay 双臂 worktree 初始提交对拍一致，主仓 `git status` 不受污染。
4. ratchet 纳入 dim6 断言后，破坏上报链路可复现阻断。