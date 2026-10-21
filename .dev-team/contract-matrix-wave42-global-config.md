# Wave-42 契约矩阵：全局配置加载四项能力（PA-149..PA-152）

基线 commit：`c1bd454`（PA-148 全局指令/技能分层已落盘）
分级：**B 级**（跨 5 个模块、触碰 workspace 边界与工具面、含安全白名单）
判定依据：改动文件 > 3；触碰 `tools.rs`/`path_permission.rs` 安全边界；`write_scopes` 跨 4 个目录。

## 冻结契约（Test Agent 依此写红相；Executor 只读）

### C1（task-1 / PA-149）技能正文按需加载
| 契约 | 断言点 | 反例 |
|---|---|---|
| 白名单内技能可加载正文 | `load_skill_body` 返回 `content` 含 SKILL.md 正文，`scope` 与扫描结果一致 | — |
| 越界路径 fail-closed | 伪造 `HotloadedSkill { path: /etc/passwd }` → `Err(OutOfAllowedRoot)` | 静默返回空正文 |
| 超限拒绝 | 文件 > `max_bytes` → `Err(TooLarge{size,limit})` | 截断返回成功 |
| 未知名 | `resolve_skill_by_name` → `None`；工具面返回结构化失败 | 返回空字符串成功 |
| 工具注册 | 模型可见工具数 28 → 29，`skill_load` 在 schema 列表内 | 只加实现未注册 |

### C2（task-2 / PA-150）指令字节预算
| 契约 | 断言点 | 反例 |
|---|---|---|
| 默认预算零漂移 | 默认 `InstructionBudget` 下输出与 `resolve_constitution_with_globals` 字节相等 | 多出空行/标题 |
| 总量超限省略低优先级 | 低优先级全局层被丢弃，输出含 `omitted <path>` | 丢项目层 |
| 单文件超限跳过 | 超 `max_source_bytes` 层不出现、计入 omitted | 载入被截断内容 |
| 预算变更不改头 | 会话第 2 轮改预算 → 静态前缀字节不变 + 返回 notice | 前缀被重写 |

### C3（task-3 / PA-151）层级发现
| 契约 | 断言点 | 反例 |
|---|---|---|
| git 根探测 | `find_project_root(cwd,".git",bound)` 上移到含 `.git` 目录 | 越过 bound 继续上爬 |
| 祖先链 | `ancestor_chain(root,cwd)` = root→cwd 顺序，逐级指令全载 | 只载 root |
| local overlay | `AGENTS.local.md` 排在同目录 `AGENTS.md` 之后 | 被忽略 |
| 去重 | trim 后相同内容只出现一次（保留高优先级） | 重复注入 |
| rank | 同名技能 rank 小者胜；rank 相同 project 胜 | 依赖遍历顺序 |

### C4（task-4 / PA-152）触达即载
| 契约 | 断言点 | 反例 |
|---|---|---|
| 深层触达 | read/write/edit 深层文件后结果含该目录 AGENTS.md 段落，浅→深 | 只载根 |
| 会话去重 | 同路径二次触达不重复注入 | 每轮重复 |
| 越界不注入 | workspace 外路径不产生段落 | 注入 home 指令 |
| 截断 | > `MAX_TOUCH_STEP_BYTES` 截断并带标记 | 原文全量注入 |

## 写域（正交）
- task-1：`agent/skill_body.rs`、`agent/path_permission.rs`、`agent/tools.rs`、`agent/mod.rs`
- task-2：`agent/workspace_hotloader.rs`、`agent/context.rs`
- task-3：`agent/workspace_hotloader.rs`（**blocked_by task-2**，串行）
- task-4：`agent/instruction_touch.rs`、`agent/tools.rs`、`agent/mod.rs`（**blocked_by task-1, task-3**）
- 测试写域（Test Agent 独占）：`crates/pony-agent-core/tests/wave42_*.rs`

## 红线
1. Executor 不得修改 `tests/wave42_*`（红相锚定后只读）。
2. 全局技能正文读取必须经 `is_within_trusted_readonly_roots`，禁止直接 `fs::read_to_string` 任意路径。
3. 触达注入只能"前置追加"，不得改变既有工具结果关键字段。
4. 默认配置下行为必须与 `c1bd454` 字节一致（C2-1 回归断言守此）。
