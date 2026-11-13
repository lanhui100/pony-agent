# 红相收据 · F1/F2/F3（独立测试者 L2-T）

- 交付人：test-agent（L2-T，独立测试分权）
- 时间窗：2026-11-12 19:37–20:03 (+08:00)
- 被测仓：`/home/dm/pony-agent`（工作区含 43+ 项与本任务无关的未提交改动，**全程未执行** `git stash/checkout/reset/commit/clean`，未修改任何业务实现文件）
- 契约真源：共享任务板 task-4 / task-5 / task-6 + 诊断裁决 `.dev-team/diag-tps/00-SYNTHESIS.md` + Lead 2026-11-12 两轮裁决（F3 契约 v2、F1 `turnBurstExcluded`=真突发 / `turnExcludedTotal`=valid−eligible）

---

## 0. 交付物（4 个测试文件，全部为 Test Agent 独占写域）

| # | 文件 | 用例数 | 说明 |
|---|------|--------|------|
| 1 | `crates/pony-agent-core/tests/f1_speed_observability.rs` | 4 | F1 轮终态 payload 的速度/口径/样本量取证 |
| 2 | `crates/pony-agent-core/tests/f2_retry_window_purity.rs` | 2 | F2 重试窗口纯净 + attempts 独立字段（本地回环 mock provider，零外网） |
| 3 | `crates/pony-agent-core/tests/f3_stream_throttle_isolation.rs` | 2 必绿 + 1 `#[ignore]` ADVISORY | F3 读循环零阻塞节流（静态 + 行为）+ 慢 sink 解耦已知限制 |
| 4 | `src/lib/runtime/__tests__/f1-token-metrics-sample.spec.ts` | 6 | F1 显示端：口径 / 样本量 / burst 剔除可辨别 / 无值不占位 |
| 5 | `crates/pony-agent-core/src/agent/runtime/tests.rs` + `src/agent/control_plane/mod.rs`（**既有用例期望**，Lead 指派，仅 test 模块） | 2 条既有用例 | 契约变更 F1 I1：把 `turn/speed` 作为显式一条加入有序相等期望（见 §9） |

> 目录约定：TS 文件放在任务指定的 `src/lib/runtime/__tests__/`，后缀取 `.spec.ts` 以匹配
> `vitest.config.ts` 的 `src/**/__tests__/**/*.spec.ts` 收集规则（`.test.ts` 在 `src/` 下不会被采集）。
> Rust 夹具未新建 `fixtures_diag_metrics/`：三条用例的记录级夹具全部内联（固定值、无外部文件）。

---

## 1. 验收命令与**真实退出码**（物理收据）

```bash
cd /home/dm/pony-agent
cargo test -p pony-agent-core --test f1_speed_observability                                  # Exit Code: 0
cargo test -p pony-agent-core --test f2_retry_window_purity                                   # Exit Code: 0
cargo test -p pony-agent-core --test f3_stream_throttle_isolation                              # Exit Code: 0
cargo test -p pony-agent-core --test f3_stream_throttle_isolation -- --ignored                  # Exit Code: 101（ADVISORY 锚点，非门禁）
npx vitest run src/lib/runtime/__tests__/f1-token-metrics-sample.spec.ts                        # Exit Code: 0
# F3 静态门（读路径零阻塞节流；干净 ⇒ Exit 0）
! grep -rnE "MIN_CLIENT_GAP|thread::sleep|\.sleep\(" \
    crates/pony-agent-core/src/agent/runtime/turn_stream.rs \
    crates/pony-agent-core/src/agent/runtime/tool_recovery.rs \
    crates/pony-agent-core/src/agent/provider/openai_sse.rs | grep -vE ':[0-9]+:[[:space:]]*//'   # Exit Code: 0
```

`test result:` 实测（原始日志：`.dev-team/diag-tps/fix/raw/*.log`）：

```
f1.log:                    test result: ok.   4 passed; 0 failed; 0 ignored
f2.log:                    test result: ok.   2 passed; 0 failed; 0 ignored   (2.79s)
f3.log:                    test result: ok.   2 passed; 0 failed; 1 ignored   (0.13s)
f3_advisory_ignored.log:   test result: FAILED. 0 passed; 1 failed; 0 ignored (2.14s)
f1_ts.log:                 Tests  6 passed (6)
```

---

## 2. 红相判定（逐项：本轮**实际观察到**什么）

> **重要前置**：三个修复由两个 Executor **并发落地**。我写测试期间 `cargo` 多次因其中间态无法编译
> （`retry.rs:1277 E0412`、`tool_recovery.rs:258 unexpected closing delimiter`）。等到第一次可编译时，
> F1/F2/F3 的实现已基本落地 ⇒ F1-Rust / F2 / F3 的「未修复基线红」**无法在事后复现**。
> 对此我不做主观推理放行，而是给出**可重跑的基线探针**（§3）与「修复前量的双数值」物理证据。

| 项 | 用例 | 红相结论 | 依据 |
|----|------|----------|------|
| F1-Rust | `f1_*`（4 条） | **未捕获到目标红**（实现已落地；首次可编译运行时即绿） | 基线探针 §3-R2：`git HEAD` 中 `turn_calls_total/turn_calls_eligible/turn_burst_excluded`（及 camelCase）命中数全部 = 0 ⇒ 修复前 payload 必缺三键，本文件断言必失败 |
| F1-TS | `f1-token-metrics-sample.spec.ts` | **已捕获真实红（断言失败，非编译错）**：2 failed / 4 passed | §4-① 原文摘录（19:44 运行，`token-metrics.ts` 当时仅返回 8 字段） |
| F2 | `f2_*`（2 条） | **未捕获到目标红**（实现已落地；首次可编译运行时即绿） | 基线探针 §3-R3（HEAD 无 `take_pure_call_window`/`ProviderAttemptWindow`）+ **同机双数值**：未修复取样点量出的 `outer_duration_ms≈1325–1363ms` > 本用例窗口上界 `350ms` ⇒ 基线必红；修复后 `pure_duration_ms≈61–73ms` ⇒ 必绿（§4-②） |
| F3-静态 | `f3_sse_read_path_has_no_blocking_throttle` | **未捕获到目标红**（读路径 sleep 已删除） | 基线探针 §3-R1：`git HEAD` 的 `turn_stream.rs` 有 4 处、`tool_recovery.rs` 有 2 处 `MIN_CLIENT_GAP + thread::sleep`（与诊断裁决定位逐行吻合）⇒ 该静态断言在基线必失败（门逻辑已用注入违例验证会 Exit 1，§3-R4） |
| F3-行为 | `f3_zero_gap_burst_is_not_capped_by_inline_throttle` | **未捕获到目标红**（16ms 节流已删除） | 同上 R1：基线 16ms/emit ⇒ 80 delta ≥ 1280ms ≫ 本用例上界 600ms |
| F3-慢 sink | `f3_read_loop_is_not_gated_by_slow_client_consumption` | **`#[ignore]` ADVISORY（契约 v2 降级）**；诊断运行实测失败：2.13s > 1.0s 上界 | §4-③（登记为已知限制，**不参与门禁**） |

**无编译期红**：最终 4 个测试文件全部编译通过（`--no-run` 等价于上表 Exit 0 的构建阶段），
所有失败/通过都来自断言而非缺失 API。唯一「编译红」窗口是 Executor 的中间态（§2 前置说明），
不是我的测试依赖新 API 名字造成的。

---

## 3. 可重跑的基线探针（供 Lead 在未修复基线复核红相）

### R1 · F3：基线 SSE 读路径确有阻塞节流（`git HEAD` 版本源码）

```bash
cd /home/dm/pony-agent
for f in crates/pony-agent-core/src/agent/runtime/turn_stream.rs \
         crates/pony-agent-core/src/agent/runtime/tool_recovery.rs \
         crates/pony-agent-core/src/agent/provider/openai_sse.rs; do
  git show HEAD:$f | grep -nE "MIN_CLIENT_GAP|thread::sleep" | sed "s|^|$f:|"
done
```
Exit Code: 0（**有命中**）。实测命中 21 行（turn_stream 15 + tool_recovery 6），分布 = `turn_stream.rs:1078/1079/1080/1081、1158/1159/1160、2328/2329/2330/2331、2378/2379/2380/2381`（4 处节流块）、
`tool_recovery.rs:214/215/216、244/245/246`（2 处节流块），`openai_sse.rs` 无。
与诊断裁决 `turn_stream.rs:1078,1158,2283,2333 / tool_recovery.rs:214,244` 逐行吻合（行号因并发改动偏移）。

### R2 · F1：基线 payload 确无样本量三键

```bash
cd /home/dm/pony-agent
for k in turn_calls_total turn_calls_eligible turn_burst_excluded turnCallsTotal turnCallsEligible turnBurstExcluded; do
  n=$( (git show HEAD:crates/pony-agent-core/src/agent/metrics_snapshot.rs
        git show HEAD:crates/pony-agent-core/src/agent/turn_flow.rs
        git show HEAD:crates/pony-agent-core/src/agent/runtime/types.rs) 2>/dev/null | grep -c "$k")
  echo "$k => HEAD 命中 $n"
done
```
实测：六个键 **全部 `HEAD 命中 0`**（工作区修复后 `turn_calls_total` 命中 2）⇒ 基线必然缺键 ⇒ `f1_terminal_event_payload_carries_speed_basis_and_sample_counts` 必失败。

### R3 · F2：基线无纯净窗口机制

```bash
cd /home/dm/pony-agent
(git show HEAD:crates/pony-agent-core/src/agent/retry.rs; \
 git show HEAD:crates/pony-agent-core/src/agent/runtime/turn_stream.rs) \
 | grep -cE "take_pure_call_window|begin_provider_attempt_window|ProviderAttemptWindow"
```
实测：**0**（工作区修复后 `turn_stream.rs` 有 3 处接线）。

### R4 · 静态门自检（证明它不是只会 Exit 0 的假门）

```bash
cd /tmp && printf 'fn main() {\n    std::thread::sleep(x);\n}\n' > gate_probe.rs
! grep -rnE "MIN_CLIENT_GAP|thread::sleep|\.sleep\(" gate_probe.rs | grep -vE ':[0-9]+:[[:space:]]*//'; echo "gate exit=$?"
rm -f gate_probe.rs
```
实测输出：`2:    std::thread::sleep(x);` + `gate exit=1`（**违例被检出**）。

---

## 4. 失败原文摘录（≤30 行/项）

### ① F1-TS：真实红（19:44，修复前 `resolveMetricsSnapshotDisplay` 只返回 8 字段）

```
 FAIL  src/lib/runtime/__tests__/f1-token-metrics-sample.spec.ts > F1 显示端：样本量可读出
 > 必须能从展示结构读出 total=3 与 eligible=1（字段 / 嵌套 / tooltip 文本任一路径）
AssertionError: 契约 I4：展示结构必须能读出「样本量 total=3」（新字段或 tooltip 文本），
实际可读数字 = [25000,100000,123.75,800,60,25]，
展示键 = ["contextTokens","contextWindow","speed","speedBasis","ttftMs","cacheRatioPercent","contextPercent","origin"]

 FAIL  src/lib/runtime/__tests__/f1-token-metrics-sample.spec.ts > F1 显示端：burst 剔除可辨别
 > burstExcluded=1 时展示必须能辨别（标记 + 数值 1）
AssertionError: 契约 I4：burstExcluded>0 时显示必须可辨别（tooltip/前缀/子行任一），
实际剔除类证据 = []，展示键 = ["contextTokens",...]

 Test Files  1 failed (1)
      Tests  2 failed | 4 passed (6)
```
修复后同一文件：`Tests  6 passed (6)`，Exit Code 0。

### ② F2：基线取样点 vs 纯净窗口的**同机双数值**（修复后日志原件，`raw/f2.log`）

```json
{"attempts":2,"failed_attempt_ms":763,"level":"info","message":"provider_call_retry_window",
 "outer_duration_ms":1334,"outer_first_token_latency_ms":null,"prelude_ms":1263,
 "pure_duration_ms":70,"pure_first_token_latency_ms":null,"slept_ms":500,
 "succeeded_attempt_offset_ms":1263,"trace_id":"...","ts":"2026-11-12T12:01:56.459Z"}
```
读数：`outer_duration_ms=1334ms`（**未修复口径**：跨 attempt 墙钟）> 用例上界 `GENERATION_WINDOW_BOUND_MS=350` ⇒ 基线必失败；
`pure_duration_ms=70ms`（修复后口径）⇒ 通过。`attempts=2` / `failed_attempt_ms=763`（≈ X=700ms）/ `slept_ms=500` 即 I2 要求的三独立字段。

### ③ F3：慢 sink 已知限制锚点（ADVISORY，`--ignored` 诊断运行，非门禁）

```
thread 'f3_read_loop_is_not_gated_by_slow_client_consumption' panicked at
crates/pony-agent-core/tests/f3_stream_throttle_isolation.rs:475:5:
F3 I1/I3：读循环处理单个 delta 不得被客户端消费速度阻塞——整轮耗时必须 < 1000ms
（线性基线 = 40 × 50ms = 2000ms 的 50%），实际 2.131238647s；慢客户端实际消费了 40 帧 delta
```

### ④ F1-Rust：实现落地过程中观察到的两次**语义不一致**（已按 Lead 裁决对齐，非最终缺陷）

```
契约 I1：turnBurstExcluded 必须是突发包剔除数 1（B：gen=100 < floor(400)=200），实际 Number(2)
        （=3 说明把「无 TTFT 观测」也当突发；=0 说明根本没统计）
契约 I1：无值场景禁止给 `metrics.turnCallsTotal` 写 0/NaN 等占位值，实际 Number(1)
```
处置：①`burstExcluded` 定义经 Lead 裁决为「真突发整包数」，我改用**无歧义夹具**（A eligible + B 真突发，
total=2/eligible=1/burst=1）——任何自洽定义都必须给出 1；混合夹具（追加「无 TTFT」记录）只断言区间与
`eligible + burst ≤ total`。②「无值」只对**速度口径**要求键缺席/null，样本量为已知事实（本轮确实调了 1 次），
故改为断言 `eligible==0 && burst==0 && total ≤ 1`。

---

## 5. 覆盖矩阵（契约条目 → 测试函数）

### F1（task-4 I1–I4）

| 契约条目 | 测试函数 | 文件 | 断言要点 |
|---|---|---|---|
| I1 终态 payload 含 `turnSpeedTokensPerSecond` / `turnSpeedBasis` | `f1_terminal_event_payload_carries_speed_basis_and_sample_counts` | f1_speed_observability.rs | 精确比对 `weighted_turn_speed` 真值（123.75）、basis=`stream_decode` |
| I1 含 `turnCallsTotal` / `turnCallsEligible` / `turnBurstExcluded` | 同上（+ `f1_sample_counts_are_consistent_on_mixed_exclusion_reasons`） | 同上 | total=2/eligible=1/burst=1；混合夹具 total=3、eligible=1、burst≥1、`eligible+burst ≤ total` |
| I1 无值不写键（禁 0/NaN） | `f1_terminal_event_payload_omits_speed_keys_when_no_value` | 同上 | 速度两键缺席或 JSON null；eligible=0、burst=0、total≤1 |
| I1 无记录 ⇒ 五键全缺席 | `f1_empty_records_omit_all_speed_keys` | 同上 | 五键一律不出现 |
| I3 同源同帧（禁消费面另写公式） | `f1_terminal_event_payload_carries_speed_basis_and_sample_counts` | 同上 | payload speed ≡ `weighted_turn_speed().speed`；`eligible == truth.calls`；同名键重复出现必须取值一致 |
| I4 显示端「口径可读出」 | `F1 显示端：口径（basis）可读出` ×2 | f1-token-metrics-sample.spec.ts | `stream_decode` / `end_to_end` 两向验证（防写死） |
| I4 显示端「样本量可读出」 | `F1 显示端：样本量可读出` | 同上 | 展示结构中可读出 3 与 1（字段/嵌套/tooltip 文本任一） |
| I4「burstExcluded>0 可辨别」 | `F1 显示端：burst 剔除可辨别` ×2（含反向钉子） | 同上 | burst=1 ⇒ 出现剔除标记且数值=1；burst=0 ⇒ 不得声称剔除 >0 |
| I4「无值不占位」 | `F1 显示端：无值不占位` | 同上 | `speed`/`speedBasis` 均为 null，展示内零 NaN/Infinity |
| （I2 日志行） | 未写成断言（日志捕获不在本轮验收口径内） | — | 实测日志行已带 I2 字段：`{"message":"turn speed settled","turnSpeedTokensPerSecond":123.75,"turnSpeedBasis":"stream_decode","turnCallsTotal":2,"turnCallsEligible":1,"turnBurstExcluded":1,"turnCallsTtftMissing":0,"turnExcludedTotal":1,"turn_id":"turn-f1-fixture"}`（见 `raw/f1.log`） |

### F2（task-5 I1–I4）

| 契约条目 | 测试函数 | 断言要点 |
|---|---|---|
| I1/I2 生成窗口只覆盖最后一次成功 attempt（不含 X） | `f2_generation_window_excludes_failed_attempt` | 真实链路「503（delay=700ms）→ 503 可重试 → 200」；`gen = dur − ttft < 350ms`；mock 侧 2 个完整请求体 |
| I3 分母干净 ⇒ 速度不被压低 | 同上 | `metrics.turnSpeedTokensPerSecond > 100`（40 tokens；污染口径只有 ~30 t/s 量级） |
| 反向钉子：不得靠禁重试通过 | 同上 | `requests >= 2` 且整轮墙钟 ≥ X |
| I2 attempts 次数与失败耗时作为独立字段可读出 | `f2_attempt_count_is_readable_as_independent_field` | 注入 `otel::log::set_log_file_path` 落盘 → 结构化日志行含 `attempts=2` 且 `failed_attempt_ms ≥ 0.8X`（wire 载荷或日志任一载体，语义键名匹配、不锁名字） |
| I4（既有测试全绿） | 由 Lead 侧门禁负责（本文件不跑全量） | — |

### F3（task-6，含 Lead 契约 v2 修订）

| 契约条目 | 测试函数 | 断言要点 |
|---|---|---|
| I1′ / I3′-① 读循环零阻塞节流（机械可查） | `f3_sse_read_path_has_no_blocking_throttle` | 剥行注释后，`turn_stream.rs` / `tool_recovery.rs` / `openai_sse.rs` 内 `thread::sleep` / `MIN_CLIENT_GAP` / `.sleep(` 零命中；文件行数 >100（防空转） |
| I3′-② 62.5 emits/s 上限不存在 | `f3_zero_gap_burst_is_not_capped_by_inline_throttle` | 80 × 0ms delta + 快 sink ⇒ 整轮 < 600ms（节流指纹 ≈ 1280ms） |
| I2 合并允许、最终帧与终态不丢 | 同上 + `f3_read_loop_is_not_gated_by_slow_client_consumption` | 末帧标记 `TOK079` 必达；恰好一条 `turn:completed` 且正文含末帧标记 |
| I1′ 慢 sink（v2 明确**允许** sink.emit 内联） | `f3_read_loop_is_not_gated_by_slow_client_consumption`（`#[ignore]`，ADVISORY） | 诊断运行实测 2.13s > 1.0s ⇒ 登记为结构性限制 |

---

## 6. 已知限制登记（Lead 契约 v2，F3）

- **限制**：`sink.emit` 在读线程内联执行。若宿主 sink 变成阻塞式（每次消费耗时 O(10ms)），读循环会被客户端消费
  速度拖慢（实测 40 × 50ms ≈ 2.13s）。
- **为何可接受**：生产 sink 均为非阻塞投递（Tauri emit fire-and-forget、TUI 无界 channel、headless 无消费者）；
  诊断实测 9/9 稳态 chunk 间隔 21–43ms（>16ms），旧节流本就不咬合。
- **彻底解耦路径**：把 emit 移到独立发送线程 + 队列合并（几十行并发改造 + 丢帧/终态风险）⇒ 走 backlog，本轮不做。
- **锚点**：`f3_read_loop_is_not_gated_by_slow_client_consumption`（`#[ignore]`，注释即本登记）；
  将来做解耦时去掉 `#[ignore]` 即为红相→绿相门。

---

## 7. 归零验收用的一条命令（Lead 可直接钉进门禁）

```bash
cd /home/dm/pony-agent && ! grep -rnE "MIN_CLIENT_GAP|thread::sleep|\.sleep\(" \
  crates/pony-agent-core/src/agent/runtime/turn_stream.rs \
  crates/pony-agent-core/src/agent/runtime/tool_recovery.rs \
  crates/pony-agent-core/src/agent/provider/openai_sse.rs \
  | grep -vE ':[0-9]+:[[:space:]]*//'
```
- 实测：当前工作区 **Exit Code 0**；`git HEAD` 基线（R1）有 21 行命中 ⇒ 基线 Exit Code 1。
- 语义：末尾过滤器只剔除**行注释**（三份源码里 `thread::sleep` 仅出现在「旧实现曾 sleep」的注释中）；
  `!` 使「无命中」= Exit 0。等价 Rust 用例：`f3_sse_read_path_has_no_blocking_throttle`。

---

## 8. 纪律与证据清单

- 只写上述 4 个测试文件 + 2 处**既有用例期望**（§9，`#[cfg(test)] mod tests` 内，Lead 指派）+ 本收据 + `raw/` 日志；
  **未修改任何业务实现体**（无 `src/` 非 test 代码改动）、**未改 git 状态**（无 stash/checkout/reset/commit/clean）；
  只跑 `-p pony-agent-core --lib` 与 `--test f1/f2/f3` 三个目标（未跑全仓 `cargo test` / `npm run verify`）。
- 全部 F2/F3 链路使用 `127.0.0.1:0` 回环 mock（零外网、零真实 provider）；无 `#[ignore]` 充当门禁用例（唯一 `#[ignore]` 已明示 ADVISORY 并经 Lead 裁决）。
- 临时文件清理：每个用例结束 `remove_file` 会话库（`/tmp/pony-f{2,3}-*`）与临时日志；mock 线程由 `Drop` 唤醒 join。
- 原始日志（可复查逐行输出）：
  - `.dev-team/diag-tps/fix/raw/f1.log`、`raw/f2.log`、`raw/f3.log`、`raw/f3_advisory_ignored.log`、`raw/f1_ts.log`

---

## 9. 追加：F1 契约变更引发的既有用例期望更新（Lead 指派，2026-11-12 20:1x）

### 9.1 变更性质

`TurnEvent::TurnSpeed`（`type = turn/speed`）是 F1 I1 引入的**新落盘事件**，派发位置固定：
`turn_flow::emit_event` → `settle_turn_speed_evidence`（先）→ `build_turn_end_event`（后，terminal 触发 flush）。
因此两条既有「事件序列精确相等」用例的期望必须把 `turn/speed` 作为**必须存在的显式一条**加入，
且保留原有顺序断言（**未**放宽成 `contains` / 「忽略多出事件」）。

| 文件 | 位置 | 原期望（≤ `step/end`） | 新期望 |
|---|---|---|---|
| `crates/pony-agent-core/src/agent/runtime/tests.rs` | `run_turn_and_start_turn_stream_produce_equivalent_event_sequences`（expected skeleton，约 L4278） | `… provider/usage, step/end, turn/end` | `… provider/usage, step/end, turn/speed, turn/end` |
| `crates/pony-agent-core/src/agent/control_plane/mod.rs` | `graph_sync_run_turn_persists_event_stream_end_to_end`（`expected_types`，约 L7991） | `… provider/usage, step/end, turn/end` | `… provider/usage, step/end, turn/speed, turn/end` |

两处均按要求写了注释：`契约变更：F1 I1 新增 turn/speed 事件（见 .dev-team/diag-tps/fix/contract-v3.md）`，
并额外注明「位置是契约的一部分：必须落在 terminal `turn/end` 之前」。

### 9.2 红相原文（修期望**之前**的物理证据，`raw/lib_seq_red.log` / `raw/lib_cp_red.log`）

```
thread 'agent::runtime::tests::run_turn_and_start_turn_stream_produce_equivalent_event_sequences'
  panicked at crates/pony-agent-core/src/agent/runtime/tests.rs:4267:5:
assertion `left == right` failed: completed skeleton shape
  left: ["turn/start", "context/observation", "user/message", "step/start", "assistant/message", "provider/usage", "step/end", "turn/speed", "turn/end"]
 right: ["turn/start", "context/observation", "user/message", "step/start", "assistant/message", "provider/usage", "step/end", "turn/end"]
test result: FAILED. 0 passed; 1 failed; 0 ignored (SEQ_EXIT=101)

thread 'agent::control_plane::tests::graph_sync_run_turn_persists_event_stream_end_to_end'
  panicked at crates/pony-agent-core/src/agent/control_plane/mod.rs:8063:9:
assertion `left == right` failed: event stream persisted end to end through the production channel
  left: [... "step/end", "turn/speed", "turn/end"]
 right: [... "step/end", "turn/end"]
test result: FAILED. 0 passed; 1 failed; 0 ignored (CP_EXIT=101)
```

修期望后（同一次全量运行内）：两条均 `... ok`（`grep -c "... ok" raw/lib_full_green.log` = 1 / 1）。

### 9.3 全量 lib 门禁（`cargo test -p pony-agent-core --lib`）

| 轮次 | 命令 | 结果 | Exit Code | 原始日志 |
|---|---|---|---|---|
| 第 1 轮 | `cargo test -p pony-agent-core --lib`（exec-core 仍在编译，机器高负载） | 1523 passed / **4 failed** | 101 | `raw/lib_full_green.log` |
| 第 2 轮 | 同命令（负载空闲后） | **1527 passed / 0 failed** | **0** | `raw/lib_full_green2.log` |

第 1 轮 4 条失败**与本轮修复无关**，均为负载敏感既有用例（逐条单独运行全绿，Exit 0）：

| 用例 | 失败表现（高负载） | 单独运行 |
|---|---|---|
| `agent::runtime::tests::run_turn_parallel_tools_keeps_run_out_of_parallel_segment` | 并发段峰值 1（期望 2） | ok（0.96s） |
| `agent::tools::tests::run::run_command_returns_stdout_and_exit_code` | stdout 不含 `hello` | ok（0.02s） |
| `agent::tools::tests::run::run_command_executes_when_a_registered_sandbox_backend_is_available` | stdout 不含 `hello` | ok（0.03s） |
| `agent::tools::tests::web::pinned_chain_exceeds_aggregate_deadline_and_is_cut_short` | 命中 0 次（期望 3，deadline 预算被打满） | ok（1.23s） |

（判定依据：`--exact` 过滤空跑得到的"ok 0 passed"不算证据，上表是**带完整测试路径过滤的真实单测运行**。）

### 9.4 本轮四组门禁的最终 Exit Code（全部为物理收据）

```bash
cargo test -p pony-agent-core --lib                                # Exit Code: 0    (1527 passed / 0 failed)
cargo test -p pony-agent-core --test f1_speed_observability        # Exit Code: 0    (4 passed)
cargo test -p pony-agent-core --test f2_retry_window_purity        # Exit Code: 0    (2 passed)
cargo test -p pony-agent-core --test f3_stream_throttle_isolation  # Exit Code: 0    (2 passed / 1 ignored=ADVISORY)
npx vitest run src/lib/runtime/__tests__/f1-token-metrics-sample.spec.ts  # Exit Code: 0 (6 passed)
# 静态门：Exit Code: 0（见 §7）
```
