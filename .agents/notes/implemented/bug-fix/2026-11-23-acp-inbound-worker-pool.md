# Agent Note: 入站请求分发改为有界 worker 池，并让「不饿死」第一次真正可观测（B177 ③④）

Status: implemented

## Problem

两件事是一个硬币的两面：

1. **③ 分发串行**：`transport.rs` 只起**一个**名为 `acp-inbound` 的线程，循环体是
   `for request in request_receiver { respond_to_inbound_request(...) }`。一个
   `terminal/wait_for_exit`（整轮预算最长 120s）会把后续所有入站请求压在队列里——
   复审实测 `terminal/output` 被压后 **5.008s**，对照组 **0.002s**。
2. **④ 断言不可观测**：既有夹具 `probe_terminal_long` 是**严格同步**的——9403/9404 只在读到
   9402 的应答之后才发出。于是 h4 那条「不得饿死其它入站应答」的断言，实际只证明了
   「wait 结束后分发线程还活着」，**结构上不可能观测到并发**。

## Decision

### ③ 有界 worker 池

`ACP_INBOUND_WORKERS = 4` 个 worker 共享同一个 `Arc<Mutex<Receiver>>`：

- **取任务时短暂持锁**（`recv()` 立即返回），**处理在锁外** ⇒ 一个慢 handler 不再挡住其它 worker 取活；
- 安全性依据：`AcpInboundRequestHandler: Send + Sync`，且应答写 stdin 走
  `Arc<Mutex<Option<ChildStdin>>>` 串行化 ⇒ 帧不会交错，应答按 JSON-RPC `id` 匹配；
- worker 数**有界**，不是「一请求一线程」——避免恶意/异常 agent 用大量入站请求把线程数打爆。

### ④ 让断言真的能失败

新增夹具模式 `probe_terminal_interleave`：create(9401) → 发 **慢**请求 wait_for_exit(9402)
→ **立即**发 **快**请求 output(9404)、**不等** 9402 的应答 → 再读两条应答。

新增用例 `g10_slow_inbound_request_does_not_starve_others` 断言 **9404 的应答必须排在 9402 之前**。

## Verification（红绿对比，这是关键证据）

| 配置 | g10 结果 |
|---|---|
| `ACP_INBOUND_WORKERS = 4`（本 ADR） | **PASS**（`1 passed`，3.13s） |
| `ACP_INBOUND_WORKERS = 1`（等价旧单线程语义） | **FAILED**：`G10 违约：…9404@2 vs 9402@1，全序列 [9401, 9402, 9404, 9405]` |

即：这条用例的绿/红确实由「分发是否并发」决定，不是摆设。

其余证据：
- `--test acp_config_regressions` → **8 passed**（含 h4/h1/h5/i1 不被破坏）
- `--test acp_capabilities_contract` → **12 passed**（含 g7 terminal 全链 + 新增 g10）
- `npm run rust:lint` → **PASS**（新增/恶化 0，EXIT=0）

## 残留（诚实标注）

复审 ②「两个 JoinHandle 都 detach、close 后线程活到 handler 返回」**未在本次收口**：worker 池
让「一个慢 handler 卡住所有请求」消失，但 close 语义仍不等线程退出（要修需引入停止标志 +
有界 join，且会改变 close 的耗时语义，属独立决策）。B177 因此只剩 ②。

## Alternatives considered

- **每请求 spawn 一个线程**：最简单，但入站请求量由对端 agent 控制 ⇒ 线程数无界，等于把
  DoS 面交给外部进程。
- **改用 tokio 异步 runtime**：能优雅地做到并发上限，但整个 transport 是同步阻塞模型
  （`ChildStdin` + `Mutex`），引入 async 是一次跨模块重写，与这条 ADVISORY 的量级不匹配。
- **保持单线程、只调大队列**：治不了"一个 120s 的 wait 压住后面所有请求"这个根因。
