# ADR 0120：插件宿主的 Windows 脚本命令解析（shebang → 解释器）

Status: implemented

## 背景

- ADR 0067 决策 1 把插件载体定为「可执行程序 + stdio JSON-RPC 2.0」，落点为
  `crates/pony-agent-core/src/agent/plugin_host/{supervisor.rs,transport.rs}`：`spawn` 只接一个
  `command: PathBuf`，宿主 `Command::new(command)` 直接启动。
- 该语义在 Unix 成立：`.py`/`.sh` 这类带 shebang 的脚本，由内核在 `execve` 时按 `#!` 起解释器。
  Windows 没有这条机制——`CreateProcess` 只认 PE，脚本路径直接启动恒失败
  （`%1 is not a valid Win32 application`）。
- 症状（B045，windows-latest CI run 42777671263）：`plugin_host_contract` 11 例中 7 例红，
  全是依赖夹具 spawn 的用例（生命周期 / 未声明工具 / 超时杀进程 / 版本协商 / 崩溃 fail-closed /
  env 白名单）；4 个只做 manifest 纯解析的用例通过。Linux 恒绿。
- 这不是测试独有需求：插件作者用 Python/Node 写插件是本协议最可能的第一形态，若 Windows 上
  恒不可加载，等于 ADR 0067 的扩展点在该平台不存在。

## 决策

`PluginTransport::spawn` 在**起进程前**把「脚本命令」解析成「解释器 + 脚本 + 调用方参数」，
解析入口是平台分支的 `resolve_script_command`：

- **Windows**：读文件首行（≤256 字节）判断 `#!`；`#!/usr/bin/env python3` 取 `env` 之后的
  解释器名（跳过 `-S` 之类 env 自身旗标），`#!/usr/bin/python3` 取文件名，解释器一律按 `PATH`
  解析；解释器自己的旗标保留，排在脚本路径之前，调用方 `args` 排在最后。
- **Unix**：原样启动（`resolve_script_command` 返回 `(command, args)`）——内核自己按 shebang
  起解释器，且 supervisor 的「执行位必须存在」（`mode & 0o111`）校验是既有契约的一部分，
  宿主不越权改写。
- 无 `#!` / 解析不出解释器 ⇒ 回落为直接启动，保留既有错误语义（不新增"猜测解释器"的隐式行为）。
- spawn 失败的错误文案在「脚本 ≠ 实际程序」时同时给出解释器
  （`describe_launch_target`），否则平台级报错无法定位到底启动了谁。
- 解析逻辑是**平台无关的纯函数**（`parse_shebang_launcher`），用值级单测冻结 `env` 形态 /
  绝对路径 / 反斜杠路径 / 前置旗标 / 非 shebang 回落；Windows 专属分支只负责读文件首行。
- 隔离面不变：env 仍走 `apply_child_env`（`env_clear()` + F8a 白名单，`PATH`/`SYSTEMROOT`
  已在名单内），cwd 仍是 workspace root，`process_group(0)` 仍只在 Unix 生效。

## 候选方案

- **改 API 加 `args` 参数（`spawn(manifest, program, args, config)`）**：能让测试自己传
  `python3` + 脚本路径，但把"脚本要谁来解释"的责任推给每个调用方，manifest 侧也得多一份
  `args` 字段；对真实插件作者仍是"Windows 上要自己写启动器"。否决（治标不治本）。
- **测试专用旁路（用例里生成 `.cmd`/`.bat` 包装）**：Windows `CreateProcess` 不执行批处理，
  必须再套一层 `cmd /C`；且只在测试里可用，产品侧脚本插件依旧不可加载。否决。
- **只在测试侧把夹具编译成小 exe**：需要测试期调用 rustc/链接器，构建时间与可移植性双输，
  且同样不解决产品面。否决。
- **文档化「Windows 插件必须是 PE」并跳过这 7 个用例**：以"平台不支持"换取绿灯，等于把
  扩展点的一半用户挡在门外；且跳过会让契约在 Windows 上永久失去可判定形态。否决。
- **按扩展名硬编码解释器（`.py` → python、`.js` → node）**：扩展名映射是第二真值源，
  且忽略 shebang 里显式声明的解释器与旗标。否决（shebang 已是脚本自带的声明）。

## 后果

- Windows 上脚本插件可加载，`plugin_host_contract` 的 7 个用例回到可判定形态；
  Unix 行为零变化（解析函数在该平台直接返回原值）。
- 解释器必须在 `PATH` 上（`PATH` 本就在 F8a 白名单内，子进程可见）；缺失时仍是明确的
  spawn 失败，错误文案带脚本与解释器两个路径。
- 超时/崩溃语义不变：Windows 仍只 kill 直接子进程（Job Object 见 ADR 0067 已知局限）。
- shebang 解析只看首行 256 字节，不改文件、不执行探测；失败一律回落直接启动，不引入新的
  fail-open 面。
