# Agent Note: 模型声明窗口优先于能力预设（`capabilities.contextWindowTokens` 不再被静默覆盖）

Status: implemented

## Problem

用户唯一能改的「上下文窗口」字段 `providers[].models[].capabilities.contextWindowTokens`
在**非 `custom` 预设**下是只读的：手填 200000 / 1000000，加载后一律变成 `262144`，连保存
回写也写成 `262144`。现场是 TUI 右上角的上下文百分比：200K 模型显示 62%（真值 81%），
1M 模型显示 62%（真值 16%）——分母被钉死，读数因而偏大或偏小。

根因在 `crates/pony-agent-core/src/agent/config.rs::normalize_model_capabilities`：

```rust
let defaults = preset_model_capabilities(...);
// looks_like_legacy_empty / looks_like_implicit_defaults 两段判断……
if looks_like_legacy_empty || looks_like_implicit_defaults { return defaults; }
defaults   // ← 两支同返，声明值在此被丢弃
```

两段 `looks_like_*` 是死代码（分支结果相同），真正的语义是「预设永远赢」。这带来三重后果：

1. **字段形同虚设**：用户在 `providers.json` 里填什么都不会生效，也没有任何提示；
2. **`DEFAULT_CONTEXT_WINDOW_TOKENS` 被迫兼职**：它本是压缩早于溢出的**保守水位**
   （[`2026-11-10-history-fold-replay-and-default-256k-window`](../architecture/2026-11-10-history-fold-replay-and-default-256k-window.md) 第 6 条），
   却同时被当成显示分母与所有模型的「真实容量」；
3. **前端镜像同一规则**：`src/stores/providers.ts::resolveCapabilityDeclaration` 在非 `custom`
   预设下同样丢弃声明值，桌面端一加载就把手填窗口改回预设默认（1M），保存即落盘。

## Decision

1. **声明优先，预设只补缺**：`normalize_model_capabilities` 在非 `custom` 预设下也保留
   `Some(>0)` 的窗口，仅在 `None`（未声明）或 `0`（非法）时取预设默认值。
2. **布尔能力仍由预设决定**：`capabilityPreset` 是能力族的真值（tools / streaming / 推理 /
   图片…），只对「窗口」这一项开例外——预设猜不准某次部署真实允许的上下文长度，那是个体事实。
3. **「未声明」必须可与「声明了 262144」区分**：`ProviderModelConfig.capabilities` 整段缺席时
   改用哨兵 `undeclared_model_capabilities()`（窗口 `None`）。否则 serde 的 `Default` 会注入
   `Some(262144)`，预设永远没有补缺的机会（未来按模型给真实窗口时立刻失真）。
4. **`0` 一律按未声明收口**（含 `custom` 分支）：`Some(0)` 会让上下文比例与压缩水位按 0 窗口
   计算，是比缺失更坏的输入。
5. **前端同口径镜像**：新增 `withDeclaredContextWindow`，非 `custom` 预设保留已声明窗口；
   否则后端修复会被桌面端加载/保存抵消，甚至把 262144 放大成前端默认的 1M。
6. **删除 `looks_like_legacy_empty` / `looks_like_implicit_defaults`**：两支同返，属死代码。

## Alternatives considered

- **A. 让用户改用 `custom` 预设**（当下可用的绕法）：`custom` 会同时关掉预设推理出的布尔能力
  （推理 / 图片输入），语义损失与「只想改窗口」不匹配；且桌面端每次保存本就写 `custom`，
  等于把「能力族」信息整体丢掉。
- **B. 新增模型级顶层字段（如 `model.contextWindowTokens`）**：制造第二个真源，要迁移、要双写，
  而现有字段语义本来就是「逐模型声明」——问题不在字段位置，在归一化把声明丢了。
- **C. 保持预设权威，改为「把预设默认猜准」**（按模型名查 128K/200K/1M）：`CAPABILITY_CATALOG`
  只覆盖已知型号，中转/自建/新模型必然猜错；窗口是部署事实，查表永远滞后。
- **D. 在归一化里把 `1000000` 当「旧前端幻觉值」清掉**：无法与真实 1M 声明（gemini / gpt-4.1）
  区分，且属于静默忽略用户可见字段——比缺陷本身更难排查。
- **E. 只改 Rust、不动前端**：桌面端 `normalizeModel` 每轮都会把声明值重写为预设默认（1M）并
  落盘；后端改成声明优先后，这些 1M 会被当真，窗口反而放大 4 倍。前后端必须同提交改。

## Consequences

- **正向**：改 `providers.json` 的窗口立即生效（TUI 每轮重读注册表，无需重启）；上下文百分比、
  压缩水位、首 token 展示同源同口径。
- **代价（显式承担）**：声明优先意味着「填错就按错的走」——给 128K 模型填 1M，压缩会晚于上游
  溢出。由用户可见字段承担，比静默覆盖更可诊断；`custom` 预设仍是「连布尔能力一起自定」的出口。
- **未收口（如实登记）**：TUI 仍无窗口输入入口（`/provider` 只有名称/URL/Key，`/settings` 只有
  三项），当前只能改注册表或用桌面端；补入口是独立变更。
- **回归证据**：
  - `config_normalize_tests` 5 条（声明保留 / 预设仍管布尔 / 补缺 / 0 视为未声明 / 往返）；
  - 集成 `tests/config_context_window_precedence.rs`：真实 `ProviderRegistryStore` 加载 + 保存 +
    落盘内容 + 重载四段断言；
  - 前端 `tests/providers.store.spec.ts`：声明优先 + 布尔仍由预设决定 + `null`/`0` 走补缺；
  - 反向证据：暂时回滚 `config.rs` 后上述 5 条 Rust 用例全部变红（`left: Some(262144)`）。
