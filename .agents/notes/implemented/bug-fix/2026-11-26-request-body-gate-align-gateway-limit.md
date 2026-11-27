# Agent Note: 请求体字节闸门对齐网关量级（撤销 512 KiB 纯文本硬闸）

Status: implemented

## Problem

`ab41d8f1`（2026-11-26，「迁移流式重试防重复、字节闸门与短窗口 429 修复至 pony-provider」）
把 `serialize_request_body_with_size_gate` 迁入 `crates/pony-provider` 时，按
「纯文本 512 KiB / 含图 10 MiB」分档设了硬上限。该取值与本仓既有决策直接冲突：

- `.agents/notes/implemented/bug-fix/2026-11-24-native-compaction-split-and-overflow-rescue.md`
  的 Decision 1 已明确「**不在客户端对 HTTP 请求体设 512 KiB 类小体积硬拦截**」，
  其 Alternatives 一节把「在客户端继续保留 512 KiB / 1 MiB 硬限制」列为**拒绝项**。
- 两条 ADR 于是在同一主题上互相矛盾，且实际生效的是被拒绝的那一档。

量级算术即可证伪 512 KiB：1M 窗口按 `trigger_threshold = 0.8` 触发压缩（80 万 token），
`estimate_text_tokens = chars/4` ⇒ 约 320 万字符；UTF-8 中文 3 字节/字符 ⇒ **约 9.6 MB**
合法请求体。512 KiB 只占窗口的约 **5%**，等于把 1M 模型阉割到约 5 万 token 可用；
更严重的是请求在本地就被判死（错误文本含 `payload too large` ⇒
`UpstreamContextOverflow` ⇒ `RequiresRequestMutation`），**永远走不到 80% 压缩路径**，
用户侧表现为「长会话突然中断且无法自愈」。

## Decision

1. **单档硬上限 128 MiB**（`MAX_REQUEST_BODY_BYTES`）：对齐 ponyllm 网关自身上限，
   删除 `MAX_MULTIMODAL_REQUEST_BODY_BYTES`，含图与纯文本共用同一判据。
   闸门保留的唯一职责是「序列化异常要 0 HTTP 失败」，而不是业务侧体积治理。
2. **软告警阈值 32 MiB**（`REQUEST_BODY_WARN_BYTES`）：只落结构化 warn，不拦截；
   取消旧实现「含图请求不告警」的例外（含图请求同样值得复盘）。
3. **上限可注入**（`serialize_request_body_with_size_gate_limit`）：生产恒用 128 MiB，
   单测用小上限验证拦截路径，不必造 128 MiB 报文换一次断言。
4. **正向锁与反向锁同时入库**（`crates/pony-provider/src/provider/tests_retry_window.rs`）：
   - 反向锁 `request_body_size_gate_passes_legitimate_long_context_body`：3.2 MB 合法长会话
     请求体**必须放行** —— 任何人把上限调回 MiB 以下，此用例立刻变红；
   - 正向锁 `request_body_size_gate_applies_one_limit_regardless_of_images`：含图请求
     不再享受 10 MiB 特例，同一上限一视同仁。
5. **上下文装配侧的字节维度治理不在本条收口**：本条只拆「误判」，不新增
   「按字节提前触发压缩」。后者要改装配口径，单独评审。

## Alternatives considered

- **保留 512 KiB 并只在纯文本档提到 8–16 MiB**：拒绝。仍是拍脑袋的两档分叉，且
  「多少字节算合法」随模型窗口漂移（1M/200K/256K 各不相同），分档必然再次腐化；
  单档取网关上限是唯一与上游事实同源的取值。
- **完全删除闸门（不设任何上限）**：拒绝。序列化异常（容器体积爆炸、误把大 blob 塞进
  请求体）会变成「一个畸形包反复打到边缘网关」，正是 `B1795488338107` 现场
  （300KB+ 请求被 EdgeOne 10~13ms RST、重试放大成防刷封禁）的复发路径。
  闸门作为 fail-closed 兜底有价值，只是判据必须高于任何真实合法请求。
- **在 HTTP 出口静默裁剪超限请求体**：拒绝（沿用 2026-11-24 条结论）。HTTP 层已脱离
  会话语义，硬剪会破坏 prompt cache 前缀、破坏 `tool_call`/`tool_result` 闭合
  （Anthropic 400）或误删 `reasoning_content`（DeepSeek 400）。
- **把软告警阈值也提到 128 MiB（即取消软告警）**：拒绝。软告警是「体积异常增长」的
  唯一低成本观测面，删掉后长会话膨胀到 GB 级才被发现。

## Consequences

- 涉及源码：`crates/pony-provider/src/provider/mod.rs`（常量与闸门）、
  `crates/pony-provider/src/provider/tests_retry_window.rs`（4 条用例）。
- 被取代的旧结论：`.agents/notes/implemented/bug-fix/2026-11-24-followup-retry-transcript-and-upstream-window.md`
  的「请求体字节闸门已落地（B1795506800002）：纯文本 512 KiB / 含图 10 MiB」一节已由本条取代，
  该条正文已链回本条。
- 1M 窗口长会话不再在本地被 512 KiB 判死，能正常走到 80% 压缩线；含图请求与纯文本
  共用同一上限，不再出现「纯文本 600KB 拦、含图 600KB 放」的口径裂缝。
- 残余边界：请求体体积仍只有「上限兜底 + 软告警」，没有「按预估字节提前触发压缩」。
  中文密集 / 重 tools schema 会话在 token 估算偏低时仍可能先撞 128 MiB 才被拦（概率低但非零）。
- 工具链注记（非本条引入）：本机 `/usr/local/bin/rustc` 与 `~/.cargo/bin/rustc` 同为
  1.91.1 但产物不通用，`cargo-lane.sh` 会把 `~/.cargo/bin` 前置导致 E0514；本条验证统一用
  `RUSTC=/usr/lib/rust-1.91/bin/rustc` 前缀。
