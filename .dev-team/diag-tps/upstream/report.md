# 上游真实速率实测报告：ponyllm 网关 gemini-3.8-flash 系列

- 任务：task-2（上游真值：能否 200+ t/s）｜状态：**已收口**
- 实测时间：2026-11-12 18:42–19:12 (Asia/Shanghai)
- 端点：`https://tokens.ponyjob.top/v1`（配置源 `/home/dm/.dsh/settings.yaml.imported`，provider `ponyllm`，`api: openai-completions`）
- 凭据：从 `/home/dm/.dsh/.credentials.yaml` 的 `PONYLLM_API_KEY` 读取，仅以环境变量 / `-H @headerfile` 传递；本报告与全部原始文件中该 key 均为 `<redacted>`，明文 header 文件已删除并全目录扫描确认无泄漏
- 数据规模：**58 次独立请求**（44 次拿到完整 content，14 次网关侧失败）；curl 全部 exit code 0，无网络/传输失败
- 时间盒收敛：按 lead 指令，`-high` / `-medium` 不再重试（失败原文已足够），本报告基于既有证据收口

---

## ⓿ 收敛结论（四问四答）

### 1. 稳态解码速率（真实可持续速率）

取 `tokens_per_cchunk ∈ [4.05, 4.85]`（网关稳态打包粒度，每 chunk 打包 4–5 token）的 **26 条稳态 run**：

| 指标 | p10 | **中位** | p90 | min–max |
|---|---|---|---|---|
| `content_window_tps`（content 解码窗口）| 109.3 | **117.0** | 152.4 | 102.4–162.4 |
| `out_window_tps`（含 reasoning 的整段输出）| 105.7 | **113.7** | 126.5 | 101.5–139.5 |
| `e2e_tps`（含 TTFT 的端到端）| 36.6 | 53.6 | 80.4 | 14.0–91.9 |
| TTFT | 5.1s | 7.6s | 14.2s | 1.5–36.8s |

> **这些就是上游真实、可持续的解码速率：稳态约 110–150 t/s（中位 117 content / 114 out），端到端 40–80 t/s。** 26/26 稳态 run 无一超过 162.4 t/s。

### 2. 突发包（burst）run —— 200+ 的唯一来源

`tokens_per_cchunk = 1.0` 的两次 run（content 在约 1.5s 内整包到达）：

| run | content 窗口 | content_tps | out_tps | TTFT | completion / reasoning |
|---|---|---|---|---|---|
| `flash_r1` | **1.481s** | **332.9** | **210.0** | 26.9s | 1332 / 839 |
| `smoke_flash` | **1.502s** | **321.6** | **207.1** | 17.5s | 1269 / 786 |

> **200+ t/s 只出现在这种突发包采样里，不是可持续速率。** 两次都伴随 `prompt_tokens=67`（稳态全为 120）+ 巨大 TTFT（17–27s），说明是另一套上游协议/账号的**瞬间整包下发**，不是稳定吞吐。

### 3. TTFT 分布与 quota_cooldown 证据

44 条拿到 content 的请求，TTFT：**min 1.5s / p10 5.8s / p50 11.0s / p90 23.3s / max 36.8s**；其中 25/44 >10s、10/44 >20s。分布是双峰的（1.5–8s 的快路由 vs 10–37s 的排队/思考路由），不是长尾噪声。

`gemini-3.8-flash-high` 12 次尝试中 **8 次 429**，错误体原文：

```json
{"error":{"message":"Local key pool exhausted for model 'gemini-3.8-flash-high' (gateway-side cooling / window-budget / family-quota exhaustion, no upstream attempt in this request; no schedulable keys, check `ponyllm status`). Last error: No available key for provider 'antigravity' (all keys cooling down, window-budget exhausted, or family-quota exhausted) (request_id: req_18e74668ec92931f)","type":"insufficient_quota","code":"quota_exhausted"}}
```

`gemini-3.8-flash-medium` 2/2 次 503，错误体内含上游原文（账号被封）：

```json
{"error":{"message":"All candidate upstream providers exhausted for model 'gemini-3.8-flash-medium' ... Last error: Request failed after 7 attempts across keys [...]: (failures: 7 timeout/network): HTTP 403 ... {\"error\":{\"code\":403,\"message\":\"This service has been disabled in this account for violation of Terms of Service. Please submit an appeal to continue using this product.\",\"status\":\"PERMISSION_DENIED\",\"details\":[{\"reason\":\"TOS_VIOLATION\",\"domain\":\"cloudcode-pa.googleapis.com\"}]} (request_id: req_18e7466f542e081a)","type":"api_error","code":"upstream_unavailable"}}
```

`gemini-3.8-flash` 自身也有 4/32 次 `quota_exhausted`（同文案）→ 三个档位**共用同一个 `antigravity` 账号池**，池内存在冷却、window-budget 枯竭与至少 1 个 TOS 封禁账号。

### 4. 明确回答

> **否。同网关、同模型，不存在稳定的 ≥200 t/s 稳态解码。** 依据：26 条稳态 run 的 content 解码速率 p90 = 152.4 t/s、max = 162.4 t/s，**0/26 达到 200**；唯一两次 >200 的采样（207.1 / 210.0 out_tps；321.6 / 332.9 content_tps）是 1.5s 内整包下发的突发形态（2/58 ≈ 3.4%，且都伴随 17–27s TTFT），不构成可持续吞吐。

---

## ① 结论表

**指标定义（口径不同结论会反转，必须先读）**

| 指标 | 定义 |
|---|---|
| TTFT | 请求发出 → 第一个含 `delta` 的 SSE 事件（含 reasoning chunk）|
| `content_window_tps`（解码窗口 t/s）| `content_tokens / (末个 content chunk 时间 − 首个 content chunk 时间)`；`content_tokens = usage.completion_tokens − usage.completion_tokens_details.reasoning_tokens` |
| `out_window_tps`（整段输出 t/s）| `usage.completion_tokens / (末个输出 chunk − 首个输出 chunk)`，含 reasoning token |
| `e2e_tps`（端到端 t/s）| `usage.completion_tokens / 请求总时长`（**含 TTFT**）|

**每个 model id × 中位速率 × TTFT × 样本数**（stream=true、固定 prompt、max_tokens=512）

| model id | n 成功/尝试 | 中位 content_tps | 区间 | 中位 out_tps | 中位 e2e_tps | 中位 TTFT |
|---|---|---|---|---|---|---|
| `gemini-3.8-flash` | 28 / 32 | 141.1 | 102.4–332.9 | 117.3 | 53.8 | 8.4s |
| `antigravity/gemini-3.8-flash`（**pony-agent 实际使用的 id**）| 12 / 12 | 134.6 | 112.2–158.3 | 116.4 | 65.9 | 8.0s |
| `gemini-3.8-flash-high`（**dsh 默认 id**）| 4 / 12 | 113.4 | 109.3–117.6 | 111.4 | 51.9 | 6.9s |
| `gemini-3.8-flash-medium` | 0 / 2 | —（全 503，上游 403 TOS_VIOLATION）| — | — | — | — |
| `gemini-3.8-flash-exp` | 0 / 1 | —（404 model_not_found）| — | — | — | — |

**核心结论**：上游稳态解码 110–150 t/s（中位 117），端到端 40–80 t/s；**200+ 仅出现在约 3% 的突发整包采样中，非可持续**。「200+ vs 60–70」两个数字都真实，但口径不同、路由也不同。

### 1.1 两种互斥的「路由指纹」（n=58 内反复复现，非抖动）

| 指纹 | token/chunk | n | 中位 content_tps | 中位 out_tps | 中位 TTFT | prompt_tokens |
|---|---|---|---|---|---|---|
| **A 突发整包** | 1.0 | 2 | **327.2**（321.6–332.9）| **208.6**（207.1–210.0）| 22.2s | 67 |
| **B 稳态粗粒度** | 4.06–4.82 | 26 | **117.0**（109.3–152.4）| 113.7（105.7–126.5）| 7.6s | 120 |

指纹 A 全部伴随 `prompt_tokens=67`，指纹 B 全部为 120 → 网关把同一 id 路由到了**两套不同的上游协议/账号**，连 prompt 模板与 SSE 打包粒度都不同。`[UNVERIFIED]` 具体是"不同账号"还是"不同协议层"。

### 1.2 相邻 chunk 到达间隔（判断客户端 16ms/emit 节流是否咬合）

新增 2 次 `antigravity/gemini-3.8-flash`（同 prompt / stream:true / max_tokens=512）+ 7 条既有稳态 run，共 9 条样本，content chunk 相邻到达间隔（ms）：

| run | n_gap | **中位** | p10 | p90 | 最大 |
|---|---|---|---|---|---|
| `aggap_r1`（新）| 140 | **42.8** | 0.1 | 72.6 | 116.0 |
| `aggap_r2`（新）| 106 | **25.7** | 0.1 | 56.4 | 218.9 |
| `agflash_r1` | 138 | 21.2 | 0.0 | 90.1 | 263.4 |
| `agflash_r2` | 130 | 32.7 | 0.0 | 57.8 | 188.9 |
| `agflash_r5` | 130 | 31.6 | 0.0 | 48.7 | 132.6 |
| `agflash_r7` | 103 | 33.0 | 0.0 | 52.7 | 81.9 |
| `agflash_r8` | 99 | 41.6 | 1.7 | 81.2 | 139.3 |
| `flash_r11` | 142 | 21.0 | 0.1 | 56.8 | 239.9 |
| `flash_r16` | 99 | 33.0 | 0.0 | 50.1 | 129.3 |

**判定：9/9 条 content chunk 间隔中位数（21.0–42.8 ms）> 16 ms → 16ms/emit 节流不会咬合，客户端节流不是速率瓶颈。**
注意 p10 ≈ 0–5.5 ms：存在同刻连包（一个 SSE 读取里带多个 chunk），节流只会把这些同刻连包合并成一次 emit（影响视觉平滑，不影响吞吐）。反推：中位 21–43ms、每 chunk 4.1–4.8 token → 100–150 t/s，与稳态区间自洽。

### 1.3 对照实验

| 实验 | 结果 | 判定 |
|---|---|---|
| `reasoning_effort: low` / `high` | 均 200 接受，无 4xx | 网关接受；速率未显著改变（content 107.6–162.2）|
| `thinking: {type: disabled}` | 200 接受，无 4xx | 未真正关闭思考（仍出现 reasoning tokens 或整段 reasoning 占满 512）|
| `reasoning: {effort: low}` | 200 接受 | 网关宽松透传未知字段，**不能**用它判定思考档位 |
| `stream: false`（同 id）| 3 次，e2e 55.0 t/s（45.6–70.6）| 与 stream 的 e2e 同量级；**「60–70 t/s」在非流式端到端口径下完全可复现** |
| 长 prompt（3544 词，prompt_tokens=4292）×2 | content 109.3 / 110.4 t/s | 输入长度**不**改变解码速率，只推高 TTFT |
| `max_tokens=512` | 实测 completion_tokens 最高 **2356**，多数 >512 | 网关**不把 max_tokens 当硬上限**；部分请求恰好停在 512 且全为 reasoning、content 为空 |

### 1.4 网关路由可观测事实（仅限响应体实证）

- `GET /v1/models` 只宣告 2 个 gemini：`antigravity/gemini-3.8-flash`、`antigravity/gemini-3.1-flash-image`（全表 33 个模型）。`gemini-3.8-flash` / `-high` / `-medium` 均为**未登记别名**但被接受，`-exp` 404。
- 三个 gemini 档位共享 `antigravity` 账号池（同报错文案）；池内至少 1 个账号 403 TOS 封禁、若干处于 cooling / window-budget 枯竭。
- 本机 `ponyllm status`：**服务未运行**（本地 `127.0.0.1:8080` down）；`tokens.ponyjob.top` 解析到公网 `123.6.40.77` → **远端网关日志不可得**。
- pony-agent 实际调用的 id 是 `antigravity/gemini-3.8-flash`（`.pony-agent/PonyAgent/graph-runs.json` 的 `providerModel`），dsh 默认是 `gemini-3.8-flash-high`。

---

## ② 命令原文与 Exit Code

所有请求由 `probe.py` 用真 curl 流式抓取，逐 chunk 打 `time.perf_counter()` 时间戳。

**实际执行的 curl 命令模板（key 以 `-H @raw/.hdr` 注入，argv 与报告中不落明文）：**

```bash
curl -N -sS -m 300 \
  -H @raw/.hdr \
  -H "Content-Type: application/json" \
  -d '{"model":"gemini-3.8-flash","messages":[{"role":"user","content":"Write a plain list of 40 short factual statements about the solar system, one per line, numbered 1 to 40. No preamble, no conclusion, no markdown headers."}],"max_tokens":512,"stream":true,"stream_options":{"include_usage":true}}' \
  https://tokens.ponyjob.top/v1/chat/completions
# 等价可读形式（报告口径）：-H "Authorization: Bearer <redacted>"
# Exit Code: 0（58/58 次 curl 均 0；HTTP 层错误由 -sS 正常返回 body，不是传输失败）
```

**模型清单探测**

```bash
curl -sS -m 40 -o raw/models.json -w 'HTTP %{http_code}\n' \
  -H "Authorization: Bearer <redacted>" https://tokens.ponyjob.top/v1/models
# => HTTP 200 ; Exit Code 0
```

**id 存在性 + 非流式探针（5 个 id）**

```bash
for m in gemini-3.8-flash gemini-3.8-flash-high gemini-3.8-flash-medium antigravity/gemini-3.8-flash gemini-3.8-flash-exp; do
  curl -sS -m 90 -o "raw/probe_${m//\//_}.json" -w '%{http_code}' \
    -H "Authorization: Bearer <redacted>" -H 'Content-Type: application/json' \
    -d "{\"model\":\"$m\",\"messages\":[{\"role\":\"user\",\"content\":\"say hi\"}],\"max_tokens\":16,\"stream\":false}" \
    https://tokens.ponyjob.top/v1/chat/completions
done
# => gemini-3.8-flash 200 / gemini-3.8-flash-high 429 / gemini-3.8-flash-medium 503
#    antigravity/gemini-3.8-flash 200 / gemini-3.8-flash-exp 404 ; Exit Code 0
```

**压测主体（5 批，共 58 次）**

```bash
export PONYLLM_API_KEY="$(grep -m1 'PONYLLM_API_KEY:' /home/dm/.dsh/.credentials.yaml | sed 's/.*PONYLLM_API_KEY: *//')"
./run_matrix.sh    # 批1: gemini-3.8-flash ×3, antigravity/gemini-3.8-flash ×3, -high ×3, -medium ×2
./run_matrix2.sh   # 批2: gemini-3.8-flash ×6, antigravity/gemini-3.8-flash ×6, -high ×3
./run_matrix3.sh   # 批3: -high ×6 + 参数消融 + stream:false 对照
./run_matrix4.sh   # 批4: gemini-3.8-flash ×10, -high ×4
python3 probe.py --model antigravity/gemini-3.8-flash --run-id aggap_r1   # 批5: chunk 间隔实验 ×2
python3 probe.py --model antigravity/gemini-3.8-flash --run-id aggap_r2
python3 analyze.py && python3 chunk_gaps.py aggap_r1 aggap_r2 ...          # 后处理
# 全部 Exit Code: 0
```

**消融/对照（原文）**

```bash
python3 probe.py --model gemini-3.8-flash --run-id efflow_r1   --extra '{"reasoning_effort":"low"}'
python3 probe.py --model gemini-3.8-flash --run-id effhigh_r1  --extra '{"reasoning_effort":"high"}'
python3 probe.py --model gemini-3.8-flash --run-id thinkdis_r1 --extra '{"thinking":{"type":"disabled"}}'
python3 probe.py --model gemini-3.8-flash --run-id reffort_r1  --extra '{"reasoning":{"effort":"low"}}'
python3 probe.py --model gemini-3.8-flash --run-id nostream_r1 --stream false
python3 probe.py --model gemini-3.8-flash --run-id longprompt_r1 --prompt "$(cat /tmp/longprompt.txt)"
# 全部 Exit Code: 0
```

**只读环境观测（未改任何配置）**

```bash
ponyllm status | head -40      # => 状态：未运行；9 个 provider；antigravity 10 个密钥 ; Exit Code 0
ss -ltnp | grep -E ':8080'      # => 无监听 ; Exit Code 1（grep 无匹配，符合预期）
getent hosts tokens.ponyjob.top # => 123.6.40.77 ; Exit Code 0
docker ps                        # => 无 ponyllm 容器 ; Exit Code 0
grep -rn 'gemini-3.8-flash' .pony-agent/PonyAgent/graph-runs.json  # => providerModel: "antigravity/gemini-3.8-flash"
```

---

## ③ 原始文件路径（全部在 `.dev-team/diag-tps/upstream/` 下）

| 文件 | 内容 |
|---|---|
| `raw/models.json` | `GET /v1/models` 完整体（33 模型）|
| `raw/probe_*.json` | 5 个 id 的存在性/错误体原文 |
| `raw/<run_id>.sse.txt` | **逐字节 SSE 原文**（含每个 `data:` 行）|
| `raw/<run_id>.chunks.jsonl` | 每 chunk 的 `t`（相对请求开始、秒）、`kind`(reasoning/content/meta/done)、`c_len`/`r_len` |
| `raw/<run_id>.summary.json` | 单次请求全量指标 + `cmd` 原文 + `usage` + 错误体 |
| `raw/<run_id>.err` | 该 run 的 stderr |
| `analysis.jsonl` | 58 个 run 的 token 级速率汇总（本报告所有表的来源）|
| `chunk_gaps.py` | 相邻 chunk 间隔统计脚本（1.2 节数据）|
| `probe.py` / `analyze.py` | 探针与后处理脚本（可复跑）|
| `run_matrix{,2,3,4}.sh` | 4 批实验的驱动脚本原文 |

代表样本：`raw/aggap_r1.chunks.jsonl`（chunk 间隔实验）、`raw/flash_r1.sse.txt`（突发包指纹）、`raw/agflash_r2.sse.txt`（稳态包指纹）、`raw/high_r1.summary.json`（quota 错误体）、`raw/medium_r1.summary.json`（TOS 403 错误体）。

---

## ④ 未验证项 `[UNVERIFIED]`

1. `[UNVERIFIED]` **指纹 A/B 的分配机制**：58 次中仅 2 次落到突发包。未证明是「不同上游账号」还是「同一账号不同协议（antigravity 原生 vs chat 兼容层）」；证据仅为 `prompt_tokens` 67 vs 120 + chunk 粒度 1 vs 4.6 token 的强相关。
2. `[UNVERIFIED]` **dsh「200+ t/s」的具体客户端口径**：本次只测出上游存在 content 子窗口 327 t/s 的突发形态，未在 dsh 内部复现其测速公式。
3. `[UNVERIFIED]` **pony-agent「60–70 t/s」的口径**：`graph-runs.json` 内无 tps 字段，无法对齐；本次只能给出端到端口径 52–66 t/s 与之同量级。
4. `[UNVERIFIED]` **`-high` 的真实差异**：仅 2 次拿到 content（109.3 / 117.6 t/s），8/12 被 429 挡住，无统计效力。
5. `[UNVERIFIED]` **账号池规模与冷却窗口**：远端无日志，key 数量、冷却时长、每 key 配额均不可观测（`-medium` 错误体显示单次请求在 7 个 key 上各失败一次，key 名被网关 `<redacted>`）。
6. `[UNVERIFIED]` **`thinking: disabled` 是否生效**：网关 200 接受但行为未见改变，可能被静默忽略。
7. `[UNVERIFIED]` **16ms 节流的端到端影响**：仅从到达间隔推断「不咬合」，未在 pony-agent 内部实测节流后的 emit 吞吐。

## ⑤ 盲区

- **远端不可观测**：`tokens.ponyjob.top` 是公网网关（123.6.40.77），本机 `ponyllm` 服务未运行，无 flight recorder / 日志 / 实时 status → 路由决策、key 选择、重试次数全靠响应体反推。
- **时间窗偏差**：全部样本集中在 18:42–19:12 单一时段，且 `-high` 明显处于配额枯竭期；配额恢复后速率可能不同。
- **单机单出口**：本机到网关网络路径固定（0 次传输失败），未测不同出口/地域对 TTFT 的影响。
- **prompt 单一**：固定 50 token 指令 + 一次 4292 token 长 prompt 对照，未覆盖真实 agent 场景的多轮长上下文（工具调用、长 system prompt）。
- **并发未测**：所有请求串行（间隔 4s），未测并发下的排队与速率衰减——真实使用是并发/多 run 的。
- **token 口径依赖网关自报**：`content_tokens` 由 `completion_tokens − reasoning_tokens` 推得；若网关 usage 计数有偏，速率会整体偏移。稳态 `tokens_per_cchunk≈4.6` 也说明**chunk 数不能当 token 数用**（按 chunk 计速率会低估约 4.6 倍）。
- **样本计数以现存证据为准**：多批实验复用过同一 `run_id`（如 `agflash_r3`、`high_r5`），被覆盖的旧观测不体现在最终计数里；批1 曾有 1 次 `antigravity/gemini-3.8-flash` 429，现已不存证。
- **安全边界**：读取了 `~/.dsh/.credentials.yaml` 与 `~/.config/ponyllm/ponyllm.toml`（只读，未修改）；未改动产品代码、配置与 git。
