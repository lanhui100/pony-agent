# Agent Note: ACP 入站线程保持 detach —— B177 ② 的「评估后接受」结论

Status: implemented

## Problem（复审 r2 的 ADVISORY ②）

`transport.rs` 为入站请求起 `ACP_INBOUND_WORKERS` 个分发 worker、为 stdout 起一个读线程，
但这些 `JoinHandle` **只在 `spawn` 失败分支被使用**，成功后即被 drop（Rust 语义 = detach）。
运输层结构体不保存句柄，`close()` 也不 join。因此：

- `close()` 返回时，线程**可能仍在运行**；
- 若某个 worker 正在处理 `terminal/wait_for_exit`（整轮预算最长 120s），它会活到 handler 返回。

## Decision

**评估后接受，不改 `close()` 语义。**

三条理由：

1. **进程内无害**：线程只持有 `Arc` 共享状态（stdin 槽、handler 槽、fatal/stderr 缓冲），
   没有任何独占资源；进程退出时 OS 回收。detach 在这里不是内存泄漏路径。
2. **改它会引入更坏的代价**：`close()` 是调用方（会话 teardown、取消、切换 agent）的热路径。
   要真 join 就得等最长 120s；要"有界 join"就得引入停止标志 + 超时通道——
   即把**一次已经正确的 teardown** 换成一次"可能卡住或需要额外状态机"的 teardown。
3. **可达最坏情形的入口已被关掉**：#79/#80 的 worker 池让"一个慢 handler 挡住所有入站请求"
   消失；慢 handler 只影响它自己那一个 worker，且它最终仍会被预算（120s）或应答失败终止。

## 残余风险（诚实标注，不粉饰）

- `close()` 之后日志里可能仍出现来自入站 worker 的一两条记录（时序上滞后于 close 返回）；
- 若未来把 `close()` 用于"必须保证线程已停"的场景（例如热重载、进程内多 agent 复用同一 stderr 缓冲），
  这个假设会破——届时必须改成本 ADR 「Alternatives」里的 B 方案。

## Alternatives considered

- **B 有界确认（推荐备选）**：`close()` 后用一个 done-channel 等 1s，超时只记 warn + 度量，
  不改语义但可观测。**没做的理由**：当前没有消费者需要这个信号（没有任何代码在 close 后
  读那两个缓冲），加一条"没人看的告警"只是噪音；等真有消费者时再按 B 实施。
- **C 完全 join**：语义最干净，但 `close()` 最长卡 120s，直接影响退出体感，否决。
- **保存 JoinHandle 但永不 join**：只是把 detach 变得"看起来像管了"，属自欺，否决。

## 结论

B177 十条 ADVISORY 到此全部有明确归属：**①⑤⑥⑦⑧⑨⑩③④ 已改**（七个独立 PR + worker 池），
**② 评估后接受并留档**。B177 可以关。
