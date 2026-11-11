# PA-166: 跨进程 SDK（可编程驱动形态）

## 状态
- **状态**: Backlog（已登记，待排期）
- **优先级**: P2（**强依赖 PA-164 完成后启动**）
- **Complexity**: C
- **创建日期**: 2026-11-02
- **宿主形态矩阵定位**: 四种交付形态中的第 3 种，当前**仅进程内具备**。

## 需求背景（物理事实）
- 现状唯一"SDK"是**同进程 Rust crate**（`pony-agent-core`，`AgentRuntimeBuilder` / `HostControlPlaneBuilder` 显式注入）。仓外调用方（Node/Python/Go 服务、其他 Agent、测试框架）**无法驱动**。
- 三宿主目前为**编译期直连 core**：`src-tauri/src/lib.rs`、`bin/tui.rs`、`bin/cli.rs`（见 `docs/architecture/system-design-usage-governance.md:59` 「三宿主直连 core」）。无跨进程控制面。
- 对比基线：DSH 已提供 JSON-RPC over stdio 的跨进程 SDK，外部进程可驱动 runtime，并**天然支撑 subagent 委派**（`docs/analysis/deepseek-harness-architecture-comparison-2026-08-14.md:54`）。
- PA-110（智能体派生与协作编排工具族）若要做**跨进程** subagent（而非同进程），其地基即本卡。

## 目标
提供稳定的**进程外编程接口**：外部程序以 JSON-RPC over stdio 驱动 Pony，获得会话创建、turn 流式订阅、工具审批回调、并发多会话与 subagent 委派能力；并提供薄客户端（Rust + TypeScript）。

## 交付契约
- **C1 传输与握手**：JSON-RPC 2.0 over stdio；`initialize` 协商 `sdkProtocolVersion`，**版本不匹配 fail-closed 拒绝**（不降级猜测）。
- **C2 会话与并发**：`session.create / session.list / session.resume`；单进程多会话并发，**会话隔离**（并发 turn 不得串数据）。
- **C3 流式订阅**：`turn.start` + 服务端 `turn.event` 通知（复用 PA-164 事件契约字段，不新造第二套事件名）。
- **C4 审批回调**：`permission.request` 通知 + `permission.reply` 请求，形成宿主-调用方闭环；**未收到回复即按拒绝收敛**（fail-closed，禁止默认放行）。
- **C5 崩溃与背压**：调用方断开 → 会话有界清理；事件队列**有界**（满则明确 `overflow` 错误而非静默丢弃）；子进程退出必须传播为客户端可区分的错误。
- **C6 薄客户端**：`crates/pony-agent-sdk`（Rust）+ `packages/pony-sdk`（TypeScript），仅传输与类型，**不含业务逻辑**；版本与 server 同源。

## 非目标
- 不做网络/远程多租户（无鉴权模型前不开放 TCP）。
- 不替代 ACP（PA-165 面向 IDE 客户端生态；本卡面向程序化集成）。
- 不做 UI。

## 验收标准（物理收据）
1. `tests/acceptance/l2at-sdk.sh`：真实子进程 + 两客户端并发会话，断言隔离与事件逐条映射，落盘原始帧与 Exit Code。
2. 客户端契约测试：TS 与 Rust 客户端对同一 server 的行为一致（同一 fixture 双跑）。
3. 失败路径：版本不匹配、事件溢出、审批超时、子进程被杀——四类各有真实断言（不得只测 happy path）。
4. `RUSTFLAGS="-D warnings" cargo check --workspace` EXIT 0。

## 当前进展
已登记，未动工。

## 下一步动作
1. **前置**：PA-164 冻结事件契约与退出码（避免双份事件定义）。
2. 先定协议版本号与错误码表（写入 ADR），再实现 server 骨架。
3. 客户端只做薄封装；先在 Rust 侧打通并发隔离，再产出 TS 包。

## 当前卡点
- **事件契约归属**：若 PA-164 未先冻结，本卡会催生第二套事件 schema，属架构债。因此**强制串行**。
- 客户端版本矩阵（Rust crate / npm 包 / core 版本）需要与既有「版本四处同步契约」对齐，否则发版门禁会红。

## 断点续跑提示
不要另造事件名——直接消费 PA-164 的 JSONL 契约；server 只是「把 `HostControlPlane` 包成 JSON-RPC 门面」，运行时不改造。

## 工作量画像
- C1+C6 骨架（server + Rust 客户端）：2 人日
- C2 并发会话隔离：2 人日
- C3 流式订阅：1 人日
- C4 审批回调闭环：1.5 人日
- C5 崩溃/背压加固 + 对抗审查：1.5 人日
- TS 客户端 + 双客户端一致性验收：1.5 人日
- **合计约 9~10 人日**

## OpenSpec Change
待建。

## Canonical Spec
待建：建议 `openspec/specs/cross-process-sdk/spec.md`。

## Spec 状态
未起草。
