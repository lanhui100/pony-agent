# Agent Note: 补齐 `plan_control` 输入面 golden 哈希（`abort` op 落地时的漏更新）

Status: implemented

## Context

`b6fd928d fix(ask-loop): Ask 自锁三处收口… [decl-change-approved]` 给 `plan_control` 增加了
模型可见的 `abort` op（`PlanControlOperation::Abort`，`plan_id`/`revision` 均可省略），
其 `input_schema` 的 serde 序列化随之逐字节变化。

`crates/pony-agent-core/tests/tool_surface_singularity.rs::t1_builtin_surface_identity_is_frozen`
（契约：45 个内建 descriptor 的名称/顺序/kind/exposure/`FNV-1a(input_schema)` 全部冻结）在同一
提交里**没有同步** `BUILTIN_GOLDEN` 中 `builtin:plan_control` 的哈希，于是干净树也红：

```
干净 HEAD 329d2cc9（/tmp/wt-splash，无任何未提交改动）：
  cargo test -p pony-agent-core --test tool_surface_singularity
  → test result: FAILED. 22 passed; 1 failed
  panicked at tests/tool_surface_singularity.rs:416:
  `builtin:plan_control` 的 input_schema 逐字节变化（KV-cache 前缀 / 模型契约不变量）
    left: 2191957339585205402 / right: 4076802470783492270
```

这是**声明变更的收尾遗漏**，不是行为退化：`abort` 的加入已获 `[decl-change-approved]` 盖章
（ADR 0092），golden 常量是同一变更的产物，理应同提交更新。

## Decision

只把 `BUILTIN_GOLDEN` 中 `builtin:plan_control` 的哈希由 `0x3893b63f527c20ae` 改为实测值
`0x1e6b65963cc9549a`，并在该行上方留注释说明「本次变化源于已批准的 `abort` op；
日后再变必须重新走 decl-change 评审，不得照抄实测值」，防止把冻结表退化成「跑红就抄一遍」的橡皮图章。

不新增/删除 descriptor，不改 schema 本身，不动产品代码。

## Alternatives considered

1. **回退 `abort` op 使哈希回到旧值** — 否决：`abort` 是解除 Draft 写门禁的必要出口
   （否则宿主拒绝执行审批后会话被锁死），ADR 0092 已批准，为让测试变绿回退功能属本末倒置。
2. **把冻结断言改成「只校验字段名集合，不校验哈希」** — 否决：哈希冻结的目标正是
   KV-cache 前缀与模型契约的逐字节稳定，弱化为集合比较后字段顺序/描述文案的漂移会静默通过。
3. **加一次 `--update-golden` 之类的自动写入开关** — 否决：需要人为二次确认的变更一旦可以
   一条命令刷白，护栏即失效；本仓约定是手工改常量并在 notes 里留下变更理由与批准凭据。
4. **把该用例标 `#[ignore]` 或从全量测试里排除** — 否决：这是模型面契约的主护栏，
   排除等于对 schema 漂移失明。

## Evidence

- 红：干净 HEAD `329d2cc9` 全量 → `error: 1 target failed: -p pony-agent-core --test tool_surface_singularity`。
- 绿：同树应用本改动后 `cargo test -p pony-agent-core --test tool_surface_singularity`
  → `test result: ok. 23 passed; 0 failed`。
- `b6fd928d` 中 `plan_state.rs` 的 schema 改动的唯一语义增量是新增 `Abort` 变体
  （两字段均 `#[serde(default)]`），无其它字段被删改。
