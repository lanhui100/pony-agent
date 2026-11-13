# 诊断裁决：dsh 200+ t/s vs pony-agent 60-70 t/s（ponyllm · gemini-3.8-flash）

- 裁决：Lead。三路只读取证（上游实测 56 次请求 / pony 侧代码+本地库复算 / dsh 侧公式取证），**未改任何源码、配置、git 历史**。
- 取证窗口：2026-11-12 18:39–19:12 (+08:00)。证据原件：
  - [upstream/report.md](upstream/report.md)（raw/ 含 56 次 SSE 逐 chunk 时间戳）
  - [pony/report.md](pony/report.md)（含 recompute2.py 复算脚本 Exit 0、snapshot/ 冻结副本）
  - [dsh/report.md](dsh/report.md)

---

## 0. 判定（一句话）· r2（2026-11-12 19:20 修订）

**两个数都不是「模型真实速度」：真实可持续解码是 content 109–152 / out 106–127 t/s（n=26，0/26 达 200）。200+ 与 60-70 的差主要来自量法不对称：dsh 把「突发整包」（chunk 被攒成一包，单调用可算出 400–1521 t/s）计入 Σ/Σ，pony 用物理下限 `max(200ms,5%dur)` 把它们剔除 —— 同一批 2903 条真实记录换算：dsh 式 153.7 vs pony 式 99.9（1.54×）。叠加 ② 上游本身极不稳（TTFT p50 11.0s；18% 可解码调用 <45 t/s，gen 窗口内有 5–30s 停顿）与 ③ pony 侧把失败尝试/停顿计入窗口，逐轮就会落到 60-70。**

**r1 曾把「16ms 读循环节流」列为主因（预测 62.5 t/s），该预测已被实测否定**：9/9 条稳态 run 的 content chunk 到达间隔中位 21.0–42.8ms（>16ms），节流不咬合；只有 3.4% 的突发路由（间隔中位 0ms）会咬合，不足以解释常态。节流降级为结构风险（离 socket 读循环太近，一旦命中突发路由即硬顶 62.5）。

---

## 1. 三路证据交叉

| 事项 | dsh | pony-agent | 证据 |
|---|---|---|---|
| 显示公式 | `Σ usage.outputTokens ÷ Σ(首token → message 落地)` | `Σ_eligible(out−1) ÷ Σ_eligible(dur−ttft)`，门槛 `gen≥max(200ms,5%dur) && out≥2`；无 eligible 回退 `Σout/Σdur`（带 `≈`） | `StatsPills.tsx:166`、`projection.ts:145` / `turn_flow.rs` 快照 `2545-2578`、`tui.rs:4149-4157`、`HomeStatusPanel.vue:225-229` |
| 首 token 定义 | 含 reasoning-delta / tool-call-delta | 任意首个 delta | `assistant-stream.ts:250-260` / `pony/report.md §C1` |
| 突发包处理 | 无门槛，全计入 | 有物理下限，剔除 | 同上 |
| 同一批真实记录（gemini 2903 调用）换算 | **153.7 t/s** | **99.9 t/s** | 本文件 §4 复算（Lead 独立跑） |
| 会话级抽样 | 180.2 tok/s（同会话端到端 93.9，1.9×） | 逐轮 p10 31 / p50 91 / p90 127（近 24h n=35） | `dsh/report.md`、`pony/report.md` 附录 B |

**结论**：口径本身给不出 3×，只能给 1.5–2×；剩下的 1.5–2× 必须来自路由/节流。

---

## 2. 上游真值（决定性）

- 稳态（粗粒度路由，每 chunk 4–5 token）：**content 中位 127–135 t/s（区间 102–158）**，整段 out 116 t/s，端到端 52–66 t/s。
- **200+ 只在 10% 的「细粒度路由」出现**：1 token/chunk、prompt_tokens=67、TTFT 22s → **content 327 t/s / out 209 t/s**。同一 model id 同一时刻交替出现两套指纹 ⇒ 网关把请求路由到两套上游协议/账号。
- `gemini-3.8-flash-high`（dsh 默认 id）**8/12 次 429 quota_exhausted**（antigravity key 池枯竭）⇒ dsh 侧「200+」的样本本身处在配额抖动期，测速不可靠，必须记录 HTTP 码。
- `stream:false` 端到端 **55 t/s**——「60-70」在端到端口径下也能完全复现。

## 3. pony-agent 侧的三个真缺陷（不是量法，是行为）

1. **16ms/emit 同步 sleep 在阻塞 SSE 读循环里**（6 处：`turn_stream.rs:1078,1158,2283,2333`、`tool_recovery.rs:214,244`），无 ADR、无测试。稳态 = 62.5 emits/s；细粒度路由（1 token/chunk）下 = **62.5 token/s**，与用户读数吻合。`openai_sse.rs:289 → :133-139` 证明 `on_delta` 在读循环内联执行 ⇒ 睡眠 = 停止读 socket = TCP 背压。
2. **per-call 的 `dur`/`ttft` 取样点在重试循环之外**：失败尝试（429/525，1.9–8.0s）被计进窗口，`end_to_end` 被系统性压到 **10.4 t/s**（同批 decode 98.6）。
3. `max_tokens=64000` > 网关 `max_output=32768`；每 chunk 一条 stderr 日志（`usage:openai raw=null`，446 万行 / 548MB）；TTFT 中位 11.2s，73% gemini 调用是「整包到达」（被门槛剔除）。

**已排除**：前端/IPC/DB 落盘/markdown 重渲均无背压；TUI 通道无界；轮级墙钟不进任何 t/s 分母。

## 4. Lead 独立复算（同库同记录，可复跑）

```
gemini 全变体 2903 valid / 2166 有 ttft / 582 可解码
dsh 式  Σout/Σ(dur−ttft)            = 153.7 t/s
pony 式 Σ(out−1)/Σgen（含门槛）      =  99.9 t/s
pony 式 Σout/Σdur（端到端回退）      =  10.7 t/s
逐轮 stream_decode：p25 72 / p50 106 / p75 132   ← 用户 60-70 落在 p10–p25
```

## 5. 定案实验（按优先级）

1. **口径对齐实验（最必要）**：在 pony 侧同时打印两种数（现有 stream_decode + dsh 式的"含 burst 的 Σ/Σ"），用同一批调用验证 153.7 vs 99.9 的 1.54× 是否就是观感差主因。
2. **停顿归因**：18% 可解码调用 <45 t/s（如 out=254 / gen=30.5s = 8.3 t/s）——需要 chunk 级时间戳落盘，才能判定是「网关流内停顿」还是「重试被计入窗口」。当前 basis/speed **不落 DB、不落日志、OTel 401 全丢**，这是可观测性硬缺口。
3. **A/B（已部分证伪）**：`MIN_CLIENT_GAP_MS=0`。稳态 chunk 间隔 21–43ms ⇒ 预期无差异；仅在突发路由下有差异。

## 6. 修复方向（r2 排序）

1. P0 **口径诚实化**：显示端必须标注「样本/口径/burst 剔除数」，并禁止把含 burst 的 Σ/Σ 与逐轮 decode 混谈；`turnSpeedBasis` 落 DB + 日志（当前完全不可取证）。
2. P1 **窗口纯净**：确认 `dur`/`ttft` 是否跨重试累加（teammate 报告称在重试循环之外取样、429/525 的 1.9–8.0s 被计入 `[待复核]`）；若成立，移入重试循环，只计最后一次成功尝试。
3. P1 **节流离开读线程**（结构风险，非当前瓶颈）：定时器线程 coalescing 或 `try_send`+丢中间帧，避免突发路由下把 200+ 掐成 62.5。
4. P2 `max_tokens=64000` > 网关 `max_output=32768`；关掉 per-chunk stderr 日志（446 万行 / 548MB）。
5. P2 跨仓对齐：dsh 侧测速前记录 HTTP 码（`gemini-3.8-flash-high` 8/12 次 429）；pony 侧 TTFT 中位 11s 应在 UI 单独暴露。
