# dsh 的 t/s 口径取证报告（只读取证，未修改任何被检仓库文件）

取证对象：`/home/dm/deepseek-harness`（TypeScript，DSH harness）+ 运行中的 `~/.dsh` profile/会话日志（只读解压到 stdout）。
取证时间：2026-11-12 18:42–19:0x（Asia/Shanghai）。

---

## 0. 结论表

| 项 | 结论 | 证据 |
|---|---|---|
| 产出点 | **纯前端派生**，无 host 侧 t/s；只有两处：①会话输入框「活动」统计 pill（整会话聚合）②轨迹面板 assistant 单步的 Request Timing | StatsPills.tsx:166-169、TrajectoryTable.tsx:362-372 |
| 公式（会话 pill） | `tps = Σ_decodeTokens / (Σ_decodeMs/1000)`，即**加权和/加权和**（非逐次比值平均、非滑动窗口） | StatsPills.tsx:168、StatsPills.tsx:240 |
| 公式（单步面板） | `tps = outputTokens / ((completedTime - firstTokenTime)/1000)`，保留 1 位小数 | TrajectoryTable.tsx:366-370 |
| 分子 | **provider 上报的 `usage.outputTokens`**（= OpenAI 语义 `completion_tokens`，**已含 reasoning tokens**）；不是增量估计、不是字符数 | session-stats/projection.ts:159-162、token-meter/projection.ts:10-11、pi-ai openai-completions.js:1178-1185 |
| 分母 | **首 token 时间 → `assistant/message` 落地时间**（"decode 窗口"）。**排除 TTFT/prefill/排队** | projection.ts:156-163、turn-metrics.ts:39-41、records.ts:51-60 |
| 首 token 定义 | 第一个 **非空 text / reasoning / tool-call delta**；reasoning 与工具调用参数字节都算首 token | llm/src/assistant-stream.ts:250-260、312-317、333-342；records.ts:56 |
| 窗口类型 | 累计（Σ/Σ）平均；不是 chunk 间隔瞬时速率，也没有平滑/EWMA | projection.ts:151-166 |
| 排除工具等待 | **是**（每步 decode 窗口终点=该步 assistant/message；工具执行发生在下一步，落在窗口外） | projection.ts:145-166（按 step 关闭边界） |
| reasoning 剔除 | **无剔除**：首 token 识别显式包含 `reasoning-delta`；分子含 reasoning（provider completion_tokens）。但 pi-ai→harness 的 usage 映射**丢弃** `reasoning` 分量，故 dsh 无法单独报告 reasoning token | assistant-stream.ts:252-254、llm-pi-ai/stream.ts:19-31、pi-ai openai-completions.js:1178-1185 |
| UI 位置 | ①输入框 dock 的 `activity` pill：`1 turns 2 steps · 20 tok/s`（compact 模式只留 `20 tok/s`）②轨迹表 assistant 行 Request Timing 面板 `Throughput` 行 | StatsPills.tsx:240-273、TrajectoryTable.tsx:362-384、locale.ts:176 |
| 刷新频率 | 数值只在**步骤结算那一刻**变化（`assistant/message` 才累加）；客户端流式发布被合并到 3 个 rAF（约 50ms）但与本数值无关；跑动中的轮次不显示 t/s | projection.ts:145-166、assembly.ts:143-156、chat-view.client.spec.tsx:3297-3310（"withholds ttft and throughput while the turn is still running"） |
| 聚合域 | pill = **整个会话日志**（whole-log 投影，所有轮所有步）；轨迹面板 = **单个 model step / 单次请求** | StatsPills.tsx:45-49 + useSessionStats:160-164、TrajectoryTable.tsx:362 |
| 请求体（实际在跑） | `provider=ponyllm, api=openai-completions`，body：`model`、`messages`、`stream:true`、`stream_options:{include_usage:true}`、`max_completion_tokens`（仅当配置了 maxTokens 或 agent 设定） | ~/.dsh/profiles/web/cordis.patch.yml:34-44,94-106；pi-ai openai-completions.js:572-583,591-595,1265-1267 |
| reasoning effort | 默认**不发** `reasoning_effort`（web profile 未设 `agent-default-model.reasoningEffort`，profile 也无 `reasoning`），由上游默认 | adapter.ts:378-386、pi-ai openai-completions.js:713-720、profiles/web/cordis.patch.yml:31-35 |
| SSE 消费 | 逐 chunk 直处理（`for await` + `Date.now()` 打点），无时间节流/背压；仅做"delta 打包压缩（保留逐成员 dt）" | agent-loop/agent.ts:453-459、agent-loop/assistant-stream.ts:60-66、llm/src/assistant-stream.ts:113-127 |
| 与 pony-agent 可比性 | **不可直接比较**。dsh 是 decode-only 且无 eligibility 门槛；pony 的 `end_to_end`=Σout/Σdur（含排队+prefill）；pony 的 `stream_decode` 才是与 dsh 同域，但仍有分子/门槛差异 | pony-agent src/lib/runtime/token-metrics.ts:5-8,177-185 |

---

## 1. 产出点原文（含分子定义）

### 1.1 会话 pill（整会话聚合）

`packages/client/ui-chat/src/client/chat/StatsPills.tsx:166`
```ts
function decodeSpeed(stats: WindowStats, t: Translate): string {
  return t('message.tokensPerSecond', {
    tps: formatTokensPerSecond(stats.decodeTokens / (stats.decodeMs / 1_000)),
  })
}
```
`StatsPills.tsx:240`（无 decode 时间即整条不显示）：`const speed = stats.decodeMs > 0 ? decodeSpeed(stats, t) : null`
`StatsPills.tsx:245`：`joined(counts, speed)` → 按钮文本 `N turns M steps · X tok/s`；`aria-label` 同值（StatsPills.tsx:249）。

字段语义（StatsPills.tsx:30-40）：`decodeMs` = "Summed decode wall time over steps that also report output tokens"；`decodeTokens` = "Summed output tokens over the same decode-timed steps"。

数值权威来自 host 的持久化投影（前端窗口 fold 只是兜底）：
`packages/session/session-stats/src/projection.ts:145`
```ts
case 'assistant/message': {
  ...
  const firstToken = open.firstTokenTime ?? assistantStreamFirstTokenTime(event.data.stream) ?? null
  ...
  if (firstToken !== null) {
    next.ttftMs += Math.max(0, firstToken - open.startTime)
    next.ttftSteps += 1
    const outputTokens = usageOutputTokens(event.data.usage)
    if (outputTokens !== null) {
      next.decodeMs += Math.max(0, event.time - firstToken)   // 分母
      next.decodeTokens += outputTokens                        // 分子
    }
  }
```
`usageOutputTokens` 只读 `usage.outputTokens`（projection.ts:106-110）。兜底 fold 同一口径：StatsPills.tsx:74-81（`reading.decodeMs !== null && reading.outputTokens !== null` 才累加）。

### 1.2 单步面板（轨迹视图）

`packages/client/ui-trajectory/src/client/TrajectoryTable.tsx:362`
```ts
function throughput(metrics: AssistantMetricDetail, t: TrajectoryTranslate): string {
  if (!metrics.usageProvided) return t('timing.usageUnavailable')
  if (metrics.outputTokens === null) return t('timing.outputTokensUnavailable')
  ...
  const generationSeconds = (metrics.completedTime - metrics.firstTokenTime) / 1_000
  if (generationSeconds <= 0) return t('timing.durationTooShort')
  return t('unit.tokensPerSecond', { value: (metrics.outputTokens / generationSeconds).toFixed(1) })
}
```
面板行：`<dt>{t('timing.throughput')}</dt>`（TrajectoryTable.tsx:384），文案 `'{value} tok/s'`（ui-trajectory/src/client/locales.ts:67）。

### 1.3 分子到底算不算 reasoning

- dsh 侧 usage 契约：`packages/llm/token-meter/src/projection.ts:10-11` — "reasoning tokens are already included in `outputTokens` and are not accumulated again"。
- 上游映射：`packages/llm/llm-pi-ai/src/stream.ts:24-31` 的 `mapUsage` **只**映射 `input/output/totalTokens/cacheRead/cacheWrite`，**没有** `reasoningTokens`（`TokenUsage` 本身有该可选字段：packages/llm/llm/src/types.ts:188）。
- pi-ai 原始取值：`openai-completions.js:1178-1185` — `outputTokens = rawUsage.completion_tokens`，"OpenAI completion_tokens already includes reasoning_tokens"，`reasoning = completion_tokens_details.reasoning_tokens`（该值被 dsh 丢弃）。
- 结论：**分子含 reasoning，且与正文不可分离**；不是"只算正文 token"。

## 2. 分母 / 时间窗口

- 单步：`firstTokenTime → completedTime`，`completedTime` = `assistant/message` 事件时间（`packages/client/ui-chat/src/client/conversation-nodes/assistant.ts:207-209`；`stepStartTime` = 匹配的 `step/start` 事件时间）。
- 单步读数实现：`packages/client/ui-chat/src/client/contract/turn-metrics.ts:36`
```ts
const ttftMs = ... timing.firstTokenTime - timing.stepStartTime ...
const decodeMs = timing !== undefined && timing.firstTokenTime !== null
  ? Math.max(0, timing.completedTime - timing.firstTokenTime) : null
return { ttftMs, decodeMs, outputTokens: usageOutputTokens(node.usage) }
```
- **排除 TTFT**：是。TTFT 与 decode 是两个独立字段，t/s 只用后者。
- **排除工具等待**：是。decode 窗口按 step 关闭；`tool/call→tool/result` 单独计入 `toolMs`，不进入 decodeMs（projection.ts:167-181）。
- **不是滑动窗口、不是瞬时速率、无平滑**：只有两个累计量相加后相除。整会话是 `Σtokens / Σdecode`（加权），因此长 decode 步占更大权重。
- **进度语义**：`decodeMs` 的终点是"assistant/message 落地"，即最后一个 chunk 之后；含尾块与落盘延迟（毫秒级）。
- 特殊情形：中断/取消的步不 assemble message ⇒ 该步整体不计入（projection.ts:1-21 注释明确）。
- 首 token 可能在 retry 之前：`open.firstTokenTime` 只写一次（projection.ts:138-143），跨 attempt 保留 ⇒ 若第一步重试，TTFT 含首发尝试的等待，而 decode 仍从首 token 起算。轨迹侧注释同义（ui-trajectory/README.md:44）。

## 3. UI 显示与刷新

- 组件：`ActivityPill`（`data-composer-stat="activity"`，输入框 dock 中的 gauge pill），Compact 模式只留 speed（StatsPills.tsx:238-247），Detailed 模式可点开 dialog（`stats.dialog.speed`，StatsPills.tsx:268-273）。
- 文案：`meta`/`细` 两套 locale 的 `message.tokensPerSecond: '{tps} tok/s'`（ui-chat/src/client/locale.ts:176 与 :374）；dialog 中标签为「输出速度（TPS）」/「Tokens per second (TPS)」。
- 格式化：`formatTokensPerSecond`，≥10 取整、<10 保留 1 位（message-chrome.ts:67-75）；轨迹面板固定 1 位小数（TrajectoryTable.tsx:369-370）。**两处显示精度不一致**。
- 每条 vs 整轮：pill = 会话级（所有 turn 的所有 step 合并，Σ/Σ）；轨迹面板 = 单 step（单次模型调用）。dsh **没有**按 turn 聚合的 t/s 展示（全仓 `formatTokensPerSecond` 仅 StatsPills 一处消费：grep 全仓仅 StatsPills.tsx:17,168）。
- 刷新：`useSessionStats` = `useProjection('sessionStats')`，投影只在 `assistant/message` 时给 decodeMs/decodeTokens 增量 ⇒ pill 数字每步跳一次，流式期间不变；无 projection 时回退为窗口节点 fold（StatsPills.tsx:160-164）。运行中轮次不显示（chat-view.client.spec.tsx:3297-3310）。
- 客户端流式渲染节流（不影响数值，只影响画面）：`ui-conversation/src/client/conversation/assembly.ts:143-156` 跨 3 个 rAF 才 flush。

## 4. 发往 ponyllm 的请求体构造

调用链：`agent-loop` → `LlmService.generate` → `PiAiAdapter.stream` → `models.streamSimple` → pi-ai `buildParams` → `client.chat.completions.create(params)`。

- 适配器侧（`packages/llm/llm-pi-ai/src/adapter.ts:416`）：
```ts
const events = snapshot.models.streamSimple(model, context, {
  ...profileOptions(profile, reasoning, apiKey),
  ...options.temperature === undefined ? {} : { temperature: options.temperature },
  ...options.maxTokens === undefined ? {} : { maxTokens: options.maxTokens },
  ...options.sessionId === undefined ? {} : { sessionId: String(options.sessionId) },
  signal: watchdog.signal, headers: requestHeaders(profile.headers),
})
```
`reasoning` 由 `resolveReasoningLevel(model, options.reasoningEffort ?? profile.reasoning)` 得到（adapter.ts:378-386）；`'off'` 直接**省略**该字段（adapter.ts:132-135）。
- body 实际字段（pi-ai `buildParams`，`node_modules/.pnpm/@earendil-works+pi-ai@1.1.0…/dist/api/openai-completions.js`）：
```js
const params = { model: model.id, messages, stream: true, prompt_cache_key: ..., prompt_cache_retention: ... }  // :571-580
if (compat.supportsUsageInStreaming !== false) params.stream_options = { include_usage: true }               // :581-583
if (options?.maxTokens) { params.max_tokens | params.max_completion_tokens = options.maxTokens }             // :588-595
... params.reasoning_effort = thinkingLevelMap[effort] ?? effort                                            // :713-716
```
`model.id` 即配置里的 id 原文（含 `/` 前缀不加改写，如 `workbuddy/deepseek-v4.1-flash`）；`baseURL` 来自 profile。
- `maxTokensField` 判定：非 deepseek.com/chutes/moonshot/together 等 ⇒ `max_completion_tokens`（openai-completions.js:1238-1267，`supportsUsageInStreaming: true`）。pony 网关 `https://tokens.ponyjob.top/v1` 或 `http://10.43.141.157:8080/v1` ⇒ `max_completion_tokens`。
- `maxTokens` 何时出现：只有"部署配置的" 才会成为请求默认（adapter.ts:331-339：`configuredMaxTokens` 仅在 models 条目显式写了 `maxTokens` 时存在）；agent 层无 `AgentOptions.maxTokens` 时不会注入（core/agent/src/runtime-types.ts:34，全仓无默认赋值）。
- 实盘证据（只读解压 `~/.dsh/sessions/.../session.v4.jsonl.zstd` 的 `request/header`）：
  - `/home/dm`（headless profile）：`{"provider":"ponyllm","model":"gemini-3.8-flash","maxTokens":54000}` + `adapterDefaults:{maxTokens:true}`（对应 `~/.dsh/profiles/headless/cordis.patch.yml:52 maxTokens: 54000`）。
  - `/home/dm/pony-agent`（web profile，本 GUI 在跑）：`{"provider":"ponyllm","model":"workbuddy/deepseek-v4.1-flash","maxTokens":128000}` + `adapterDefaults:{maxTokens:true}`（`~/.dsh/profiles/web/cordis.patch.yml:35, 97`）。
  - 注意与 `~/.dsh/settings.yaml.imported:3-5` 的 `ponyllm / gemini-3.8-flash-high` **不一致**：那是导入快照；web profile 的 agent-default-model 已改为 `workbuddy/deepseek-v4.1-flash`，headless 为 `gemini-3.8-flash`，sdk 为 `gemini-3.8-flash-high`。`gemini-3.8-flash-high` 条目**无** `maxTokens`，故其请求体不带 max 字段。
- reasoning effort 实况：`workbuddy/deepseek-v4.1-flash` 声明了 `reasoningEfforts: {off: null, low, medium, high, max}`（profiles/web/cordis.patch.yml:101-106），但 `agent-default-model` 未设 `reasoningEffort`（:31-35），profile 也未设 `reasoning`，故 `options.reasoningEffort === undefined` ⇒ **不发送 `reasoning_effort`**（pi-ai 的 off 回填分支要求 `thinkingLevelMap.off` 为字符串，此处为 null）。
- `stream`：恒为 `true`（:574）。`stream_options`：恒为 `{include_usage:true}`（`supportsUsageInStreaming` 默认 true，:1265），dsh 未关闭。

## 5. SSE 消费：是否逐 chunk、有无合并/背压/节流

- 逐 chunk 处理：`packages/core/agent-loop/src/agent.ts:453-459`
```ts
const stream = preparedCall?.stream(request) ?? this.loopCtx.llm.stream(request)
live.start(); started = true
for await (const chunk of stream) { signal.throwIfAborted(); live.push(chunk) }
```
- 到达时间打点：`packages/core/agent-loop/src/assistant-stream.ts:60-66` — `this.accumulator.push({ time: Date.now(), chunk })`，即在**host 收到该 chunk 时**取时钟（不是 provider 事件时间戳；上游无逐 token 时间戳）。
- 合并（持久化压缩，非丢弃）：`packages/llm/llm/src/assistant-stream.ts:113-127` 把连续同类 delta 打包成 `text-chunks / reasoning-chunks`，但保存 `time0` 与逐成员 `dt`，因此能还原每个成员的毫秒时间（`runFirstTokenTime` 用 `time0 + Σdt` 重建，:306-317）。打包 `gap` 有上限校验（`safeGap`）。
- 背压/节流：请求路径**无**时间批量 flush；唯一合并是客户端渲染发布（3× rAF，assembly.ts:143-156）。
- 超时：`idleWatchdog(upstream, streamIdleTimeoutMs)`，默认 300000ms（llm-pi-ai/src/config.ts:47,344）；web profile 显式 `timeoutMs: 180000`。
- 重试：pi-ai `maxRetries: 0`（adapter.ts:142），重试由 harness 恢复层按 attempt 记录；首 token 在重试间保留。

## 6. 与 pony-agent 口径的可比性判定

pony-agent 真源（只读引用）：`src/lib/runtime/token-metrics.ts:5-8`
```
decode_eligible = valid 且 ttft 存在且 dur > ttft 且 out >= 2 且 gen >= max(200ms, 5% × dur)
轮级 speed = Σ(out−1) / (Σgen/1000)   (basis = stream_decode)
无 eligible 时 Σout / (Σdur/1000)      (basis = end_to_end)
```
`end_to_end` 定义见 token-metrics.ts:181-183（"含排队与 prefill，不可与前直接比较"）。

| 维度 | dsh | pony `end_to_end` | pony `stream_decode` |
|---|---|---|---|
| 分子 | `Σ outputTokens`（含 reasoning，无 −1） | `Σ out` | `Σ(out−1)` |
| 分母 | `Σ(firstToken→message)` | `Σ dur`（整请求墙钟） | `Σ gen`（≈ dur−ttft） |
| TTFT/prefill | 排除 | 含 | 排除 |
| 工具等待/多 hop | 排除（步间不计） | 单调用内不含，轮级按调用累计 | 同左 |
| eligibility 门槛 | **无**（仅 `decodeMs>0` / `generationSeconds>0`） | `dur>0 && out>0` | `out>=2 && gen>=max(200ms,5%dur)` |
| 聚合域 | 整会话（所有轮） | 轮/调用 | 轮/调用 |

判定：
1. **dsh 的 pill 与 pony 的 `end_to_end` 不可比**（分子同域，分母相差 TTFT/prefill 全部，且 dsh 无门槛）——同一批调用上 dsh 必然系统性偏高。
2. 与 pony `stream_decode` **同域但不等价**：dsh 不减 1（首 token 计费）、无 `gen>=200ms/5%` 与 `out>=2` 门槛 ⇒ 短 decode 步会给出爆炸值（见 §7 实测 10800 tok/s）。
3. dsh **不是**"只算正文 token"：thinking 流也计入首 token 与分子；更准确的描述是"**首 token 之后的纯解码窗口**，含 reasoning、含 tool-call 参数，排除 TTFT 与工具等待"。

## 7. 实盘抽样（只读重放会话日志，按 dsh 同规则计算，非 UI 截图值）

A. `/home/dm/deepseek-harness` headless 会话（`session-cb25864e…`，ponyllm / gemini-3.8-flash，19 步，1 轮）：
- 逐步 TTFT ≈ 3.7–16.4s（首 token 多为 tool-call），decode 窗口 10ms–5.25s。
- 单步峰值 `out=108 / decode 10ms ⇒ 10800 tok/s`（dsh 无门槛，UI 会照显）。
- 会话聚合：`Σout=1897, Σdecode=8157ms ⇒ 232.6 tok/s`；同批 `Σstep wall=…` 端到端仅约 11 tok/s 量级。

B. 本 GUI 在跑的 web 会话（`session-771804b7…`，ponyllm / `workbuddy/deepseek-v4.1-flash`，截读时 63/69 步，仍在追加）：
- 首 token 类别：`reasoning-run` 为主（亦有 `text-run`、`tool-call-run`）⇒ 分母窗口内包含 thinking 生成时间。
- 会话聚合：`Σout=22398, Σdecode=124301ms ⇒ 180.2 tok/s`；同一批 `Σ(step/start→assistant/message)=238581ms ⇒ 93.9 tok/s`（端到端上界，仍**未**含工具执行时间）。两者差 1.9×。
- 说明：dsh 的 180 tok/s 与 pony 的 `end_to_end` 在**同一流量**上会落在 90 tok/s 量级，差异来自口径而非机器。

## 8. 未验证项 `[UNVERIFIED]`

- `[UNVERIFIED]` 本 GUI（http://127.0.0.1:3080）当前进程究竟加载 `profiles/web` 还是其它 profile：由会话日志 `request/header` 反推为 web profile（`workbuddy/deepseek-v4.1-flash`），未检查进程命令行。
- `[UNVERIFIED]` `sessionStats` 投影从 host 到浏览器的推送频率/batching（未定位 transport 层）：只验证了"数值仅在 assistant/message 结算时变化"，未逐层确认推送是否另有节流。
- `[UNVERIFIED]` 真实线上 HTTP body 抓包：字段结论来自 `buildParams` 源码 + profile 配置 + 会话 `request/header`（`maxTokens` 由 `adapterDefaults` 标记为 adapter 默认），未抓 3080→ponyllm 的出站报文；`prompt_cache_key/retention` 是否实际出现取决于 `api.openai.com`/`supportsLongCacheRetention` 条件（本 profile 下预计不出现）。
- `[UNVERIFIED]` ponyllm 网关是否改写 `completion_tokens`（是否含 reasoning、是否含 tool-call 参数）：dsh 只是透传 provider 数字；网关上游（antigravity / workbuddy）的计数口径未取证。
- `[UNVERIFIED]` pony-agent 侧 `stream_decode` 与本报告的逐字段比对只读了 `token-metrics.ts` 的定义与注释，未运行其验收测试。
- `[UNVERIFIED]` 轨迹面板 `Request Timing` 的 UI 现网呈现（未开浏览器截图），结论来自单测固定值 `'20.0 tok/s'`（packages/client/ui-trajectory/tests/table.client.spec.tsx:223）与源码。

## 9. 盲区（搜过的关键词与目录）

- 关键词：`tokensPerSec`、`tokens_per_second`、`tok/s`、`tps`、`throughput`、`charsPerSec`、`speed`、`decodeMs`、`decodeTokens`、`ttft`、`firstTokenTime`、`stream_options`、`include_usage`、`reasoning_effort`、`max_tokens`、`max_completion_tokens`、`reasoningEffort`、`maxTokens`、`stream: true`、`requestAnimationFrame`、`throttle`、`coalesce`、`debounce`。
- 目录：`/home/dm/deepseek-harness` 全仓（`packages/**`、`apps/web`、`snapshots/web`、`docs`、`benchmarks`）＋ `node_modules/.pnpm/@earendil-works+pi-ai@1.1.0…/dist/api/openai-completions.js`（含 `patches/@earendil-works__pi-ai@1.1.0.patch`）＋ `~/.dsh/{settings.yaml.imported,profiles/*/cordis.patch.yml,sessions/**}`。
- 负结果：全仓无 host 侧 t/s 计算、无 `charsPerSec`、无滑动窗口/EWMA 平滑、无 server 指标 `tokens_per_second`（唯一 `throughput` 命中为 benchmarks 注释与 RPC 吞吐，均与本指标无关）。
- 未覆盖：桌面端（Electron/Tauri 壳）与 TUI profile 的展示差异；`apps/web` 之外的其它前端入口。
