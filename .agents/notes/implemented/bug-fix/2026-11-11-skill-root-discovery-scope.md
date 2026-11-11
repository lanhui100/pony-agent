# Agent Note: 技能根发现补全仓库根上探与自定义技能根

Status: implemented

## Problem

`WorkspaceHotloader` 的技能根发现只认「会话根单层」，导致三处可见性缺口（问题单 ISSUE-20261111-007）：

1. `discover_skill_dirs` 只扫描 `workspace_root/{.dsh,.agents,.claude}/skills` 三层，没有 `.git` 上探。
   会话 cwd 落在仓库子目录时（如 `crates/pony-agent-core`），仓库根的 `.agents/skills` 完全不可见；
   而宪法发现（`discover_constitution_sources`）已有祖先链语义，两个子系统口径不一致。
2. 没有任何「用户显式声明额外技能根」的入口。`/home/dm/pony-agent` 与 `/home/dm/job_copilot` 是兄弟仓库，
   前者 `.agents/skills/pa-issue-triage` 既不是后者的祖先也不是全局根，于是在 job_copilot 会话里
   `pa-issue-triage` 从未进入可用技能清单（本次会话由用户点名才触发，正是这条缺陷的直接后果）。
3. `GlobalRootIdentity::rank` 的冻结表声明 `~/.dsh/skills`=500，但 `resolve_global_agent_homes_from`
   从未 push `home.join(".dsh")`，该分支永不命中——声明与实现不一致。
4. 加载侧另有一套根清单（`path_permission::WORKSPACE_SKILL_DIRS` + `trusted_skill_roots{,_with_policy}`），
   与扫描侧各自演算：任何一侧新增根而另一侧没跟上，就会复现历史缺陷 P1-6「列得出、载不进」。

## Decision

1. **仓库根上探**：新增 `find_git_root(start)`（探测 `.git` 目录**或文件**，兼容 worktree / submodule，上探上限 32 层）。
   `enumerate_project_skill_roots(workspace_root)` 在会话根三层之外补仓库根三层，
   rank = 350(`.dsh`) / 360(`.agents`) / 370(`.claude`)，**整体劣于**会话根 100/200/300 ⇒
   「近者覆盖远者」成立，且项目带恒小于全局带（400+）的不变量不破。命中不到 `.git` 即退回会话根单层。
2. **自定义技能根**：新增环境变量 `PONY_AGENT_SKILL_DIRS`（路径分隔符分隔，每项是**技能根目录本身**，
   其下直接放技能目录）。解析入口 `resolve_custom_skill_dirs{,_from}`；这些根以 `Global` 作用域进入清单，
   rank = 450 + index（封顶 499）——劣于产品自有根（400），优于 `~/.dsh`(500)/`~/.agents`(510)/`~/.claude`(520)，
   即 dsh 的「customSkillDirs 优于用户全局根」语义。它们**不参与宪法分层**，避免新增全局指令层导致提示词基线漂移。
3. **`.dsh` 全局根兑现声明**：`~/.dsh/skills` 由 `enumerate_global_skill_roots` 单独补齐（rank 经
   `GlobalRootIdentity::rank` 得 500），使冻结表从「文档」变成「可达代码」；仍不进入宪法分层。
4. **单一枚举 + 分域信任**：新增 `SkillRootSpec { scope, dir, rank, containment_base }` 与
   `enumerate_skill_roots` / `enumerate_global_skill_roots` / `enumerate_project_skill_roots`。
   扫描侧（`scan_skills_internal`）与加载侧（`path_permission::trusted_skill_roots{,_with_policy}`）
   都消费同一枚举，`WORKSPACE_SKILL_DIRS` 第二套真源随之删除。`containment_base` 把项目根的
   canonical 包含上界从「会话根」扩到「所属项目域」（仓库根或会话根）：仓库根技能可列可载，
   而 `<ws>/.claude/skills -> /tmp/evil` 这类逃逸仍整根不采纳。

## Alternatives considered

- **只加自定义根、不做 `.git` 上探**：能解兄弟仓库场景（本次最痛的一条），但子目录 cwd 下仓库根技能仍失踪，
  且与宪法发现的祖先链语义继续背离；放弃。
- **把祖先链无界上探（copy dsh 的 global → ancestors → exact scope 全链）**：中段目录也是候选根，
  会在 monorepo 里引入多级同名竞争与更多 canonical 越界判定，收益仅覆盖「嵌套子仓库」这一罕见形态；
  改为「仓库根一层 + 会话根一层」，语义可判定、rank 表可冻结。
- **把 `.dsh` 直接塞进 `resolve_global_agent_homes_from`**：rank 500 分支同时变可达，但宪法分层会新收
  `~/.dsh/AGENTS.md`（本机 1490 B），等于未经评审变更所有会话的提示词基线；改为技能面专用的单独补齐。
- **自定义根走 `settings.json` 的 `skillDirs` 字段**：需要 AppSettings 与 hotloader 之间的新耦合、
  设置页 UI 与 schema 同步，成本远高于收益；环境变量已满足「用户显式声明」，且便于用例注入与 CI 隔离。
- **让自定义根复用项目带 rank（<400）**：会把用户级根伪装成项目层，破坏「项目层永远最高优先级」不变量；
  按全局带兜底段（450+）落位。

## Consequences

- 行为变化（用户可见）：仓库子目录启动时会额外列出仓库根技能（rank 35x，`[project]`）；
  设置 `PONY_AGENT_SKILL_DIRS` 后跨仓库技能以 `[global]` 出现；`~/.dsh/skills` 若存在则进入清单。
- 契约测试：`crates/pony-agent-core/tests/skill_root_discovery_contract.rs`（9 例：上探 / worktree 文件形态 /
  近者覆盖 / 无 `.git` 不越界 / 自定义根作用域与 rank / 同名覆盖 / 仓库根与自定义根可加载 / 逃逸负样本）；
  模块内新增 `test_dsh_global_root_rank_is_reachable` 与 `test_custom_skill_dirs_parse_and_dedupe`。
- 变更面：`crates/pony-agent-core/src/agent/workspace_hotloader.rs`、
  `crates/pony-agent-core/src/agent/path_permission.rs`。
- 真机收据：`scan_skills("/home/dm/job_copilot")` + `PONY_AGENT_SKILL_DIRS=/home/dm/pony-agent/.agents/skills`
  → `pa-issue-triage` 以 `Global`/rank 450 出现在清单（共 101 条），即本次缺陷的原始场景已闭合。
- 剩余风险：本次会话进程在启动时已完成技能注入，故当前对话仍看不到该技能；需在**新会话**里
  export `PONY_AGENT_SKILL_DIRS=/home/dm/pony-agent/.agents/skills` 后生效（本仓不改用户 shell 配置）。
