# PA-021 Implementation Slice Review

## 审核范围

- [PA-021 任务卡](/C:/Users/HUAWEI/Documents/pony-agent/management/task-system/03_TASKS/PA-021-build-skills-registry-and-bridge.md)
- [skills-registry-bridge/tasks.md](/C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/add-skills-registry-bridge/tasks.md)
- [capability_bridge.rs](/C:/Users/HUAWEI/Documents/pony-agent/src-tauri/src/agent/capability_bridge.rs)
- [control_plane.rs](/C:/Users/HUAWEI/Documents/pony-agent/src-tauri/src/agent/control_plane.rs)
- [runtime.rs](/C:/Users/HUAWEI/Documents/pony-agent/src-tauri/src/agent/runtime.rs)
- [telemetry.rs](/C:/Users/HUAWEI/Documents/pony-agent/src-tauri/src/agent/telemetry.rs)
- [lib.rs](/C:/Users/HUAWEI/Documents/pony-agent/src-tauri/src/lib.rs)

## 审核结论

本轮已完成 `PA-021` 的第一段实现闭环，可以确认它不是一层新的宿主私有脚本通道，而是复用了既有 unified capability registry boundary。

结论：

- `2.x / 3.x / 4.x` 第一批目标已达成
- `5.2` skill lineage observability 字段已打通到既有 telemetry record
- `5.1 / 5.3` 仍未完成，下一步应继续收口 planner 消费与 monitor 前端读面

## 已验证事实

1. registry model 已落地
   - 已新增 `SkillSourceSnapshot / SkillDescriptor / SkillFailureLayer`
   - skill 会保留 `composedCapabilityRefs / composedCapabilityKinds`
2. ingress boundary 复用了既有 control-plane/capability-registry 链路
   - `apply_skill_source_snapshot` 会先按当前 capability registry 归一化，再同步到 runtime 与 control-plane registry
   - 同一 `sourceId` 的旧 skill 集会被原子替换
3. `v1` runtime 只执行 tool-composed skills
   - tool-only skill 可执行
   - 含 `resource` 的 skill 会显式返回 `unsupported_composition`
4. 观测字段已进入既有 telemetry 结构
   - `skillId`
   - `skillSourceId`
   - `composedCapabilityRefs`
   - `composedCapabilityKinds`
   - `failureLayer`

## 定向验证

已通过以下 Rust 定向测试：

- `cargo test registry_normalizes_skill_snapshot_with_aggregated_permission_facts --lib`
- `cargo test registry_replaces_stale_skills_for_same_source --lib`
- `cargo test apply_skill_source_snapshot_updates_read_plane_and_runtime_registry --lib`
- `cargo test apply_skill_source_snapshot_replaces_stale_skills_for_same_source --lib`
- `cargo test skill_bridge_executes_tool_only_skill_without_second_scheduler --lib`
- `cargo test skill_bridge_rejects_non_tool_composed_skill_as_unsupported --lib`
- `cargo test skill_bridge_propagates_underlying_capability_execution_failure --lib`

## 剩余风险

- planner 仍未正式消费 normalized skill facts，`5.1` 还不能关闭
- monitor 前端还未专门展示 skill lineage，`5.3` 仍需补验
- Windows 增量编译目录出现过一次 `os error 5` 的 finalize warning，不影响当前用例结果，但后续批量验证时要继续观察
