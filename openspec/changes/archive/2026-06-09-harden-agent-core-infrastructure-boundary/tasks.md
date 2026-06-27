# Tasks: Harden Agent Core Infrastructure Boundary

## 1. Spec And Task-System Alignment

- [x] 1.1 新增 `PA-044` 任务卡，明确 agent core infrastructure boundary 的目标、范围和验收标准
- [x] 1.2 新增 OpenSpec change：`harden-agent-core-infrastructure-boundary`
- [x] 1.3 将本轮 core 审核发现转写为 proposal / design / delta spec / tasks
- [x] 1.4 对 spec 做一轮独立审核，确认没有把实现细节误写成产品 contract

## 2. Construction Boundary

- [x] 2.1 设计并实现 `AgentRuntimeBuilder` 或等价稳定构造 API
- [x] 2.2 设计并实现 `HostControlPlaneBuilder` 或等价稳定构造 API
- [x] 2.3 让 session backend、graph store、provider resolver、tool executor、workspace root、secret store 可以由非测试宿主注入
- [x] 2.4 将当前桌面默认构造收口为 desktop preset 或等价 host preset

## 3. Package Boundary

- [x] 3.1 建立 `pony-agent-core` 独立 crate / workspace member，或先建立可审计的 Tauri-free core target
- [x] 3.2 将 Tauri-free agent modules 迁入 core boundary
- [x] 3.3 确认 core package 不依赖 `tauri` / `tauri-build`
- [x] 3.4 让 `src-tauri` 只作为 desktop adapter 依赖 core

## 4. Host Preset And Adapter Boundary

- [x] 4.1 将 local data path、graph run path、provider registry path、secret backend fallback 从 core default 改为 host preset
- [x] 4.2 将 `current_dir` workspace root 改为可注入 workspace policy
- [x] 4.3 保持 Tauri event names 与 existing frontend command contract 不变
- [x] 4.4 明确 SSE/HTTP/CLI adapter 只能消费 core command/event contract，不复制 core runtime/session/tool semantics

## 5. Second Host Proof

- [x] 5.1 新增非 Tauri harness，使用 builder 显式构造 core
- [x] 5.2 覆盖 sync turn 与 stream turn
- [x] 5.3 覆盖 graph run stream 或最小 graph control path
- [x] 5.4 覆盖 injected workspace root 的 tool execution
- [x] 5.5 覆盖 injected session/graph storage 的 persistence roundtrip

## 6. Verification

- [x] 6.1 `rg "tauri::|AppHandle|State<|Emitter|Manager"` 在 core package / target 内无命中
- [x] 6.2 core-only cargo check/test 通过
- [x] 6.3 Tauri desktop cargo check/test 通过
- [x] 6.4 现有 frontend command/event contract 回归通过
- [x] 6.5 non-Tauri harness smoke test 通过
- [x] 6.6 更新架构文档，明确 Tauri 是 first host adapter，不是 core ownership boundary
- [x] 6.7 完成 acceptance audit，并同步任务卡 / OpenSpec 状态
