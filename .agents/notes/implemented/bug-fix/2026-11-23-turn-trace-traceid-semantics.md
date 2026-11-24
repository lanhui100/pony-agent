# Agent Note: turn trace 继承判据改口径——`trace_id()` 是「导出态」，不是「继承关系」

Status: implemented

## Problem

Windows CI 上 `agent::otel::tests::begin_turn_trace_with_parent_inherits_trace_id_locally`
**连续四次偶发失败**（#58 两次、#59 一次、#62 一次），每次都是同一条断言：

```
assertion `left == right` failed: W3 trace：队友 turn 的 trace id 必须与宿主父 trace 相同
left: None
```

根因不是产品缺陷，而是**测试口径用错了字段**：

- `TurnTraceGuard::trace_id()` 的语义是**导出态** trace id（代码注释原文：
  `exported = false ⇒ 永不作子 span 的 parent、永不作 issue/前端跳转的 trace_id`）；
- 它的取值来自 `sampled_trace_id(&mut span)`：`span_context.is_valid() && is_sampled()` 才返回
  `Some(trace_id)`，**未采样时返回 `None`**；
- 于是「真实 SDK span 通路 + 未被采样」这一组合下，`trace_id()` 必然是 `None`，但 trace 关系
  其实**已经正确建立**在 span context 上；
- 走哪条通路又取决于进程内全局 OTEL provider 是否已被别的测试装配 —— 并行执行时这就是个
  **环境相关的骰子**，于是表现为 Windows 上偶发、本地单跑必绿。

## Decision

**只改测试口径，不动生产代码**（生产的语义本来就是对的）：

1. 继承关系的主判据换成 `current_trace_id()` —— 它直接读活动 turn 的 **span context**
   （`active_turn_span_context()` → 校验 valid + 小写 32 位 hex），与采样无关。
2. `scope.trace_id()` 改为**上界约束**：若存在则必须等于父 trace id；为 `None` 时不再据此判失败。
3. 断言消息同步改写，避免下次有人按旧措辞继续误判。

## Verification

- `cargo-lane test -p pony-agent-core --lib begin_turn_trace_with_parent` → 1 passed
- `cargo-lane test -p pony-agent-core --lib` → 1665 passed / 0 failed
- 后续 Windows CI 观察：该用例不再因「未采样」而红

## Alternatives considered

- **把该用例与其它装配全局 provider 的测试串行化**：只能把概率降低，不能消除——真正的错配是
  "拿导出态字段判继承关系"，串行化后换个采样配置仍然会红。
- **强制走本地派生通路再断言**：本地通路永远正确，测出来当然绿——但真实通路（有后端时的生产路径）
  就彻底失去覆盖，属于用伪装换绿灯。
- **CI 上给该 job 加重试**：把口径缺陷藏进重试统计里；本仓已有「重跑通过」的历史记录（#58 那次），
  继续重试等于承认这条断言不可信。
- **改生产代码让 `trace_id()` 恒返回 span context 的 trace id**：会破坏「导出态」与「本地相关性」
  的分水岭（B1/B1b），下游 issue 跳转与子 span 挂载都会拿到不可导出的 trace。
