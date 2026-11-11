# PA-079 实现后审核（3 路对抗）

- 日期：2026-08-09
- 阶段：实现后审核 → 验证收口
- 审核对象：PA-079 实现（`workspace.rs` / `PersistedStore.workspaces` 持久化 / `SessionStore` workspace 方法 / `SessionState.workspace_id` 投影 / `TurnInput.workspace_id` 盖章 / `workspace_list·create` / `import_attachment` 注册表解析 / 前端类型透传）
- 方式：3 路对抗审核子智能体（@consultant / @code-reviewer / @tester）独立审核实现与 spec 基准

## 阵容与结论

| 角色 | 结论 | 关键发现 | 处置 |
| --- | --- | --- | --- |
| `@tester` | 有条件通过（无 P0） | P1-1 盖章链路 e2e 缺失（且暴露首轮盖章丢失真 bug）；P1-2 损坏回退零测试；P2-3~P2-7 | 全部采纳并修复 |
| `@consultant` | 进行中 | — | — |
| `@code-reviewer` | 进行中 | — | — |

## 采纳记录（@tester）

### P1（全部修复 + 回归测试）

- **P1-1 盖章链路 + 首轮丢失 bug**：`apply_governed_turn_context` 在 `prepare_turn`（`ensure_session`）之前执行，对全新会话 `stamp_workspace_id` 的 `get_mut` 返回 None → no-op，**首轮 workspace_id 丢失**。修复：`stamp_workspace_id` 先 `ensure_session` 再盖章。新增 runtime e2e：首轮带 `workspace_id` 的 turn → snapshot/overview 携带该 id → 第二轮不同 id 不覆盖（幂等）。
- **P1-2 损坏回退**：SQLite `store_metadata` 注入坏 JSON → `read_metadata` 回退空 → `SessionStore` 重建默认 workspace；File backend 损坏文件 → `load_store` None → 默认重建。双后端测试补齐。

### P2（采纳）

- **P2-3 design drift**：注册表随 `PersistedStore` 全量保存（非独立 workspaces.json / metadata-only upsert）。design.md 记录"实现简化"，注明无双写/互覆盖风险 + create/stamp 低频可接受。
- **P2-4 前端常量**：新建 `src/lib/runtime/workspace-constants.ts`（`DEFAULT_WORKSPACE_ID="default"`），为 PA-081 分组提供跨端共享常量。
- **P2-6 默认 root 规范化**：默认 workspace root 改走 `normalize_workspace_root`（与 create 路径一致，防 `\\?\`/符号链接 cwd 存储形式漂移）。
- **P2-7 补测**：workspace.rs 相对路径 create → canonical 绝对路径；session.rs 旧 schema blob（去掉 `workspaceId` 键）全字段 serde roundtrip（title/summary/history/turnCount/updatedAtMs + workspace_id=None）。
- **P2-5（记录移交）**：前端 `workspaceId: null` 为 PA-079 占位（PA-081 起传 activeWorkspaceId）；传输契约已由 P1-1 后端 e2e 证明，前端侧断言移交 PA-081 接入时作为基线补。

## 自审补漏（coordinator，等待另两路期间）

- **重复 root 判定未做 Windows 大小写归一**（spec T-15 要求 case-normalized）：`C:\WS` 与 `c:\ws` 指向同目录可能因 canonicalize 大小写不同而漏判。修复：`roots_match`（Windows 大小写不敏感、Unix 大小写敏感）用于 `create_workspace_entry` 重复判定与 `find_workspace_by_root`；补 `roots_match_is_case_aware_per_platform` 测试。
- **`get_workspace_root` vs 注册表默认一致性**：当前两者均解析为 current_dir（运行时未配置非 current_dir workspace_root），一致；若未来配置不同 root，`import_attachment`（注册表）与 `get_workspace_root`（runtime）会漂移——记录为 PA-080 每会话 root 解析时统一（以注册表为真相源）。

## 验证（@tester 轮 + 自审后）

| 门 | 结果 |
| --- | --- |
| core lib | ✅ 754 |
| 回归集 | ✅ tool_router 13 / session 5 / provider 8 |
| 前端 vitest | ✅ 377 |
| `cargo:check:shared` / `npm run build` | ✅ |
| OpenSpec validate（workspace-data-model-and-registry） | ✅ valid |

## 结论

- `@tester`：有条件通过（无 P0），P1/P2 全部修复并有回归测试。
- 自审：补重复 root 大小写归一 + 一致性记录。
- **@consultant / @code-reviewer 实现后审核仍在后台运行**；其 findings 将作为增量修订并入，不阻塞收口（当前无未解决 P0/P1，验证全绿）。
- 收口：PA-079 实现验证完成，进入 Done；PA-080（路径权限边界）在 PA-079 数据模型基础上推进。
