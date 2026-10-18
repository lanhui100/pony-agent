# Wave-37 契约矩阵：工具面扩展 / write_file 观察门 / 子代理真实执行引擎

- 波次：wave-37
- Owner：w37-protocol（protocol agent）
- 状态：冻结（本文件为 T2 红相测试与 T3/T4 实现的唯一契约依据）
- 背景：三方审计发现 ① 模型可见工具面 21 项与基准断言/注册表漂移；② write_file 缺 edit_file 的 wave-22 read-before-write 观察门；③ `subagent()` 为伪实现（返回假字符串）。

---

## C1 工具面扩展：模型可见面 21 → 28 项

### C1.1 新增 7 项工具（全部 ModelVisible，priority=100）

| 产品名 | primitive 名 | tool_kind | exposure | permission_facts (workspace) |
|---|---|---|---|---|
| Present | present | Read | ModelVisible | workspace.read |
| Find | fuzzy_file_search | Search | ModelVisible | workspace.read |
| GitDiff | git_diff_remote | Read | ModelVisible | workspace.read |
| JobStart | job_start | Execute | ModelVisible | workspace.execute |
| JobOutput | job_output | Execute | ModelVisible | workspace.execute |
| JobKill | job_kill | Execute | ModelVisible | workspace.execute |
| JobList | job_list | Execute | ModelVisible | workspace.execute |

约束：
- `contract_priority` 一律为 100（与既有内置工具一致）。
- `tool_kind_for_name`：Present=Read，Find=Search，GitDiff=Read，JobStart/JobOutput/JobKill/JobList=Execute。
- `tool_exposure_for_name`：7 项全部 `ModelVisible`。
- dispatcher 派发臂调用 lib 函数：`present` / `fuzzy_file_search` / `git_diff_remote` / `job_start_with_workspace_root`（带 context workspace root）/ `job_output` / `job_kill` / `job_list`。
- canonical name、别名表、display metadata、default permission facts 七处映射同步登记，与 builtin_tools() 注册保持一一对应。

### C1.2 完整 28 项模型可见清单（逐行，权威）

1. Run
2. Ask
3. Read
4. ReadDocument
5. List
6. Search
7. Glob
8. WebFetch
9. WebSearch
10. MCPResource
11. ToolSearch
12. Write
13. Edit
14. BatchExecute
15. Plan
16. ViewImage
17. TodoWrite
18. TodoList
19. create_goal
20. get_goal
21. update_goal
22. Present
23. Find
24. GitDiff
25. JobStart
26. JobOutput
27. JobKill
28. JobList

（1–21 为现网 21 项；22–28 为本波新增。顺序仅为本清单书写顺序，断言以集合+计数为准。）

### C1.3 验收判据（测试如何断言）

- 测试 A（tools.rs `#[cfg(test)]` 基准区更新）：`characterization_product_surface_keeps_unique_names_and_json_object_schemas` 断言 `names.len() == 28`，且 28 名逐项等于 C1.2 清单（含顺序或集合断言，二选一冻结）；每个 view 的 `input_schema.type == "object"` 且含 `properties`。
- 测试 B（tests/wave37_tool_surface.rs）：调用 `builtin_turn_tool_contract_views()`，断言：
  - 恰好 28 项；
  - `Present/Find/GitDiff/JobStart/JobOutput/JobKill/JobList` 均在清单内且 exposure == ModelVisible；
  - `contract_priority` 对 7 项新工具均为 100；
  - `tool_kind_for_name` 与 permission facts 符合 C1.1 表（Job*=Execute+workspace.execute，其余 Read/Search+workspace.read）。
- 测试 C（provider/mod.rs 两处 28 名基准）：provider 侧基准数组更新为 28 名，断言与 C1.2 完全一致。
- 红相要求：在 T3 实现落地前，A/B/C 全部失败（现网为 21 项 / 静默覆盖 / 伪 subagent）。

---

## C2 write_file 观察门（read-before-write，与 edit_file wave-22 同语义）

### C2.1 契约

- 触发条件（三条同时成立才拦截）：
  1. 目标文件 `existed_before == true`（写前已存在）；
  2. 调用未携带 `expectedHash`；
  3. 环境变量 `PONY_ALLOW_UNOBSERVED_EDIT` 未设置（未设为真值）。
- 命中时返回结构化 error：
  - `code: "fs_not_observed"`
  - `recoveryHint`：指引先 Read/获取当前文件哈希后携带 `expectedHash` 重试，或显式设置 `PONY_ALLOW_UNOBSERVED_EDIT` 豁免（语义与 edit_file 现行提示一致）。
- 新文件（`existed_before == false`）不设门，直接写入。
- 携带 `expectedHash` 时按既有 edit_file 同样的哈希比对语义执行（不匹配返回对应既有错误码）。

### C2.2 验收判据

- 测试 D（tests/wave37_tool_surface.rs）：临时目录中预置文件 → 未传 expectedHash、未设 PONY_ALLOW_UNOBSERVED_EDIT 调 write_file → 断言返回 error 且 `code == "fs_not_observed"`，且包含 recoveryHint 字段/文本；断言文件内容未被修改（fail-closed）。
- 测试 E：同上但设置 `PONY_ALLOW_UNOBSERVED_EDIT=1` → 写入成功，无 `fs_not_observed`。
- 测试 F：对不存在路径（existed_before==false）直接写 → 成功，无门拦截。
- 测试 G：预置文件 + 正确 expectedHash → 写入成功。
- 红相要求：实现前 D 失败（现行为静默覆盖）。

---

## C3 子代理真实执行引擎（SubagentTurnEngine）

### C3.1 契约

```rust
pub trait SubagentTurnEngine: Send + Sync {
    fn run_turn(&self, prompt: &str, timeout_ms: u64) -> Result<SubagentRunOutcome, String>;
}

pub struct SubagentRunOutcome {
    pub final_text: String,
    pub session_id: String,
}
```

- 全局槽：`static SUBAGENT_ENGINE: OnceLock<Arc<dyn SubagentTurnEngine>>`。
  - `set_subagent_engine(eng: Arc<dyn SubagentTurnEngine>) -> Result<(), String>`：成功置入返回 `Ok(())`；已设置过返回 `Err(String)`（幂等注册保护）。
  - `subagent_engine() -> Option<Arc<dyn SubagentTurnEngine>>`。
- `HostTurnEngine`：独立 AgentRuntimeBuilder 建栈（独立 SessionStore、ProviderRegistryStore），把 prompt 包为 TurnInput 运行一个 turn，从 TurnResult 提取 final 文本与 session_id 填 `SubagentRunOutcome`。本文件（T1）仅落空壳，方法体 `unimplemented!("wave-37 executor-b fills")`；真实现归 executor-b（task-4）。
- 错误契约（T4 实现必须遵守，T2 测试冻结）：
  - `subagent_busy`：并发运行中子代理数 > 2；
  - `prompt_too_large`：prompt 字节数 > 200000；
  - `subagent_timeout`：耗时 > timeout_ms。
- 引擎由 T4 写入 `crate::agent::orchestration::SUBAGENT_ENGINE` 槽位；`SUBAGENT_RUNTIME_WIRED` 改为运行时判定（引擎槽非空即 wired）。

### C3.2 验收判据

- 测试 H（tests/wave37_tool_surface.rs）：注入成功 StubEngine → subagent() 返回真实 output（等于 Stub final_text），且 `set_subagent_engine` 二次注册返回 `Err`。
- 测试 I：注入返回 `Err("...")` 的失败 Stub → subagent() 返回结构化错误而非假 ok（不得出现伪实现字符串）。
- 测试 J：并发 > 2 次调用 → 第三次起返回 `subagent_busy`。
- 测试 K：prompt 字节 > 200000 → 返回 `prompt_too_large`。
- 测试 L：Stub sleep 超过 timeout_ms → 返回 `subagent_timeout`。
- 红相要求：实现前 H–L 全部失败。

---

## 附：本契约产物清单

1. `.dev-team/contracts/wave-37-tool-surface-subagent.md`（本文件）
2. `.dev-team/nfr-baseline-wave37.json`
3. `crates/pony-agent-core/src/agent/subagent_runtime.rs`（空桩）
4. `crates/pony-agent-core/src/agent/mod.rs` 仅追加 `pub mod subagent_runtime;` 一行接线。
