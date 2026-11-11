# Agent Note: pa-issue-triage 技能不可调用的根因诊断与 dsh 技能加载机制对齐方案

Status: proposed

## Problem

当前 `pa-issue-triage`（`.agents/skills/pa-issue-triage`）在 pony-agent 的会话里并非处处可被成功调用：
问题单线 ISSUE-20261111-007/008 记录了「job_copilot 会话看不到 `pa-issue-triage`」与「技能调用策略无闸门」。
上一轮的根因与修复见链入记录：
[2026-11-11-skill-root-discovery-scope.md](../../implemented/bug-fix/2026-11-11-skill-root-discovery-scope.md)（仓库根上探 + `PONY_AGENT_SKILL_DIRS` + 扫描/加载同一信任枚举，已 implemented）。
经代码审读与真机探针，**「不可调用」的真实边界与本轮仍未闭合的三处缺口**如下（机器收据见文末）：

1. **按工作区根判定的可见性（已部分修复）**：技能清单与 `skill_load` 都以会话工作区根为发现根。
   在本仓库（`/home/dm/pony-agent`）及仓库子目录会话中，`pa-issue-triage` 可列可载（实测 `LOAD_OK` 6859B）；
   但在兄弟仓库（如 `/home/dm/job_copilot`）会话中该技能根本不在扫描面内，`skill_load` 返回 `unknown_skill`，
   模型只能退化为 `Run: cat ../pony-agent/.agents/skills/pa-issue-triage/SKILL.md` 代偿（job_copilot 会话日志实证，
   模型自述「原因有三，主因是我看不到这个 skill」）。修复手段依赖环境变量 `PONY_AGENT_SKILL_DIRS` 且**新会话才生效**
   ——无配置持久化面、会话内无热刷新（这正是上轮修复的剩余风险原文）。
2. **目录预算静默裁剪（未闭合，新证据）**：清单段受 64 条 / 16 KiB 预算约束（`context.rs::skill_catalog_budget`，
   裁剪在 `workspace_hotloader::render_prompt_prefix_with_budget`）。实测本仓库扫入 93 个技能（`~/.agents/skills` 91 +
   项目 8，同名去重后 93），目录仅渲染 ~53 条（16304 字节），约 40 个技能只以 ≤8 条具名标记 + 一行计数折叠，
   **模型从目录里根本无法得知其存在**——「可载而不可知」。`pa-issue-triage` 因项目层 rank 200 未被裁掉，但
   任何 rank 低（全局 510+）的技能在日常会话里都会周期性失踪。
3. **调用策略无闸门（挂号 ISSUE-20261111-008）**：frontmatter 的 `user-invocable` 只解析不消费
   （`workspace_hotloader.rs` 命中数：解析 2 处、消费 0 处），`disable-model-invocation` 全仓零支持；
   渲染侧无条件列全部技能，无「人可调 / 模型可调」两套目录；人类侧也没有确定的调用手势（TUI 斜杠命令
   无技能命名空间），用户点名只能靠自由文本碰运气。
4. **本技能自带的 CLI 缺陷（本次执行中复现）**：`scripts/pa-issue-triage.mjs::recordIssue` 的 ID 生成用
   `issues.length + 1`（而非「已有最大序号 +1」），一旦中间序号被归档（如 ISSUE-20261111-007 已归档），
   下一次记录会**复用被占用的 ID**（本次实测复现：登记 009 时实际又造出一个 `ISSUE-20261111-008`，
   与既有条目 ID 冲突，被手工改号纠正）——「记录问题」这条调用路径本身也有根因缺陷。

对比 dsh（deepseek-harness）的技能加载机制，pony-agent 缺失的是**调用策略模型、发现配置面、目录保真与热刷新**：

| 维度 | dsh（deepseek-harness） | pony-agent（现状） |
|---|---|---|
| 调用策略 | `SkillInvocationPolicy{modelInvocable,userInvocable}` 双布尔；frontmatter 用 `disable-model-invocation`/`user-invocable`，拒绝 camelCase 旧键（`packages/skill/skill-filesystem/src/index.ts::parseInvocationPolicy`） | 仅解析 `user-invocable`，无任何消费点；无 `disable-model-invocation` |
| 模型面 | 目录与 `skill` 工具双重 `isModelInvocable` 过滤，越权调用显式报错 | `skill_load` 无策略检查；目录无过滤 |
| 人类面 | `/skill-name` 首行手势，`isUserInvocable` 闸门，是 `disable-model-invocation` 技能唯一入口（`packages/skill/tool-skill/src/index.ts`） | 无；TUI 斜杠命令无技能命名空间 |
| 发现 | provider 注册表 + 分层 rank（项目 .dsh=100/.agents=200/custom=300、用户 .dsh=400/.agents=500）+ chokidar 热刷新（稳定阈值 200ms） | 单一 hotloader 枚举（含 rank 350-370 仓库根、450+ 自定义根，机制已对齐），无热刷新、无配置面 |
| 目录 | 全量目录，description 500 字符截断，digest 变更推送（会话内更新） | 64 条/16 KiB 硬预算静默裁剪 |
| 校验 | `name` 必须是 kebab-case（`isSkillName`），缺 `name`/`description` 直接忽略并告警 | 任意 name，缺省回落目录名，静默容忍 |

## Proposal

按 dsh 三包（`skill` 注册表 / `skill-filesystem` 发现 / `tool-skill` 目录与工具）的语义对齐 pony-agent，分四段实施
（对应 `.agents/issue-triage/current-plan.json` 的 PLAN-20261111-02）：

1. **双面调用策略（四象限，首要）**：
   - `workspace_hotloader` 的 `SkillFrontmatter` 新增解析 `disable-model-invocation`（bool，缺省 false），
     `HotloadedSkill` 增加 `model_invocable: bool` 字段；`user-invocable` 从「解析即弃」变为真实字段语义
     （人机双向默认 true）。
   - 渲染侧：`build_cached_prompt_prefix*` 只列 `model_invocable` 的技能，被排除者输出
     `omitted skill:<name> (model_invocation_disabled)` 独立标记行（沿用既有 omitted 契约格式）。
   - 加载侧：`skill_load` 对 `model_invocable=false` 显式返回结构化错误
     `skill_not_available_for_model_invocation`（对齐 dsh 工具层双重检查：summary 与 definition 各查一次）；
     `resolve_skill_by_name` 保持物理可载（人类面不受影响）。
   - 人类面：TUI 新增 `/skill:<name>` 斜杠手势，仅 `user_invocable` 技能可注入，作为
     `disable-model-invocation` 技能的唯一入口（对齐 dsh `/name` 手势）；输入框 `/help` 同步登记。
2. **目录预算保真（消除「可载而不可知」）**：description 目录截断从 1024 收紧到 500 字符
   （对齐 dsh `catalogDescriptionMaxLength=500`）；预算裁剪后**保留全部技能 name 行**（最短清单，
   无描述或 `(truncated)` 标注），或用独立 `skill_search(name)` 工具作为被裁技能的检索路径；
   `omitted skills: N more` 折叠行保留为计数诊断。
3. **发现配置化 + 会话内热刷新**：新增 `settings.json` 的 `skillDirs` 配置（与 `PONY_AGENT_SKILL_DIRS` 同一
   解析入口，env 为兜底）→ 跨仓库技能无需环境变量即可持久可见；引入技能根 watchdog（稳定性阈值 ~200ms，
   对齐 dsh chokidar 配置），目录/正文变更经既有 `delta_notice` 通道在**当前会话下一轮**生效，替代「新会话才生效」。
4. **frontmatter 校验强化**：新增 kebab-case name 校验（`^[a-z0-9]+(?:-[a-z0-9]+)*$`，非法 →
   `omitted skill:<dir> (invalid_name)`），缺 `name`/`description` 的候选显式标记而非静默回落目录名。
5. **triage CLI 健壮性修补**：`recordIssue` 的 ID 生成改为「当前最大数字序号 +1」（避免归档后碰撞）；
   处理逻辑保持 `fingerprint` + `status` 去重语义不变，存量序号断层不再复用。

## Alternatives considered

- **A. 仅继续扩大 `PONY_AGENT_SKILL_DIRS` 命中的根（不动架构）**：把兄弟仓库根加入枚举。
  落选：仍依赖环境变量且新会话生效，无法表达「人可调 / 模型可调」两套目录，目录裁剪问题不动——
  只修了 ISSUE-007 的某一次出现，不闭合本问题的其他两环。
- **B. 目录不做预算，照抄 dsh 全量注入**：语义与 dsh 完全一致，目录永不裁剪。
  落选：pony 目录注入是**每轮前缀的一部分**（`context.rs` semistable 段，F1-3 基线），无 dsh 的
  digest 差分机制兜底；全量注入在 93→200 技能规模下会显著抬高每轮 token 成本并稀释目录信噪比，
  且会与既有的 wave-58 预算契约（D5'）及其契约测试正面冲突。折中修正案：预算保真而非预算取消。
- **C. 用 MCP skills 能力桥收编全部本地技能**：技能走 `capability_bridge` 的 plugin 工具面。
  落选：本地技能到 MCP 之间存在信任判定（受信根）、热加载、分层 rank 的语义鸿沟，且 `skill_load`
  契约与 `tool_surface_singularity` 测试已冻结；重构成本高、收益仅为换取 MCP 侧的工具统一，
  与「本问题的最小闭合」不符。
- **D. 直接把 dsh 的 `modelInvocable`/`userInvocable` camelCase 键当契约引入**：字段命名与 dsh 零转换。
  落选：dsh 自身**拒绝** camelCase 旧键（`rejectLegacyInvocationKey`），且 pony 既有 frontmatter
  契约是 kebab-case（`user-invocable`、`allowed-tools`）；按 dsh 规范用 `disable-model-invocation` 才是对齐风向。

## Acceptance criteria

1. 四象限契约测试全绿：`{user-invocable:false, disable-model-invocation:true} × 模型面/人类面` 可达性断言
   （目录可见性、`skill_load` 结果、`/skill:` 手势注入）——沿用 `tests/wave42_skill_body.rs`/`wave58_*` 用例形态。
2. 预算保真探针断言：构造 100+ 技能工作区，预算裁剪后任一被裁技能仍存在可检索路径（最短清单或 `skill_search`），
   且 `omitted skills: N more` 计数行准确。
3. watchdog 单测：目录变更在稳定阈值后进入下一轮 `delta_notice`，同一会话内可见新技能（F1-3 基线不回退）。
4. frontmatter 校验单测：非法 name / 缺 description 命中 `omitted skill:<name> (invalid_name)` 标记。
5. 回归：`cargo test -p pony-agent-core` 既有 1463 例 lib 单测 + skill 相关集成套件全绿；`verify-note.sh` exit 0。
6. 真机收据：job_copilot 会话内无需环境变量即可 `skill_load("pa-issue-triage")` 返回正文（`tool_status=ok` 日志）。
7. triage CLI 回归：`tests/pa-issue-triage.spec.ts` 全绿，且新增用例覆盖「归档后 ID 不复用」（断言最大序号 +1）。

## Risks

- **双面策略语义误判**：若 `disable-model-invocation` 在渲染/加载两侧有一侧漏查，会出现「目录列了但
  `skill_load` 404」或「目录藏了但模型仍可调」的剪刀差——两侧必须共用同一谓词（复用
  `is_trusted_skill_file` 同款「扫描与加载同一判定」的既有设计约束）。
- **最短清单稀释信噪比**：全量 name 保留会降低目录命中率；需以「描述截断优先、name 兜底」的顺序收缩。
- **热刷新与 KV-cache 基线竞争**：会话内目录变更若直接改写 semistable 段会破坏 F1-3 的「第 2 轮起不变」
  契约——必须严格走 `delta_notice` 通道（volatile 段追加），与 dsh 的「目录 digest 差分更新」同构。
- **`/skill:` 手势与既有斜杠命名空间冲突**：需在 `AVAILABLE_COMMANDS` 登记并做 unknown-command 处理
  的一致性测试（现有 `unknown_slash_command` 用例形态可复用）。

## 真机收据（本次探针与日志）

- 探针（临时集成测试，跑完已删除；`cargo test --test pa_triage_probe` exit 0）：
  - `scan_skills(/home/dm/pony-agent)` → total=93；`pa-issue-triage` = Project/rank 200；
    预算目录 `(16KiB, 64)` 渲染 16304B/56 行 → 约 40 个技能仅剩折叠标记；`skill_load` → `LOAD_OK` 6859B。
  - `scan_skills(/home/dm/pony-agent/crates/pony-agent-core)`（子目录会话）→ rank 360（仓库根上探命中）、`LOAD_OK`。
  - `scan_skills(/home/dm/job_copilot)` → `has_pa=false`、`RESOLVE_NONE`（`skill_load` 将返回 `unknown_skill`）。
  - `scan_skills(/home/dm/job_copilot)` + custom root `.agents/skills` → `pa-issue-triage`=Global/rank 450、目录可见。
- job_copilot 会话日志（`~/.pony-agent/tui-logs/tui-runtime.log`）：
  `[pony-runtime] turn:start … message_preview=为什么一开始没有触发调用pa-issue-triage这个skill？`；
  请求体内 Developer 消息自述「原因有三，主因是我看不到这个 skill」；模型以
  `Run: cd ../pony-agent && cat …/pa-issue-triage/SKILL.md`（日志 6098650/6174908）代偿 `skill_load`。