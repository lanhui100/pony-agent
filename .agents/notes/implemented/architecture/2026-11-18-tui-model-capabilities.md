# Agent Note: TUI 模型能力编辑（上下文规格快选 + 输入模态）与默认窗口十进制 256K

Status: implemented

## Problem

`/models` 面板的 `w` 编辑器（ADR 2026-11-14-model-declared-window-tui-entry）只覆盖上下文窗口，留下三个可见缺口：

1. **默认值显示不自洽**：`DEFAULT_CONTEXT_WINDOW_TOKENS = 262_144`（2^18），而面板按桌面端
   `formatTokenUnit` 的十进制口径渲染，于是「默认 256K」在界面上显示为 `262.1K·预设`；
   用户看到的第一眼是「这不是 256K」。
2. **窗口只能手输**：桌面端有 `CONTEXT_TOKEN_PRESETS`（32K/64K/128K/256K/512K/1M）一键填入，
   TUI 只能敲 `200000` / `200K` / `1M`，常用规格不可选。
3. **输入模态无入口**：`providers[].models[].capabilities.supportsImage/Video/AudioInput`
   是逐模型事实，桌面端可勾选，TUI 完全没有入口；用户只能手改 `providers.json`。

## Decision

1. **默认窗口对齐十进制 256K**：`config::DEFAULT_CONTEXT_WINDOW_TOKENS = 256_000`
   （原 `262_144`）。压缩水位仍由窗口推导（0.8×窗口 ≈ 205K），行为差异可忽略；
   面板从此显示 `256K`，与桌面端 `CONTEXT_TOKEN_PRESETS` 的 `256K = 256000` 同口径。
2. **常用规格成为共享常量**：`agent::model_window::CONTEXT_WINDOW_PRESETS`
   = `[32K, 64K, 128K, 256K, 512K, 1M]`，TUI 快选只引用它，不另写一张表。
   **2026-11-20 后续**：TUI 快选档位收敛为 `[256K, 512K, 1M]`（下限 256K），默认窗口回到 1M，
   见 [2026-11-20-model-discovery-preserves-per-model-window](../bug-fix/2026-11-20-model-discovery-preserves-per-model-window.md)。
3. **写入面扩展为 `set_model_capabilities`**：一次定点写入上下文窗口 + 输入模态
   （图片/视频/音频），仍只改目标模型的键、仍用临时文件 + rename。
   - 窗口语义不变：`Some(Some(tokens))` 声明；`Some(None)` 删键（跟随预设）。
   - 模态语义：目标值与「当前生效值」相同 ⇒ **一个键都不写**（避免打开即保存把预设固化成声明）。
   - 目标值不同 ⇒ 切 `capabilityPreset = custom` 并显式落**完整能力族**。原因：
     `normalize_model_capabilities` 在非 custom 预设下忽略声明布尔值，只认预设；
     若只插入 `supportsVideoInput` 而不切 custom，勾选是死写入。完整能力族同时保证
     工具/流式/推理/输出模态沿用当前生效值，不会因切 custom 被 serde 默认值静默关掉。
4. **TUI `w` 编辑器扩为「模型能力」**：
   - `tab` 在「上下文窗口 / 多模态输入」间切字段；
   - 窗口字段：`←/→`（或 `↑/↓`）在常用规格间快选并填入缓冲，仍接受 `200000` / `200K` / `1M` /
     `1.5M` 手输；留空 / `-` = 跟随预设；
   - 模态字段：`←/→` 移项、`空格` 开关；只暴露图片/视频/音频三项输入能力；
   - 不暴露工具 / 流式 / 推理 / 输出模态等高级参数（保持「常用能力可改、高级面留桌面端」）。
5. **模型面板徽标去掉 `ctx` 前缀**：`200K`（声明）/ `256K·预设`（补缺）。
   列表中只有窗口一个数值型徽标，前缀纯占宽。
6. **失败语义不变**：解析错误 / 未命中 / 落盘失败留在面板显示，零写入；写入后读不回即报错。
7. **不新增斜杠命令**：入口仍是 `/provider → 选提供商 → 模型面板 → w`。

## Alternatives considered

- **A. 保留 262_144，只把显示改成 1024 进制**：`262144 → 256K` 看似整齐，但解析口径是十进制
  `256K = 256000`，同一串 `256K` 读回会变成另一个数，显示/解析往返不自洽；且与桌面端
  `formatTokenUnit` 冲突。
- **B. 保留 262_144 并显式接受 `262.1K` 文案**：用户预期「默认 256K」，首屏却给 `262.1K`，
  正是本卡要消除的认知成本；窗口值本身也只差 0.8%，没有守住旧数字的必要。
- **C. 在 `/provider` 新增第 4 步配置窗口/模态**：`/provider` 是新增 provider 的向导，
  新 provider 尚无 models；窗口与模态是既有模型的属性，挂错表单（同 ADR 2026-11-14 备选 C）。
- **D. 只写模态键、不切 `capabilityPreset=custom`**：非 custom 预设下声明布尔值被
  `normalize_model_capabilities` 整体忽略，勾选保存后不生效——比没有入口更坏（谎报成功）。
- **E. 切 custom 时只改模态三个键**：serde 对缺失布尔取默认（`supports_reasoning=false`、
  输出全 false），会把预设模型的能力静默关掉；必须显式落完整能力族。
- **F. 直接暴露全部 `ProviderModelCapabilities`**：工具/流式/输出模态/推理属于高级参数，
  TUI 面板空间有限且误触代价高；本卡按用户要求只做输入模态。
- **G. 用 `load_view → save_view` 整份重写**：会把所有未声明窗口固化成默认值，并重写
  `maxOutputTokens` 等归一化字段（同 ADR 2026-11-14 备选 A）。

## Consequences

- 已有「未声明窗口」的模型默认分母从 262144 改为 256000（显示 `256K`）；显式声明过
  `262144` 的模型不受影响（声明优先）。
- 首次在 TUI 修改模态会把该模型的 `capabilityPreset` 固化为 `custom`（这是让布尔能力
  真正生效的必要代价）；此后该模型的工具/推理等能力不再跟随预设升级，面板会按当前值显示。
- 视频/音频输入目前只落配置声明；运行时只有 `supports_image_input` 有消费点
  （`runtime/turn_prep.rs` / `context.rs`），视频/音频的运行时投影仍是后续工作。
- 回归证据：
  - core `agent::model_window` 13 例（含 `changing_modalities_switches_to_custom_and_keeps_other_capabilities`、
    `unchanged_modalities_write_nothing`、`presets_are_decimal_and_parse_back_exactly`）；
  - TUI `model_window_editor` 15 例（含 `right_arrow_fills_common_preset_and_commit_writes_it`、
    `modality_toggles_switch_to_custom_and_persist`、渲染断言 `256K·预设` / `多模态输入`）；
  - `agent::compression` 默认窗口断言改为 256000；
  - 全量 `cargo clippy -p pony-agent-core --all-targets` exit 0（新增代码无告警）；
  - `npm run rust:lint` **未通过**，原因是既有基线漂移、与本卡无关：
    `tui/text.rs::uninlined_format_args` live 8 / base 4，而 `text.rs` 本轮零改动
    （最近一次改动为 `99b3adce` 拆分提交）⇒ 已登记 **B252**。本卡不擅自重基线，
    也不顺手改无关文件。
- 相关旧记录：本卡**部分取代**
  [2026-11-14-model-declared-window-tui-entry](./2026-11-14-model-declared-window-tui-entry.md)
  的徽标文案与编辑器范围，并调整
  [2026-11-10-history-fold-replay-and-default-256k-window](./2026-11-10-history-fold-replay-and-default-256k-window.md)
  决策 6 的数值口径；两条旧记录均已链回本卡。
