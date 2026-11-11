# 0039 wave-37 工具面扩展、write_file 读前写观察门与子代理真实执行引擎

Status: implemented

## 背景

三方对比审计（claude-code 2.1.88 restored-src / deepseek-harness / pony-agent-core）发现三类 P0 缺陷：

1. **模型可见工具面 21 项**，且三处基准断言（tools.rs 契约测试、provider/mod.rs 两处 28 名基准）
   与注册表漂移，门禁为红（机器证据：`builtin_tool_contract_views_deduplicate_to_model_surface`
   FAILED，left=21 项/goal 三件，right=28 项/subagent 团队九件）。
2. **write_file 缺读前写观察门**：`edit_file` 自 wave-22 起强制 `fs_not_observed` fail-closed，
   但 `write_file` 对已存在文件且未传 `expectedHash` 时静默覆盖。
3. **子代理委派为伪实现**：`orchestration::subagent()` 返回合成字符串
   `"Subagent completed execution for prompt: ..."`，对模型谎报执行成功；
   `SUBAGENT_RUNTIME_WIRED=false` 使 10 件委派工具全部返回 `capability_unavailable`。

## 决策

### D1 工具面 21 → 28（C1）

把已实现但零工具入口的 lib 能力（present.rs / git_search.rs / jobs.rs）注册为模型可见工具：
`Present`/`Find`/`GitDiff`（Read/Search + workspace.read）与
`JobStart`/`JobOutput`/`JobKill`/`JobList`（Execute + workspace.execute），
`contract_priority=100`，全部走 governed dispatcher 派发臂；`job_start` 强制携带
会话 workspace root 传给 `job_start_with_workspace_root` 做 fail-closed cwd 校验。
子代理/团队 10 件工具保持摘除（workflow 真实现延后至 wave-38）。

### D2 write_file 观察门（C2）

对 `existed_before==true` 的覆盖写：未携带 `expectedHash` 且未设
`PONY_ALLOW_UNOBSERVED_EDIT` → 结构化错误 `fs_not_observed` + recoveryHint
（先 Read 取 contentHash 再写，或显式豁免）。新文件路径不设门。
Barrier 修补（Lead）：`overwrite=false` 显式拒绝覆盖语义**先于**观察门判定，
保证既有 `file_exists` 错误码契约可达（lib 回归
`write_file_respects_overwrite_false_for_existing_file` 暴露的顺序缺陷）。

### D3 子代理真实执行引擎（C3）

- `SubagentTurnEngine` trait + `SubagentRunOutcome{final_text, session_id}` + OnceLock 全局槽；
- `HostTurnEngine`：独立 `AgentRuntimeBuilder` 建栈（独立 SessionStore/ProviderRegistryStore，
  与宿主完全隔离），prompt 包为 `TurnInput` 运行单 turn，提取 final 文本与 session_id；
- 错误契约：`subagent_busy`（并发 >2，AtomicUsize CAS + Drop guard 防 panic 泄漏许可）、
  `prompt_too_large`（>200000 字节）、`subagent_timeout`（recv_timeout ≤1s 粒度，
  超时会话标注孤儿 session_id）、`subagent_engine_panicked`（catch_unwind）；
- `orchestration::subagent()` 槽空返回 `capability_unavailable`，槽在则真实执行；
  `run_in_background=true` spawn 线程执行、返回 running；
- `SUBAGENT_RUNTIME_WIRED` 常量置 true 供二进制兼容，运行时判定改为
  `subagent_runtime_wired()`（引擎槽非空），10 处判断点同步替换。

### D4 引擎未自动装配

`HostTurnEngine` 不在 DesktopRuntimePreset 中自动注册——宿主（src-tauri / CLI / TUI）
显式 `set_subagent_engine(Arc::new(HostTurnEngine))` 后委派能力才上线（保守接线，
避免桌面宿主未审计并发语义前静默开启）。此装配遗留为 PA-146 后续任务。

## 替代方案考量

- **workflow 真实现**：需 DAG 引擎与子 turn 编排，依赖 C3 落地观测，推迟 wave-38。
- **引擎改为共享宿主 runtime**：会话/ProviderStore 与宿主耦合，隔离故障域差；否决。
- **超时强杀子 turn**：runtime 无 stop API（wave-36 在飞），超时改孤儿标注 + 人工排查。

## 验证收据（机器原生）

- `cargo check -p pony-agent-core --lib` EXIT=0
- `cargo test -p pony-agent-core --test wave37_tool_surface`：9 passed / 0 failed
  （串行与 5 连跑并行均稳定；红→绿时间线存档 `.dev-team/wave37-red-receipts.md`）
- `cargo test -p pony-agent-core --test wave36_adversarial_classification`：8/8 无回归
- `cargo test -p pony-agent-core --lib`：1063 passed / 1 failed——唯一红
  `governed_executor_dispatches_plan_control_create_replace_merge_and_complete_step`
  经 stash 对照判定为 wave-36 PA-141 在飞红相用例，非本波因果（欠账登记 wave-36）。

## 交叉影响

- `.dev-team/contracts/wave-37-tool-surface-subagent.md`（契约矩阵唯一真源）
- PA-146 任务卡（引擎宿主装配 + 文档同步）
- ADR-0038（wave-35 读前写契约）沿用其 `fs_not_observed` 语义，本波扩展到 write_file。
