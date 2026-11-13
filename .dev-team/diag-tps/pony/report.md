# pony-agent 侧 t/s 取证报告（只读）

- **取证对象**：`/home/dm/pony-agent`（Rust core + Vue）、`~/.local/share/PonyAgent/sessions.db`、`~/.pony-agent/tui-logs/tui-runtime.log`
- **取证时间**：2026-11-12 18:42–19:05 (+08:00)
- **代码基线**：HEAD `23091fa3`，**工作区脏**（`git status --porcelain` 43 项；取证期间 `turn_flow.rs` mtime 仍在变动）。
  关键文件已冻结快照到 `snapshot/`（附录 A），**`snapshot/` 前缀的行号指冻结副本**，其余指工作区当前内容。
- **只读声明**：未修改任何源码/配置/git；写操作仅限 `.dev-team/diag-tps/pony/`。
- **复算脚本**：`recompute.py`、`recompute2.py`（可复跑），原始输出 `recompute2.out.txt`。

---

## 摘要（结论表）

| # | 结论 | 严重度 | 证据指针 |
|---|------|--------|----------|
| C1 | **pony 显示口径 = 扣 TTFT 的解码速率**：`Σ_eligible(out−1)/(Σ_eligible gen/1000)`，`gen=dur−ttft`，过物理下限 `max(200ms,5%dur)`；无 eligible 才回退 `Σout/Σdur` 且带 `≈`。TUI 与桌面端**同源同式**，均不本地另算。 | 口径本身无缺陷 | `snapshot/turn_flow.rs:2416,2446,2461,2545-2578`；`snapshot/metrics_snapshot.rs:226,283-284`；`snapshot/tui.rs:4149-4157`；`token-metrics.ts:321,421,484,504,771`；`HomeStatusPanel.vue:225-229` |
| C2 | **不存在「usage 全量 token ÷ 墙钟」**：轮级墙钟（含工具执行的多跳 elapsed）只写 `agent.turn.duration_ms`，不参与任何 t/s 分母。「多调用加权聚合」存在，但按 eligible 集合加权，不是整轮墙钟。 | 无 | `snapshot/turn_flow.rs:2628-2654`（只用 per-call records）；`otel/mod.rs:2406-2412`（墙钟只落 span） |
| C3 | **`turn_speed_basis` 实际取值无法直接取证**：DB 无该列；`trace_data`/`turn_events` 中所有 `turnSpeedBasis` 命中均为对话正文（契约文本）；日志 0 条 speed 记录；OTel 导出全失败（401/timeout）。**按同源公式派生**：全库 440 stream_decode / 67 end_to_end / 74 none；gemini 110 / 15 / 8。 | 取证盲区（中） | 附录 D |
| C4 | **三口径复算（gemini 变体：133 轮 / 2822 调用 / 2800 有效 / 579 可解码）**：decode **98.6**、end_to_end **10.4**、整轮墙钟 **9.5** t/s；ttft 缺失 **621/2822（22%）**。三者差 10×＝用户观感来源。 | 高（口径敏感） | 附录 B |
| C5 | **契约自述数字仅部分可复现**：decode 143.9 ≈ 契约 141.5（一致）；契约「旧口径 50.0 / 端到端 27.6」在当前库为 **103.6 / 59.3**（样本 1998 → 16803 条）。方向性成立，绝对值不可当结论引用。 | 中（文档漂移） | 附录 B.4 |
| C6 | **口径级真实缺陷：per-call 窗口把「失败重试」算进 dur 与 ttft**。`initial_decision_started_at`/`provider_call_started_at` 在**重试循环之外**取样，而重试（`attempt=1..5`，429/525，单次 1.9–8.0s）在循环内 ⇒ 每次调用的 `dur` 含全部失败尝试。decode 口径近似无恙（`dur−ttft` 抵消首部），**end_to_end 口径被系统性压低**（gemini e2e 10.4 vs decode 98.6）。 | 高（口径） | `snapshot/turn_stream.rs:978,1164,2174,2342`；`provider/mod.rs:5436-5444,5793`；附录 E.3 |
| C7 | **客户端 16ms 节流确实存在，但生产未成为瓶颈**：`on_delta` 在单线程阻塞读循环内联执行，节流用 `thread::sleep(≤16ms)` 阻塞读循环（6 处），仿真上限 ≈124 emits/s（C→0）。但实测 delta 节拍仅 **0.06–1.75 次/s**（整轮），远未触顶 ⇒ **不能**用它解释 60-70 vs 200+。 | 中（结构风险） | `snapshot/turn_stream.rs:1078-1084,1156-1163,2283-2286,2333-2336`；`snapshot/tool_recovery.rs:212-218,242-246`；`openai_sse.rs:289`；`responses_api.rs:92`；附录 E.2/E.4 |
| C8 | **前端/IPC 不构成背压**：Tauri `emit` fire-and-forget；delta 只做字符串缓冲 + 120ms/120 字刷新；markdown 有已定稿块缓存；事件落盘只在终态批量 flush；TUI 用无界 channel。⇒ UI 慢只掉帧，不压上游速率。**但**存在 per-chunk 同步 stderr 日志（`usage:openai raw=null`，全量 446 万行 / 548MB 无轮转）与轮末 3s 同步落盘。 | 低-中（I/O 噪声） | `tauri_adapter.rs:19-24`；`runtime.ts:4045,989-1009`；`constants.ts:21-22`；`MarkdownRenderer.vue:271-284`；`control_plane/mod.rs:1200-1215`；`provider/usage.rs:9`；附录 E.5/E.6 |
| C9 | **请求体差异候选**：pony 发 `model="antigravity/gemini-3.8-flash"`、`max_tokens=64000`（网关清单 `max_output=32768`）、`temperature=0.2`、`stream:true`、`stream_options.include_usage:true`、恒带 `tools`（实录 36 个）+`tool_choice:"auto"`、**无任何 thinking/reasoning 字段**（preset=open-ai-chat ⇒ pattern=None，实测日志 25203 次 `pattern=None supports_reasoning=false`）。dsh 的 `gemini-3.8-flash-high` 探针显示落到**同一 antigravity key 池**（`No available key for provider 'antigravity'`）。 | 中（路由/参数） | `~/.config/pony-agent/providers.json`；`provider/mod.rs:2413-2440,4155-4201`；`config.rs:965,1837-1871`；附录 C |
| C10 | **上游侧主导项是 TTFT**：gemini per-call ttft p50 **11.16s** / p90 **29.5s**；**73%（1619/2223）** 有 ttft 的调用落在 `gen<5%dur`（「尾部/整包到达」签名）；该模型今天触发 `provider_cooldown_entered retry_after_s=150318` + `short_circuit`。 | 高（上游） | 附录 B.5；`pony-agent.log:1483-1484,1520-1523` |
| C11 | **用户报的 60-70 落在 gemini 显示值的下四分位**（非系统性偏置）：近 24h 35 轮中 27 轮 stream_decode（p10 **31.2** / p50 **91.3** / p90 **126.8**），8 轮 end_to_end（中位 5.2，带 `≈`）。 | 观察 | 附录 B.5 |

**一句话判定**：pony 的**显示公式**不是「把分母撑大的错误聚合」（C1/C2/C3 已排除）；三口径本身差 10×（C4），**其中 decode↔e2e 的差主要由「重试并入 per-call 窗口」这一口径缺陷造成**（C6）；16ms 客户端节流结构上真实存在但生产未触顶（C7）。因此 **「pony 60-70 vs dsh 200+」不能由 pony 侧公式解释**，需在 (a) dsh 侧分母定义、(b) 网关路由/参数（model 后缀、max_tokens 64000 vs 32768、thinking 档）两者中定位；pony 侧能确证的是「解码速度中位 ~91–106 t/s、p90 ~127–161，60-70 属其低分位」。

---

# 附录 A：基线与快照

```
$ cd /home/dm/pony-agent && git rev-parse HEAD   → 23091fa3…（工作区 43 项未提交）
$ sha256sum <关键文件>                            → snapshot/ 副本（2026-11-12 18:48 冻结）
  d0ca7cd9… turn_flow.rs           d7e6d0bf… metrics_snapshot.rs
  82c7cf00… runtime/turn_stream.rs c5ea7fda… bin/tui.rs
  a8fe67f8… src/lib/runtime/token-metrics.ts    a9844cdc… HomeStatusPanel.vue
```
`turn_flow.rs` 在取证期间被外部改动（`resolve_turn_speed` 由 ~2820 行移至 2545 行），故**行号一律以快照为准**。

# 附录 B：同库复算（决定性证据）

## B.1 复跑
```
$ cd /home/dm/pony-agent/.dev-team/diag-tps/pony && python3 recompute2.py; echo EXIT:$?
… EXIT:0
```
`sqlite3` CLI 本机不存在（Exit 127），改用 python3.12 / sqlite 3.45.1，`file:…sessions.db?mode=ro` 只读打开。

## B.2 全库 / 按 model（`legacy` = Σ_valid out / Σ_valid gen，即 v2 契约 §0 的「旧口径」）
```
== 全库汇总 ==
ALL                          turns= 581 calls= 16803 valid= 16739 elig= 9447 ttft_missing= 5447 | legacy= 103.6 decode= 143.9 e2e=  59.3

== 按 model 分组 ==
workbuddy/deepseek-v4.1-flash        turns= 285 calls= 10358 valid= 10338 elig= 6873 ttft_missing= 3435 | legacy= 118.6 decode= 163.3 e2e=  77.0
trae-cn/deepseek-v4.1-flash          turns=  17 calls=  1139 valid=  1139 elig=  562 ttft_missing=  477 | legacy= 143.0 decode= 152.6 e2e= 129.9
magpie/trae-cn/deepseek-v4.1-flash   turns=  32 calls=  1268 valid=  1264 elig=  615 ttft_missing=  540 | legacy= 111.3 decode= 137.0 e2e=  88.0
antigravity/gemini-3.8-flash         turns= 114 calls=  2754 valid=  2740 elig=  562 ttft_missing=  571 | legacy=  38.0 decode=  98.1 e2e=  10.3
qoder-cn/lmodel                      turns=  21 calls=   569 valid=   569 elig=  429 ttft_missing=  140 | legacy=  66.4 decode=  71.5 e2e=  56.9
kimi/k3.1-pro                        turns=   4 calls=   155 valid=   154 elig=   76 ttft_missing=   78 | legacy=  48.2 decode=  57.5 e2e=  40.9
deepseek/deepseek-v4.1-flash         turns=   9 calls=   121 valid=   120 elig=   91 ttft_missing=   30 | legacy= 154.6 decode= 168.8 e2e= 124.2
sense/deepseek-v4-flash              turns=  10 calls=   115 valid=   109 elig=   80 ttft_missing=   35 | legacy=  68.5 decode=  77.3 e2e=  38.2
sense/deepseek-flash                 turns=  14 calls=    89 valid=    87 elig=   48 ttft_missing=   40 | legacy=  57.7 decode=  99.9 e2e=  35.6
qoder-cn/dfmodel                     turns=   6 calls=    85 valid=    85 elig=   60 ttft_missing=   23 | legacy=  75.1 decode= 127.4 e2e=  63.4
opencode-zen/deepseek-v4.1-flash-free turns=  2 calls=    57 valid=    56 elig=   29 ttft_missing=    8 | legacy=  69.0 decode= 178.8 e2e=  20.2
gemini-3.8-flash                     turns=  19 calls=    68 valid=    60 elig=   17 ttft_missing=   50 | legacy=  32.9 decode= 118.4 e2e=  23.8
antigravity/claude-opus-4-6-thinking turns=  13 calls=    24 valid=    17 elig=    4 ttft_missing=   20 | legacy=  39.3 decode=  65.8 e2e=  33.0
```

## B.3 用户所问模型（gemini-3.8-flash 全部变体）
```
turns=133 calls=2822 valid=2800 elig=579 ttft_missing=621
decode      = (Σout_elig − elig) / (Σgen/1000) = (222713 − 579) / (2252692/1000) =  98.6 t/s
end_to_end  = Σout / Σdur                      =  505697 / (48616056/1000)        =  10.4 t/s
turn_wall   = Σturn_out / Σturn_dur            =  499803 / (52460857/1000)        =   9.5 t/s
```

## B.4 契约自述数字独立校验
| 契约（400 轮/1998 调用） | 本次（581 轮/16803 调用） | 判定 |
|---|---|---|
| 旧口径 50.0 | `Σout/Σgen` = **103.6**；per-call 均值 3945.8（被 gen≈1ms 突发包污染）；轮墙钟 35.4 | 不可复现（样本 8.4×，且旧口径无唯一数学定义） |
| 仅有效流式 141.5 | decode **143.9** | 一致（1.7%） |
| 端到端 27.6 | e2e **59.3** | 不可复现（方向一致，绝对值随样本漂移） |

## B.5 gemini 分布事实
```
ttft None: 621/2822 (22.0%)
dur  p10/p50/p90/max = 4848 / 11278 / 35451 / 196668 ms
out  p10/p50/p90/max =   36 /    63 /   404 /   4616 tok
ttft p10/p50/p90/max = 5561 / 11160 / 29482 / 192904 ms        ← TTFT 中位 11.2 s
decode(eligible per-call) p10/p25/p50/p75/p90 = 30.4 / 61.6 / 95.7 / 130.9 / 164.1
gen<5%dur 的调用：1619/2223（73%）                              ← 「尾部/整包到达」签名
provider_followup_local_fallback 且 dur≈180s：12 条（超时兜底）
近 24h 逐轮显示值（n=35）：stream_decode 27 轮 p10 31.2 / p50 91.3 / p90 126.8；end_to_end 8 轮中位 5.2
```

# 附录 C：请求体（C9）

`~/.config/pony-agent/providers.json`（ponyllm 项）：
```
protocol=openai-completions  base_url=https://tokens.ponyjob.top/v1
model="antigravity/gemini-3.8-flash"  temperature=0.2  maxOutputTokens=64000
capabilityPreset=open-ai-chat ⇒ thinking_param_pattern=None
capabilities: contextWindow 262144, supportsTools true, supportsStreaming true, supportsReasoning false
upstream_policy: request_timeout_ms=45000, decision_max_attempts=5, retry_budget_ms=30000
```
代码模板（`provider/mod.rs:2422-2440`，followup-stream；初始流同构）：
```json
{"model":"antigravity/gemini-3.8-flash","messages":[…],"temperature":0.2,"max_tokens":64000,
 "stream":true,"stream_options":{"include_usage":true},"tool_choice":"auto","tools":[…]}
```
**线上实测印证**（`~/.pony-agent/tui-logs/tui-runtime.log`，548MB，18:55 仍在写）：
```
$ grep -a -c -F '"max_tokens":64000'                        → 25092
$ grep -a -c -F 'with_openai_request_options pattern=None'  → 25203   （含 supports_reasoning=false）
$ grep -a -c -F 'model=antigravity/gemini-3.8-flash'        → 6807
$ grep -a -c -F '525 <unknown status code>'                 → 30
实录（session 日志内嵌 provider 日志）：
  request:openai followup-sync-batch body_preview={"max_tokens":64000,"messages":[{"content":"You are Pony Agent, …
  with_openai_request_options pattern=None supports_reasoning=false reasoning_effort=None
```
另有别名路径实录：`endpoint=http://10.42.0.67:8080/v1/chat/completions model=gemini-3.8-flash`（另一 base_url）。
⇒ 确认：`max_tokens=64000` 恒定、**无 thinking/reasoning 字段**、`supports_reasoning=false` ⇒ `temperature` 不被剔除（`mod.rs:4195-4199` 只在 supports_reasoning 时删）；走 `openai_sse.rs`（`/chat/completions`），**不**走 `responses_api.rs`（该 provider 的 responses endpoint `enabled:false`）。

**与 dsh 的候选差异**：
1. model 串：`antigravity/gemini-3.8-flash`（显式 provider 前缀、无思考档）vs `gemini-3.8-flash-high`（网关别名 + 思考档 high）。探针显示二者**同池**：`-high` 报 `No available key for provider 'antigravity'`；pony 日志同期 `provider_cooldown_entered model=antigravity/gemini-3.8-flash retry_after_s=150318`。
2. `max_tokens=64000` vs 网关模型清单 `max_output=32768` → 是否钳制/触发重试 **[UNVERIFIED]**。
3. pony 恒带 36 个 tools + 巨量 system 前缀（实录一条 Developer 消息 +34312 chars）⇒ prefill 大、TTFT 高；dsh 若为纯文本则无此开销。
4. `include_usage:true` 两边一致，但 pony 侧 `usageOrigin=provider` 2314/2822、`None` 497、`estimated_chars` 11 —— 无 usage 的调用进不了 I1，直接不参与速度。

# 附录 D：`turn_speed_basis` 取值取证（C3）

```
sqlite3 CLI: command not found (Exit 127) → python3
pragma table_info(normalized_turn_traces): 无 basis / speed 列
逐表扫描 '%turnSpeedBasis%'：session_turn_traces.trace_data 82、turn_events.payload 249、
  normalized_sessions/normalized_messages/normalized_history_nodes 等
  —— 抽样上下文全部是**对话正文**（任务里贴的契约文本），非指标字段
$ grep -c "speed" ~/.local/state/pony-agent/logs/pony-agent.log   → 0   (Exit 1 = 无匹配)
$ grep -c "pony-provider" 同上                                     → 0   （provider_log 走 stderr，未落该文件）
OTel 根 span 是唯一出口，而导出持续失败：日志开头 30+ 条 401、`OTLP 导出失败（2/2 次尝试后放弃…TimedOut）`
```
**派生分布**（`recompute2.py`）：
```
全库 basis: stream_decode 440 / end_to_end 67 / none 74
gemini   : stream_decode 110 / end_to_end 15 / none 8
轮级速度分位 stream_decode n=440 p10=67.4 p25=109.0 p50=150.0 p75=169.9 p90=188.0 max=293.1
             end_to_end   n= 67 p10= 4.2 p25= 17.6 p50= 51.0 p75= 88.6 p90=107.8 max=149.7
```

# 附录 E：客户端链路

## E.1 读循环单线程阻塞 + 回调内联
`responses_api.rs:92` `block_on(async { tokio::time::timeout(poll_tick, stream.next()).await })`；`openai_sse.rs:289` `acc.push_payload(data, on_delta)?`；`push_payload` 内 `on_delta(ProviderStreamChunk::Text(..))`（`openai_sse.rs:133-139`）⇒ 回调内任何阻塞 = 停止读 socket = TCP 背压。

## E.2 六处 16ms 节流（睡眠在读循环线程）
`snapshot/turn_stream.rs:1078-1084`（followup 正文/思考）、`1156-1163`（followup 思考尾刷）、`2283-2286` 与 `2333-2336`（initial 通道）；`snapshot/tool_recovery.rs:212-218,242-246`（恢复通道）。语义：`gap<16 ⇒ sleep(16-gap)`，`set(now)` 用 sleep 前时点 ⇒ 隔次才再睡。
仿真上限 ≈ `2 emits/(16ms+2C)`：C→0 ⇒ 123.5/s；C=1ms ⇒ 117.7；C=3ms ⇒ 105.4；C=8ms ⇒ 62.5。
无 ADR/注释解释该常量（`git log -S MIN_CLIENT_GAP_MS` 仅命中引入提交 `301e5d06` "time-based throttle" 与重构 `aeaad4a7`），**无任何测试覆盖节拍**。

## E.3 口径缺陷：重试被算进 per-call 窗口（C6）
`snapshot/turn_stream.rs:2174` `let initial_decision_started_at = Instant::now();` 在 `provider_decision_stream_guarded_with_mode`（同文件 ~2199）**之前**；`provider_call_started_at`（978）同理。重试在调用内部（`provider/mod.rs:5436-5444`：`max_retries = decision_max_attempts−1 = 4`、`retry_budget_ms=30000`；日志格式串见 5793 行 `{}:attempt={}/{} failed`）。实录：
```
decision_stream:attempt=1/5 failed class=TransientRetryable { "429 Too Many Requests；耗时=1951ms" }
decision_stream:attempt=2/5 failed class=TransientRetryable { "429 Too Many Requests；耗时=2715ms" }
followup_sync_batch:attempt=1/5 failed class=NonRetryable { "provider 返回错误状态：525 <unknown status code>；耗时=8036ms" }
```
⇒ `ttft` =「首次尝试开始 → 首次拿到内容」，`dur` =「首次尝试开始 → 全部尝试结束」。decode 用 `dur−ttft` 大致还原末次尝试；**e2e 用整段 dur，把失败尝试与退避一起当分母** ⇒ e2e 被系统性压低（gemini 10.4 vs decode 98.6；8 轮显示 `≈5 t/s`）。

## E.4 节流在生产未触顶（实测 delta 节拍）
从 `tui-runtime.log` 按 turn id 计 `emit turn:delta` 行数，与 DB 的 Σdur / Σout 对照：
| turn | 模型 | deltas | Σdur | Σout | delta/s | tok/delta |
|---|---|---|---|---|---|---|
| b6e0c099… | magpie/trae-cn/deepseek-v4.1-flash | 169 | 450.3s | 23084 | 0.38 | 137 |
| fee8cd57… | 同上 | 778 | 444.9s | 21904 | 1.75 | 28 |
| d51afe98… | antigravity/gemini-3.8-flash | 4 | 68.9s | 400 | 0.06 | 100 |
⇒ 实际 emit 频率比节流上限低 2–3 个数量级（原因：思考增量按 96 字符批处理 `stream_support.rs:5,760-770`，且上游 chunk 本身粗粒度）⇒ **节流不是 60-70 的成因**，仅作结构风险登记。

## E.5 per-chunk 同步 stderr 日志 + 轮末同步落盘（真实 I/O 成本）
`provider/usage.rs:9`（该文件本身在 dirty 树里为**未跟踪新文件**）在每个 SSE payload 上 `provider_log!`（`eprintln!`）；OpenAI 规范下 `include_usage` 的每个 chunk 都带 `"usage":null` ⇒ 每 chunk 一次 stderr 写：
```
$ grep -a -c -F "usage:openai raw=null" tui-runtime.log → 4463542
$ grep -a -c -F 'usage:openai raw={'   tui-runtime.log → 31121
日志文件 548MB（未轮转，最后写入 18:55）
实录：commit_boundary_committed: label=save_to_backend session_count=203 elapsed_ms=3008
```
⇒ 轮末**3.0s 同步落盘**（1.25GB DB + 1.5GB WAL），影响轮墙钟而非读循环。

## E.6 无背压环节（逐项排除）
| 环节 | 结论 | 证据 |
|---|---|---|
| Tauri IPC | fire-and-forget，不等待 JS | `src-tauri/src/tauri_adapter.rs:19-24` |
| 前端 delta | 纯字符串缓冲；trace 走节流 | `src/stores/runtime.ts:4045-4122` |
| 前端刷新 | ≥120ms 或 ≥120 字 flush（≈8 次/s） | `constants.ts:21-22`；`runtime.ts:989-1009` |
| markdown | 已定稿块缓存 + 仅尾巴重渲 | `MarkdownRenderer.vue:271-284,296-333` |
| DB 写 | 内存缓冲，仅终态批量 flush | `control_plane/mod.rs:1200-1215` |
| TUI 通道 | 无界 `channel()` | `snapshot/tui.rs:3049-3051`；`tui.rs:7023,7153` |

# 附录 F：未验证项 [UNVERIFIED]

1. **[UNVERIFIED] dsh 侧分母定义**：200+ 是否扣 TTFT、是否按 chunk 计 token、是否「最近 N 秒窗口速率」（属 dsh 任务）。
2. **[UNVERIFIED] 线上请求字节**：无抓包；body 由代码模板 + `providers.json` + 日志 `body_preview` 拼合（body_preview 截断到 1600 字符，`stream`/`stream_options`/`temperature` 常在截断位之后）。
3. **[UNVERIFIED] `max_tokens=64000` 对网关 `max_output=32768` 的实际后果**（钳制/4xx/重试）——需网关日志。
4. **[UNVERIFIED] 用户看到的是哪一处 t/s**（TUI header / 桌面状态面板 / trace per-hop 三处同式不同集合）。
5. **[UNVERIFIED] 节流实测权重**：C（单 chunk 处理成本）未测；需 A/B（关节流或加 emit 计数埋点），超出只读取证边界。
6. **[UNVERIFIED] 采样偏差方向量化**：gemini 79%（2800→579）调用被 `decode_eligible` 剔除，剔除规则偏**抬高**估值；影响未量化。
7. **[UNVERIFIED] 实时日志无时间戳**（`tui-runtime.log` 为纯 stderr 文本），故无法反推「窗内瞬时速率」，delta 节拍只能按整轮平均。

# 附录 G：覆盖盲区

**看过**：`agent/turn_flow.rs`、`agent/metrics_snapshot.rs`、`agent/runtime/{turn_stream,turn_sync,tool_recovery,stream_support,types,trace_timeline}.rs`、`agent/provider/{mod,openai_sse,responses_api,usage}.rs`、`agent/otel/mod.rs`（速度段）、`agent/control_plane/mod.rs`（落盘段）、`agent/config.rs`、`bin/tui.rs`（指标行/事件 sink）、`src-tauri/src/tauri_adapter.rs`、`src/lib/runtime/{token-metrics,useTraceProjection,constants}.ts`、`src/stores/runtime.ts`、`src/components/{HomeStatusPanel,HomeTracePanel,MarkdownRenderer,HomeSidebar}.vue`、`sessions.db` 全表 schema + 581 轮 trace、`~/.config/pony-agent/providers.json`、`~/.local/state/pony-agent/logs/pony-agent.log`、**`~/.pony-agent/tui-logs/tui-runtime.log`（548MB 实录）**、`.dev-team/contract-matrix-token-speed-v2.md`、`.dev-team/diag-tps/upstream/raw/*`（旁证）。

**没看**：`agent/session/*` 与 `sqlite_session.rs` 的落盘实现（只看了 flush 时机）；`compaction_ops.rs`；`responses_api.rs` 全量（本 provider 不走）；`src/lib/chat/*` 渲染调度细部；`src-tauri` 其余命令层；网关侧路由实现（upstream 目录仅作旁证引用，未复核其脚本）；**未运行任何 cargo/前端测试**（会写 `target/`、`node_modules`，超出只读边界）；未复跑用户当时那次会话的实时读数（日志无时间戳）。

**方法学声明**：所有数值来自 DB 只读查询、日志只读 grep 或本目录脚本复算；未修改源码/配置/git；`snapshot/` 为防行号漂移的冻结副本。
