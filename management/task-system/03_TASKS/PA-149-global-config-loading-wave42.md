# PA-149 ~ PA-152: wave-42 全局配置加载四项能力（技能正文 / 指令预算 / 层级发现 / 触达即载）

## 状态
- **状态**: Review（实现 + 双 L3 复审 + L3-P 五问完成；待收口提交）
- **负责人**: Dev Team（Lead / Protocol / Test / Executor-A / Executor-B）
- **创建日期**: 2026-10-21
- **实施日期**: 2026-10-21
- **基线**: `c1bd454`（PA-148 全局指令/技能分层）→ 协议冻结 `7d90340` → 红相锚定 `6e1bf18`

## 需求背景
对比 DSH 源码（`agent-instructions` / `skill-filesystem` / `tool-skill`）后定位 pony-agent 的四项结构性缺口：
技能只可见不可用（正文永不进上下文、全局技能在工作区外读不到）、指令注入无预算（大文件顶爆前缀）、
层级只认 workspace 根一层（无 `.git` 根/祖先链/`*.local.md`/去重）、深层目录指令不随触碰注入。

## 交付契约
- **PA-149 / C1 技能正文按需加载**：`skill_load {"name"}` 工具（模型面 28→29、内部 44→45）；
  受信只读根 = 全局 `<home>/skills` + workspace `.agents|.claude|.dsh/skills`；canonical 包含 + 普通文件 +
  ≤262144B；错误 fail-closed（`OutOfAllowedRoot`/`TooLarge`/`Unreadable`）；权限事实 `skills.read` 仅映射只读。
- **PA-150 / C2 指令预算**：`InstructionBudget{65536,1048576}`；最高优先级层**截断**（`truncated <path>`），
  低优先级层**省略**（`omitted <path>`）；默认预算输出逐字节兼容（golden bytes 断言）；会话基线固化预算，变更走 notice。
- **PA-151 / C3 层级发现**：`find_project_root(cwd,marker,bound)` + `ancestor_chain(root,cwd)` 有界原语；
  宪法发现**不做无界上探**（默认 root=cwd=workspace_root）；local overlay 晚于 base；跨层 trim 去重保留最高优先级；
  缓存持久化**去重前全部来源**指纹；技能 rank 显式化（100/200/300/400/450+/500/510/520，项目恒优先）。
- **PA-152 / C4 触达即载**：read/write/edit 成功结果前置 `=== Additional instructions (<path>) (…treat as guidance data…) ===`；
  每候选文件 canonical 包含 + 普通文件门禁（阻断符号链接逃逸）；会话聚合 ≤64KiB、64 桶 LRU、`session:`/`ws:` 分桶、
  锁内重判保证并发幂等。

## 验收收据（物理）
- 红相锚定：`test(wave-42-red)` @ `6e1bf18`，20 例全红（unimplemented ×17 + 断言不成立 ×3）
- `cargo test -p pony-agent-core --test wave42_global_config`: **20 passed / 0 failed**（含符号链接逃逸、8 线程并发去重、巨项目层截断、去重来源失效、golden bytes）
- `cargo test -p pony-agent-core --test wave42_skill_body`: **6 passed / 0 failed**
- `cargo test -p pony-agent-core --test wave37_tool_surface`: **9 passed / 0 failed**（T1 迁移至 29 项）
- `cargo test -p pony-agent-core --test global_agent_config_contract` / `context_cache_red_contract`: 1 / 4 passed
- `cargo test -p pony-agent-core --lib`: **1084 passed / 1 failed**（`agent::config::tests::missing_capabilities_fields_use_stable_defaults`，
  已在干净 HEAD worktree 复现，属基线红相）
- 冒烟：`target/debug/pony --version` → `pony 0.1.127 · linux-x86_64`，EXIT 0
- NFR：`dev-nfr-lint.py .dev-team/nfr-baseline-wave42.json` EXIT 0
- 双 L3 审查首轮均 FAIL（A：缓存失效不完备；B：符号链接逃逸 / 项目层被丢弃 / TOCTOU / fail-open 等 8 项），
  修复轮后复审见 `.dev-team/review-w42-{A,B}-r2.json`；L3-P 五问见 `.dev-team/review-w42-l3p.json`
- 证据目录：`.dev-team/evidence/wave42/gates.md`；契约：`.dev-team/contract-matrix-wave42-global-config.md`
- ADR：[`docs/decisions/0046-global-config-loading-wave42.md`](../../docs/decisions/0046-global-config-loading-wave42.md)

## 非目标
- 文件 watcher（每轮 mtime/size 重扫等价轮级感知）
- `disable-model-invocation` / `whenToUse` 的模型面语义
- 技能资源目录（`references/`、`scripts/`）的可读代理、企业策略层
