# Design: core-hotfile-refactor

## 边界切分

### Line A（独立并行）：runtime/mod.rs 测试外移

- 定位（rustc 裁决，见 `.tmp/p1p2-scratch/ENGINEER-BRIEFING.md`）：物理 16,588 行；行 7,063 `#[cfg(test)]`、行 7,064 `mod tests {`，tests 区延伸至 EOF，内含嵌套 `mod parity`（行 15,574 起）随整块搬移。
- 操作：
  1. 执行已验证脚本 `.tmp/p1p2-scratch/cut-runtime.ps1`（DryRun 断言已全绿：bare-LF/无 BOM/moved=9,523 行），**整块逐字搬移** body 至 `runtime/tests.rs`——严禁 dedent 或任何逐行改写（测试区多 raw-string，dedent 会静默改变字符串内容）；随后仅对新文件跑 `rustfmt --edition 2021` 规范缩进；
  2. 原位替换为 `#[cfg(test)]\nmod tests;` 两行声明。
- 语义保证：内联 `mod tests` 与文件式 `mod tests;` 在模块树中路径完全一致（`crate::agent::runtime::tests`），`use super::*` 等引用零变化。

### Line B（串行两步）：session.rs → session/ 目录

Step B1（P1-b）：先按行号把 `mod tests { … }`（6,219 行起）外移为 `session_tests.tmp`（暂存，不参与编译）。
Step B2（P2）：生产代码按段落边界切分入目录：

```
session/
├── mod.rs           // 子模块声明 + 重导出（唯一新增"代码"）
├── types.rs         // 数据模型 + EnvironmentInfo/collect_env_info/read_git_branch + 常量/类型别名
├── backend.rs       // SessionBackend trait + PersistCommand/TurnStateRecord/TraceTerminalPatch/
│                    //   SessionMetaPatch/PersistCommandOutcome/SeparateTraceTableMode/
│                    //   TraceMigrationState/SessionBackend*结果枚举/SessionTraceMutation
├── file_backend.rs  // FileSessionBackend + #[cfg(test)] MemorySessionBackend
├── store.rs         // SessionStore 结构与全部 impl、文件私有辅助函数（含 unique_test_session_dir，
│                    //   物理位于原文件 5772 行、被 store 侧 1110/5750 引用，归 store 射程）
└── tests.rs         // B1 外移的测试（恢复编译，`use super::*` 不变）
```

## 可见性策略（关键风险点）

原 session.rs 中文件私有项（如 `fold_session_traces_all`、`refresh_session_metadata`、`materialize_last_turn_messages`、`default_sessions`、`now_timestamp_ms`、`reject_stale_cursor_version`、`sanitize_*`、`rebuild_*`、`ensure_history_graph`、`snapshot_from_state`、`default_snapshot_for_session`、`attachment_assets_for_query`、`default_attachment_root` 等）跨子模块使用时：

- 最小可见性原则：仅被 session/ 内部使用的项标 `pub(super)`；被 crate 其他模块直接引用的原 `pub(crate)` 项维持 `pub(crate)` 并在 mod.rs `pub(crate) use` 重导出。
- **可见性上限规则**：重导出可见性 ≤ 被导项声明可见性——原 `pub(crate)` 项在新子模块中必须仍声明为 `pub(crate)`，不得降为 `pub(super)` 后借重导出抬升。
- 原 `pub` 项一律经 mod.rs `pub use` 保持外部路径与可见性完全不变。
- **全部采用定向重导出，禁止对任何子模块做 blanket glob 重导出**（types 区段含私有别名 `SessionMap`，glob 即触发 E0364）。`SessionMap` 由 mod.rs 以私有 `use types::SessionMap;` 保持 crate 内可解析即可——子模块经父模块隐私规则可解析该绑定。
- 禁止借机放宽任何可见性到超出原有等效范围（reviewer 专项检查项）；mod.rs 中同名不得重复 use/pub use。

tests.rs 兼容性：旧测试 `use super::*` 指向 session 模块本身；mod.rs 的重导出（含 `pub(crate) use`）使 glob 导入继续解析全部既有名字。若个别名字因移动后可见性不足而失联，允许在 mod.rs 追加针对性 `pub(crate) use`，不允许改测试断言。

## 实施方式约束（防错核心）

- 一律脚本切割（pwsh 按行号/正则标记读写），LLM 只负责决定边界行号与事后校验，禁止逐行手抄大文件。
- 切割完成后必须跑：`cargo check -p pony-agent-core` → 修复失联名字 → `git diff --stat` 核对只有预期文件变动。
- 每条 Line 结束即独立可编译可测，两 Line 合并后再走全量门禁。

## 替代方案与否决理由

1. **一次性连 runtime 生产代码一起拆**（否决）：runtime 是核心执行路径且改动最频繁，缺乏内聚性分析前强拆风险大于收益 → P3 另立。
2. **用 `include!` 拼接避免移动代码**（否决）：破坏 IDE/rg 导航与 rust-analyzer 模块语义，是反模式。
3. **测试移入顶层 `tests/` 集成测试**（否决）：这些是模块私有的单元测试，依赖 `super::*` 私有项，集成化会强迫放宽可见性，违背最小变更原则。

## 回滚

全部改动限定于 `crates/pony-agent-core/src/agent/{runtime,session*}`，**按路径域隔离回滚**（不依赖中间 commit，遵守"未经用户要求不擅自提交"）：
- 回滚 Line A：`git checkout -- crates/pony-agent-core/src/agent/runtime/mod.rs && git clean -f -- crates/pony-agent-core/src/agent/runtime/tests.rs`
- 回滚 Line B：`git checkout -- crates/pony-agent-core/src/agent/session.rs && git clean -fd -- crates/pony-agent-core/src/agent/session/`
两域文件集不相交，互不牵连；checkout 后必须做 CR 计数断言（autocrlf 环境下 checkout 可能 CRLF 化，若发生则以 `.tmp/p1p2-scratch` 字节快照恢复）。不涉及数据迁移与持久化格式（serde 输出逐字节不变）。

## 验证策略

1. 基线先行：重构前 `npm run cargo:test` exit 0 全绿（已完成，job pwsh-2）；存量 warning 7 个为基线集合。
2. 结构守恒校验（脚本化 gate，不做人工抽查）：字面量多集逐字节比对（复用 `.tmp/p1p2-scratch` 探针方法：raw + 普通字符串排序比较，before/after 全等）；P1 后 head 区哈希一致。
3. warning 集合对比：编译输出 warning 按"文件+符号"归因对比基线 7 条——零新增（无 deny(warnings)，check 通过会吞掉新增 warning，必须显式比对）。
4. 终门禁：`npm run cargo:test` 无新增失败 + 下游 `cargo check` 干净 + fmt 口径一致。
