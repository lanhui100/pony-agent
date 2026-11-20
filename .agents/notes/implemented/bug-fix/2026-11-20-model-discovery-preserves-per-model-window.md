# Agent Note: 模型发现不得覆盖逐模型上下文窗口，provider 默认窗口回到 1M

Status: implemented

## Problem

两件事在同一入口（TUI `/models` 面板）暴露：

1. **声明的窗口被模型发现顶替**：面板 `w` 编辑器把窗口定点写进
   `providers[].models[].capabilities.contextWindowTokens`（ADR 2026-11-14-model-declared-window-tui-entry），
   但**打开面板就会触发一次远程模型发现**，`UiEvent::ModelsDiscovered` 事件臂按
   `load_view → apply_models_discovered → save_view` 落盘。旧 `apply_models_discovered` 用 provider 的
   **首个模型条目**当模板重建全部条目，于是刚声明的窗口被模板值顶替，逐模型温度 / 能力预设 /
   能力族同样被抹平。用户可见现象：「改了模型的上下文，退出面板再进又退回默认值」。
2. **默认值与档位两端不一致**：TUI 快选档位下限 32K，provider 默认窗口（模型未声明时按多少算）
   是 256K，桌面端自始是 1M —— 同一份 `providers.json`，TUI 与桌面算出两套占用率。

## Decision

1. **发现结果只决定「有哪些模型、按什么顺序」**：`apply_models_discovered` 对已登记条目
   （id 优先、线上模型名兜底匹配）**整条原样保留**；只有新出现的 id 才用模板克隆协议 / 能力族，
   且**清空模板的窗口声明**（新模型从未被编辑过，应跟随自己的能力预设，而不是被写成别人的数字）。
2. **provider 默认窗口 256K → 1M**：`config::DEFAULT_CONTEXT_WINDOW_TOKENS = 1_000_000`，
   与桌面端 `src/stores/providers.ts` 同口径。十进制口径（`1M = 1_000_000`）不变。
3. **TUI 快选档位下限 256K**：`agent::model_window::CONTEXT_WINDOW_PRESETS = [256K, 512K, 1M]`；
   桌面端 `CONTEXT_TOKEN_PRESETS` 不动（仍有更宽列表）。
4. **回归护栏**：`tests/tui_batch4_regression.rs` 的 N17d（内存映射保留逐模型声明 / 新模型不继承模板窗口）
   与 N17e（磁盘 `load_view → apply → save_view` 往返后声明值存活）；压缩水位测试改为按 1M 默认断言。

## Alternatives considered

- **A. 保留模板克隆，靠「保存后重读面板」掩盖**：数据已被写坏，重读只会如实显示坏值；
  一次编辑的声明在下一次发现时丢失，编辑本身失去意义。
- **B. 发现结果只改内存、不落盘**：新模型不持久，重开面板即消失；面板列表与磁盘永久分叉。
- **C. 只跳过模板的窗口键，其余字段照旧覆盖**：温度 / 预设 / 能力族仍被抹平，且新 id 仍会
  继承模板的窗口数字，问题形态只是变窄。
- **D. 保留 256K 默认，只把桌面端改成 256K**：与用户裁决相反，且会把桌面端已声明的 1M 语义
  全面改写。
- **E. 让压缩预算与默认窗口解耦（显示 1M、预算仍按 256K）**：同一会话两个窗口结论，
  正是 2026-11-10 已修过的分叉形态。
- **F. 档位表保留 32K / 64K / 128K**：快选等于把压缩水位压到几十 K（几乎每轮触发压缩），
  实操上只会误选；真要用小窗口手输即可。

## Consequences

- 逐模型声明在「打开面板即发现」的路径上不再丢失；未声明模型的窗口不再被模板数字污染。
- 默认 1M ⇒ 压缩水位约 800K（`min(0.8×窗口, 窗口−输出预留−headroom)`）：真实窗口 128K~256K 的
  模型必须在 `/models` 的 `w` 里声明真实窗口，否则压缩晚于上游溢出（硬失败）。这是本次裁决
  接受的代价；ADR 2026-11-10-history-fold-replay-and-default-256k-window 的「默认 256K」决策被本条取代。
- 模型发现仍经 `save_view` 把「预设补缺」固化成显式声明（既有语义，本条未改）：声明值一旦落盘
  不会随预设演化。
