# Agent Note: 技能有界前缀读取的多字节截断修复（skill_load unknown_skill）

Status: implemented

工单：B234。影响文件：`crates/pony-agent-core/src/agent/workspace_hotloader.rs`（+ 回归用例）。

## Problem

现场：会话 pa_01a1e905 step 22 `skill_load{name:"dev-team"}` 返回 `{kind:"unknown_skill"}`，
全局技能 `~/.agents/skills/dev-team/SKILL.md`（83KB < `MAX_SKILL_BODY_BYTES`=262144）明明存在；
模型遂退化为裸 Read 全局路径，触发工作区外审批并挂起整轮。

诊断（临时探针 + 复现）：扫描确实覆盖了全局根（rank=510），但该技能被记
`omitted skill:dev-team (NotAFile)`。根因在**有界前缀读取**：

- 扫描用 `FileFingerprint::compute_bounded(path, SKILL_FRONTMATTER_READ_LIMIT=65536)`；
- `file.take(limit)` 恰好读满 65536 字节（不读第 65537 字节）；
- 旧 `utf8_safe_prefix_len` 开头 `if bytes.len() <= max_bytes { return bytes.len(); }`，
  读满时 `bytes.len() == max_bytes` **直接返回**，跳过字符边界回退；
- 该文件第 65536 字节落在汉字中间 ⇒ 末尾残缺 UTF-8 序列进入 `String::from_utf8` 失败
  ⇒ 候选被丢弃并归因为 `NotAFile`（掩盖了真实原因）。

受害区间：正文 > 64KiB 且 65536 字节边界切裂多字节字符的技能（本仓 `design-review` 137KB
等恰好切在字符边界而侥幸可用，故该缺陷长期未暴露）。

## Decision

- `utf8_safe_prefix_len` 改为返回 `Result<usize, usize>`：`Ok(n)` = `bytes[..n]` 是不超过
  `max_bytes` 的**最长合法 UTF-8 前缀**；`Err` = 前缀内部存在非法字节序列（`error_len() == Some(_)`），
  调用方 fail-closed。
- `compute_bounded`：超限判定仍按真实 `file_size`（语义不变），截断按边界回退结果执行；
  内部非法 UTF-8 仍返回 `None`（不放宽）。
- `ConstitutionSourceSnapshot::capture` 同步消费新签名：只有尾部残缺才回退，内部非法仍
  `read_ok=false`。

## Alternatives considered

- **A. 只按文件大小跳过超大技能（`too_large`）。** 落选：现场文件 83KB 远小于 240KB 上限，
  这不是「太大」而是「边界切裂」；跳过会丢功能。
- **B. 把前缀读取上限提到 256KB 避开 64KiB 边界。** 落选：治标；任何上限都会在其它文件上
  重现同一类 bug。
- **C. 扫描失败时回退为「按需全文加载」。** 落选：扫描/加载必须同一信任谓词（D1 契约），
  且会把 64KiB 的 O(1) 开销变成全文件读取，绕过既有的有界预检语义。

## Verification

- `cargo-lane test -p pony-agent-core --lib workspace_hotloader`（19 passed，含新增 2 例）；
- `cargo-lane test -p pony-agent-core --test skill_root_discovery_contract`（10 passed，含
  `oversized_global_skill_with_split_multibyte_prefix_is_resolvable`：临时 `PONY_AGENT_HOME`
  伪造全局根 + >64KiB 且 65536 边界切裂 3 字节字符）；
- 红灯复现：把截断逻辑改回旧行为，新增用例 exit 101；恢复后通过；
- 真实现场：`resolve_skill_by_name("/home/dm/pony-agent","dev-team")` → 全局根命中。
