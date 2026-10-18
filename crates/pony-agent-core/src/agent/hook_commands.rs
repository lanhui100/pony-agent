//! 外部命令 Hooks 执行器（W30a）。
//!
//! 背景：审计（`docs/audit/agent-core-gap-matrix.md` P1-5 / A07）指出钩子体系「点位与注册表
//! 齐备，但执行器只有 `NoopHookExecutor`」——所有钩子点都在空转，用户无法用 shell 脚本在
//! 工具执行前后做校验/拦截/注入，对比基线（claude-code `hooks/` 命令钩子、DSH
//! `hooks-claude-code`）完全缺失可编程扩展面。
//!
//! 本模块提供最小可用的命令钩子执行器：
//! - 配置：`{"hooks": {"<hook 名>": {"command": "sh 命令", "timeoutMs": 5000}}}`；
//! - 执行：`sh -c <command>`（Unix）/ `cmd /C <command>`（Windows），JSON payload 走 stdin；
//! - 决策：stdout 返回 `{"decision":"allow|deny|observe","summary"|"reason":...}`；
//! - 超时：到点 kill 并返回错误（绝不悬挂回合）；
//! - 未配置的 hook 名：退化为 Noop 语义（与既有行为一致）。
//!
//! 接线：`build_default_hook_executor()` 在 `PONY_HOOKS_FILE` 或 `<cwd>/.pony/hooks.json`
//! 存在时加载配置，否则返回空执行器（Noop 等价）。

use std::collections::BTreeMap;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde::Deserialize;
use serde_json::{json, Value};

use crate::agent::hooks::{
    AgentHookDescriptor, AgentHookExecutor, HookClass,
    HookFailurePolicy, HookRecoveryMode, HookReplayRequirements,
    HookSideEffectPersistenceRequirements, HookTraceRequirements, NoopHookExecutor, TurnHookPoint,
};
use crate::agent::hooks::{HookDenyDecision, HookExecutionResult, HookResultKind, HookStructuredResult};

/// 默认命令钩子超时（毫秒）。
pub const DEFAULT_HOOK_TIMEOUT_MS: u64 = 5_000;

/// 单个命令钩子的配置。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandHookSpec {
    /// shell 命令（Unix 用 `sh -c`，Windows 用 `cmd /C`）。
    pub command: String,
    /// 超时毫秒（0/缺省 → [`DEFAULT_HOOK_TIMEOUT_MS`]）。
    pub timeout_ms: u64,
}

impl CommandHookSpec {
    pub fn new(command: impl Into<String>) -> Self {
        Self {
            command: command.into(),
            timeout_ms: DEFAULT_HOOK_TIMEOUT_MS,
        }
    }

    pub fn with_timeout_ms(mut self, timeout_ms: u64) -> Self {
        self.timeout_ms = timeout_ms;
        self
    }

    fn effective_timeout(&self) -> Duration {
        let ms = if self.timeout_ms == 0 {
            DEFAULT_HOOK_TIMEOUT_MS
        } else {
            self.timeout_ms
        };
        Duration::from_millis(ms)
    }
}

/// 按 hook 名索引的命令钩子执行器。
pub struct CommandHookExecutor {
    specs: BTreeMap<String, CommandHookSpec>,
    fallback: NoopHookExecutor,
}

impl CommandHookExecutor {
    pub fn new(specs: BTreeMap<String, CommandHookSpec>) -> Self {
        Self {
            specs,
            fallback: NoopHookExecutor,
        }
    }

    /// 从 JSON 文本加载：`{"hooks":{"<name>":{"command":"...","timeoutMs":2000}}}`。
    pub fn from_json_str(json_text: &str) -> Result<Self, String> {
        #[derive(Deserialize)]
        struct RawSpec {
            command: String,
            #[serde(default, rename = "timeoutMs")]
            timeout_ms: Option<u64>,
        }
        #[derive(Deserialize)]
        struct RawConfig {
            #[serde(default)]
            hooks: BTreeMap<String, RawSpec>,
        }

        let config: RawConfig = serde_json::from_str(json_text)
            .map_err(|err| format!("hooks 配置不是合法 JSON：{err}"))?;

        let mut specs = BTreeMap::new();
        for (name, raw) in config.hooks {
            if name.trim().is_empty() {
                return Err("hooks 配置中存在空 hook 名。".to_string());
            }
            if raw.command.trim().is_empty() {
                return Err(format!("hook `{name}` 的 command 不能为空。"));
            }
            specs.insert(
                name,
                CommandHookSpec {
                    command: raw.command,
                    timeout_ms: raw.timeout_ms.unwrap_or(DEFAULT_HOOK_TIMEOUT_MS),
                },
            );
        }
        Ok(Self::new(specs))
    }

    /// 从文件加载配置。
    pub fn from_file(path: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path)
            .map_err(|err| format!("读取 hooks 配置 {} 失败：{err}", path.display()))?;
        Self::from_json_str(&text)
    }

    pub fn is_empty(&self) -> bool {
        self.specs.is_empty()
    }

    /// 已配置的 hook 名（自查/诊断用）。
    pub fn hook_names(&self) -> Vec<&str> {
        self.specs.keys().map(String::as_str).collect()
    }

    fn run_command(
        &self,
        descriptor: &AgentHookDescriptor,
        hook_point: TurnHookPoint,
        spec: &CommandHookSpec,
    ) -> Result<HookExecutionResult, String> {
        let started = Instant::now();
        let payload = json!({
            "hookName": descriptor.name,
            "descriptorName": descriptor.name,
            "hookPoint": hook_point_label(&hook_point),
            "hookClass": format!("{:?}", descriptor.class),
        })
        .to_string();

        #[cfg(windows)]
        let mut command = {
            let mut cmd = Command::new("cmd");
            cmd.arg("/C").arg(&spec.command);
            cmd
        };
        #[cfg(not(windows))]
        let mut command = {
            let mut cmd = Command::new("sh");
            cmd.arg("-c").arg(&spec.command);
            cmd
        };

        // Unix：独立进程组，超时时可连同 shell 派生的孙进程一起回收（对齐 MCP 客户端做法）。
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }

        if let Some(root) = std::env::var_os("PONY_WORKSPACE_ROOT") {
            command.current_dir(root);
        }

        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|err| format!("启动 hook `{}` 命令失败：{err}", descriptor.name))?;
        let child_pid = child.id();

        if let Some(mut stdin) = child.stdin.take() {
            // EPIPE（脚本不读 stdin 就退出）不是致命错误，忽略即可；必须在 drop 前写完。
            let _ = stdin.write_all(payload.as_bytes());
        }

        // 先取走 stdout/stderr 并起后台读线程：绝不能只 try_wait 而不 drain，
        // 否则输出超过管道缓冲（约 64KiB，如 clippy/大文件 cat）时子进程会写满阻塞，
        // 被误判为超时并 kill。
        let stdout_pipe = child.stdout.take();
        let stderr_pipe = child.stderr.take();
        let stdout_reader = std::thread::spawn(move || read_all(stdout_pipe));
        let stderr_reader = std::thread::spawn(move || read_all(stderr_pipe));

        let timeout = spec.effective_timeout();
        let deadline = Instant::now() + timeout;
        loop {
            match child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) => {
                    if Instant::now() >= deadline {
                        kill_process_tree(&mut child, child_pid);
                        // kill 之后管道关闭，读线程会自然结束；仍取回已读内容用于诊断。
                        let _ = stdout_reader.join();
                        let _ = stderr_reader.join();
                        return Err(format!(
                            "hook `{}` 执行超时（{timeout:?}），已终止子进程（含子进程组）。",
                            descriptor.name
                        ));
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(err) => {
                    kill_process_tree(&mut child, child_pid);
                    let _ = stdout_reader.join();
                    let _ = stderr_reader.join();
                    return Err(format!("hook `{}` 等待子进程失败：{err}", descriptor.name));
                }
            }
        }

        // 进程已退出：等读线程收尾（EOF 后自然结束），再取回 status。
        let stdout_bytes = stdout_reader.join().unwrap_or_default();
        let stderr_bytes = stderr_reader.join().unwrap_or_default();
        let status = child
            .wait()
            .map_err(|err| format!("等待 hook `{}` 进程失败：{err}", descriptor.name))?;
        let output = std::process::Output {
            status,
            stdout: stdout_bytes,
            stderr: stderr_bytes,
        };
        let elapsed_ms = started.elapsed().as_millis() as u64;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!(
                "hook `{}` 退出码非零（{:?}）：{}",
                descriptor.name,
                output.status.code(),
                stderr.trim()
            ));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let decision: Value = serde_json::from_str(stdout.trim()).map_err(|err| {
            format!(
                "hook `{}` 输出不是合法 JSON（{err}）：{}",
                descriptor.name,
                stdout.trim()
            )
        })?;

        let kind = decision
            .get("decision")
            .and_then(Value::as_str)
            .map(|raw| raw.trim().to_ascii_lowercase())
            .ok_or_else(|| format!("hook `{}` 输出缺少 decision 字段。", descriptor.name))?;
        let kind = kind.as_str();

        let (result_kind, structured_result, blocked, trace_summary) = match kind {
            "allow" => {
                let summary = decision
                    .get("summary")
                    .and_then(Value::as_str)
                    .unwrap_or("allow")
                    .to_string();
                (
                    HookResultKind::Allow,
                    HookStructuredResult::Allow {
                        summary: summary.clone(),
                    },
                    false,
                    format!("command hook `{}` allowed: {summary}", descriptor.name),
                )
            }
            "deny" => {
                let reason = decision
                    .get("reason")
                    .and_then(Value::as_str)
                    .unwrap_or("denied by command hook")
                    .to_string();
                (
                    HookResultKind::Deny,
                    HookStructuredResult::Deny(HookDenyDecision {
                        reason_code: format!("command_hook_{}", descriptor.name),
                        message: reason.clone(),
                    }),
                    true,
                    format!("command hook `{}` denied: {reason}", descriptor.name),
                )
            }
            "observe" => {
                let summary = decision
                    .get("summary")
                    .and_then(Value::as_str)
                    .unwrap_or("observe")
                    .to_string();
                (
                    HookResultKind::Observe,
                    HookStructuredResult::Observe {
                        summary: summary.clone(),
                    },
                    false,
                    format!("command hook `{}` observed: {summary}", descriptor.name),
                )
            }
            other => {
                return Err(format!(
                    "hook `{}` 返回未知 decision `{other}`（支持 allow/deny/observe）。",
                    descriptor.name
                ))
            }
        };

        if !descriptor.allowed_result_kinds.contains(&result_kind) {
            return Err(format!(
                "hook `{}` 不允许 result kind `{result_kind:?}`。",
                descriptor.name
            ));
        }

        Ok(HookExecutionResult {
            hook_name: descriptor.name.clone(),
            hook_class: descriptor.class.clone(),
            hook_point,
            hook_order: 0,
            result_kind,
            structured_result,
            blocked,
            elapsed_ms,
            input_summary: Some(format!("command hook stdin payload for `{}`", descriptor.name)),
            persistence_evidence_ref: None,
            trace_summary,
        })
    }
}

impl AgentHookExecutor for CommandHookExecutor {
    fn execute(
        &self,
        descriptor: &AgentHookDescriptor,
        hook_point: TurnHookPoint,
    ) -> Result<HookExecutionResult, String> {
        match self.specs.get(&descriptor.name) {
            Some(spec) => self.run_command(descriptor, hook_point, spec),
            // 未配置的 hook：保持既有 Noop 语义（含 hook_point / result kind 校验）。
            None => self.fallback.execute(descriptor, hook_point),
        }
    }
}

/// 读完一个管道（用于后台 drain 线程）。
fn read_all(pipe: Option<impl std::io::Read>) -> Vec<u8> {
    let mut buffer = Vec::new();
    if let Some(mut pipe) = pipe {
        let _ = pipe.read_to_end(&mut buffer);
    }
    buffer
}

/// 终止 hook 子进程树：Unix 对进程组发 SIGKILL（覆盖 shell 派生的孙进程），其它平台 kill 子进程。
fn kill_process_tree(child: &mut std::process::Child, pid: u32) {
    #[cfg(unix)]
    {
        // safety: 子进程以 process_group(0) 启动，pgid == 子进程 pid。
        unsafe {
            libc::kill(-(pid as i32), libc::SIGKILL);
        }
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
    }
    let _ = child.kill();
    let _ = child.wait();
}

/// hook point 的稳定文本标签（写入 payload 供脚本分支）。
pub fn hook_point_label(hook_point: &TurnHookPoint) -> String {
    serde_json::to_value(hook_point)
        .ok()
        .and_then(|value| value.as_str().map(str::to_string))
        .unwrap_or_else(|| format!("{hook_point:?}"))
}

/// 默认 hooks 配置路径：`PONY_HOOKS_FILE` 优先；否则 `<PONY_WORKSPACE_ROOT 或 cwd>/.pony/hooks.json`。
pub fn default_hooks_config_path() -> std::path::PathBuf {
    if let Some(explicit) = std::env::var_os("PONY_HOOKS_FILE") {
        return std::path::PathBuf::from(explicit);
    }
    workspace_hooks_path()
}

fn workspace_hooks_path() -> std::path::PathBuf {
    let root = std::env::var_os("PONY_WORKSPACE_ROOT")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from(".")));
    root.join(".pony").join("hooks.json")
}

/// 是否允许加载「工作区内的」hooks 配置。
///
/// 安全边界：仓库内的 `.pony/hooks.json` 是 **可被仓库作者控制的 shell 命令**，默认不启用
/// （否则 clone/打开一个恶意仓库即 RCE）。只有当调用方显式设置 `PONY_HOOKS_ENABLE=1`
/// 或显式指定 `PONY_HOOKS_FILE` 时才加载。
fn workspace_hooks_enabled() -> bool {
    matches!(
        std::env::var("PONY_HOOKS_ENABLE").ok().as_deref(),
        Some("1") | Some("true") | Some("TRUE") | Some("yes")
    )
}

/// 解析 hook point 文本（支持 snake_case 与 PascalCase）。
fn parse_hook_point(value: &str) -> Result<TurnHookPoint, String> {
    let normalized: String = value
        .chars()
        .filter(|ch| *ch != '_' && *ch != '-')
        .collect::<String>()
        .to_ascii_lowercase();
    const ALL: [TurnHookPoint; 19] = [
        TurnHookPoint::TurnPrepareStart,
        TurnHookPoint::TurnPrepareEnd,
        TurnHookPoint::ContextBuildStart,
        TurnHookPoint::ContextBuildEnd,
        TurnHookPoint::ModelCallStart,
        TurnHookPoint::ModelResponseEnd,
        TurnHookPoint::ToolCallStart,
        TurnHookPoint::ToolCallEnd,
        TurnHookPoint::CheckpointPersistStart,
        TurnHookPoint::CheckpointPersistEnd,
        TurnHookPoint::TurnFinalizeStart,
        TurnHookPoint::TurnFinalizeEnd,
        TurnHookPoint::PlannerTurnPreflight,
        TurnHookPoint::PlannerToolSelection,
        TurnHookPoint::PlannerGraphDecision,
        TurnHookPoint::CapabilityResolve,
        TurnHookPoint::SkillToolActionsResolve,
        TurnHookPoint::CapabilitySourceIngress,
        TurnHookPoint::SkillSourceIngress,
    ];
    ALL.into_iter()
        .find(|point| {
            let label = format!("{point:?}").to_ascii_lowercase();
            label == normalized
        })
        .ok_or_else(|| format!("未知 hook point `{value}`。"))
}

/// 从配置文本构建（注册表 + 执行器）。
///
/// 这是 A07 闭环的关键：执行器必须与**注册表描述符**同时构建，否则钩子点永远不会触发
/// （有执行器无描述符 = 空转）。返回的 registry 已注册配置里的每个 hook。
pub fn build_hooks_from_json_str(
    json_text: &str,
) -> Result<(crate::agent::hooks::AgentHookRegistry, CommandHookExecutor), String> {
    #[derive(Deserialize)]
    struct RawSpec {
        command: String,
        #[serde(default, rename = "timeoutMs")]
        timeout_ms: Option<u64>,
        #[serde(default, rename = "hookPoints")]
        hook_points: Vec<String>,
        #[serde(default, rename = "onFailure")]
        on_failure: Option<String>,
    }
    #[derive(Deserialize)]
    struct RawConfig {
        #[serde(default)]
        hooks: BTreeMap<String, RawSpec>,
    }

    let config: RawConfig = serde_json::from_str(json_text)
        .map_err(|err| format!("hooks 配置不是合法 JSON：{err}"))?;

    let mut registry = crate::agent::hooks::AgentHookRegistry::new();
    let mut specs = BTreeMap::new();

    for (name, raw) in config.hooks {
        if name.trim().is_empty() {
            return Err("hooks 配置中存在空 hook 名。".to_string());
        }
        if raw.command.trim().is_empty() {
            return Err(format!("hook `{name}` 的 command 不能为空。"));
        }

        let mut hook_points = Vec::new();
        for label in &raw.hook_points {
            let point = parse_hook_point(label)?;
            if !hook_points.contains(&point) {
                hook_points.push(point);
            }
        }
        if hook_points.is_empty() {
            hook_points = vec![TurnHookPoint::ToolCallStart, TurnHookPoint::ToolCallEnd];
        }

        let failure_policy = match raw.on_failure.as_deref() {
            None | Some("fail_turn") | Some("failTurn") => HookFailurePolicy::FailTurn,
            Some("ignore") => HookFailurePolicy::Ignore,
            Some("degrade") => HookFailurePolicy::Degrade,
            Some(other) => return Err(format!("hook `{name}` 的 onFailure=`{other}` 不受支持。")),
        };

        // Guard 类只允许 Allow/Deny（Observe 属 Observe 类，注册校验会拒绝）。
        let descriptor = AgentHookDescriptor {
            contract_version: "v1".to_string(),
            name: name.clone(),
            class: HookClass::Guard,
            priority: 0,
            timeout_ms: raw.timeout_ms.unwrap_or(DEFAULT_HOOK_TIMEOUT_MS),
            allowed_hook_points: hook_points,
            allowed_result_kinds: vec![HookResultKind::Allow, HookResultKind::Deny],
            can_block: true,
            default_failure_policy: failure_policy.clone(),
            allowed_failure_policies: vec![failure_policy],
            default_recovery_mode: HookRecoveryMode::ReplayRequired,
            trace_requirements: HookTraceRequirements {
                include_name: true,
                include_hook_point: true,
                include_elapsed_ms: true,
                include_result_summary: true,
            },
            replay_requirements: HookReplayRequirements {
                include_hook_order: true,
                include_input_summary: true,
            },
            side_effect_persistence_requirements: HookSideEffectPersistenceRequirements {
                require_persistence_evidence: false,
                require_effect_summary: false,
            },
        };
        registry
            .register(descriptor)
            .map_err(|err| format!("注册 hook `{name}` 失败：{err}"))?;
        specs.insert(
            name,
            CommandHookSpec {
                command: raw.command,
                timeout_ms: raw.timeout_ms.unwrap_or(DEFAULT_HOOK_TIMEOUT_MS),
            },
        );
    }

    Ok((registry, CommandHookExecutor::new(specs)))
}

/// 默认（注册表 + 执行器）：配置存在且被允许加载时启用命令钩子，否则空注册表 + Noop。
///
/// 触发条件：`PONY_HOOKS_FILE` 显式指定，或 `PONY_HOOKS_ENABLE=1` 且工作区存在 `.pony/hooks.json`。
pub fn build_default_hooks() -> (
    crate::agent::hooks::AgentHookRegistry,
    Box<dyn AgentHookExecutor>,
) {
    let explicit = std::env::var_os("PONY_HOOKS_FILE").is_some();
    if !explicit && !workspace_hooks_enabled() {
        return (
            crate::agent::hooks::AgentHookRegistry::new(),
            Box::new(NoopHookExecutor),
        );
    }

    let candidate = default_hooks_config_path();
    let text = match std::fs::read_to_string(&candidate) {
        Ok(text) => text,
        Err(err) => {
            if explicit {
                eprintln!(
                    "[pony-agent][hooks] 无法读取 PONY_HOOKS_FILE={}：{err}，钩子未启用。",
                    candidate.display()
                );
            }
            return (
                crate::agent::hooks::AgentHookRegistry::new(),
                Box::new(NoopHookExecutor),
            );
        }
    };

    match build_hooks_from_json_str(&text) {
        Ok((registry, executor)) if !executor.is_empty() => (registry, Box::new(executor)),
        Ok((registry, _)) => (registry, Box::new(NoopHookExecutor)),
        Err(err) => {
            // 不能再静默降级：配置写错却毫无提示等于"配了也不生效"。
            eprintln!(
                "[pony-agent][hooks] 配置 {} 解析/注册失败：{err}，本次会话钩子未启用。",
                candidate.display()
            );
            (
                crate::agent::hooks::AgentHookRegistry::new(),
                Box::new(NoopHookExecutor),
            )
        }
    }
}

/// 仅构建执行器（保留旧入口；建议改用 [`build_default_hooks`] 以同时得到注册表）。
pub fn build_default_hook_executor() -> Box<dyn AgentHookExecutor> {
    build_default_hooks().1
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::hooks::{
        HookClass, HookFailurePolicy, HookRecoveryMode, HookReplayRequirements,
        HookSideEffectPersistenceRequirements, HookTraceRequirements,
    };

    fn descriptor(name: &str, hook_point: TurnHookPoint) -> AgentHookDescriptor {
        AgentHookDescriptor {
            contract_version: "v1".to_string(),
            name: name.to_string(),
            class: HookClass::Guard,
            priority: 0,
            timeout_ms: 5_000,
            allowed_hook_points: vec![hook_point],
            // Guard 类只允许 Allow/Deny（Observe 属 Observe 类，注册校验会拒绝）。
            allowed_result_kinds: vec![HookResultKind::Allow, HookResultKind::Deny],
            can_block: true,
            default_failure_policy: HookFailurePolicy::FailTurn,
            allowed_failure_policies: vec![HookFailurePolicy::FailTurn],
            default_recovery_mode: HookRecoveryMode::ReplayRequired,
            trace_requirements: HookTraceRequirements {
                include_name: true,
                include_hook_point: true,
                include_elapsed_ms: true,
                include_result_summary: true,
            },
            replay_requirements: HookReplayRequirements {
                include_hook_order: true,
                include_input_summary: true,
            },
            side_effect_persistence_requirements: HookSideEffectPersistenceRequirements {
                require_persistence_evidence: false,
                require_effect_summary: false,
            },
        }
    }

    fn write_hook_script(dir: &Path, name: &str, unix_body: &str, win_body: &str) -> std::path::PathBuf {
        #[cfg(windows)]
        let (script_name, script_body) = {
            let cmd_name = format!("{}.cmd", name);
            let cmd_body = format!("@echo off\r\nchcp 65001 >nul 2>&1\r\n{}", win_body);
            (cmd_name, cmd_body)
        };
        #[cfg(not(windows))]
        let (script_name, script_body) = {
            let _ = win_body;
            (format!("{}.sh", name), unix_body.to_string())
        };

        let path = dir.join(script_name);
        std::fs::write(&path, script_body).expect("write script");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&path).expect("meta").permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&path, perms).expect("chmod");
        }
        path
    }

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "pony-hook-unit-{}-{}-{}",
            tag,
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).expect("create dir");
        dir
    }

    #[test]
    fn default_hooks_build_registers_descriptors_and_executor() {
        let config = serde_json::json!({
            "hooks": {
                "guard": {
                    "command": "echo '{\"decision\":\"allow\"}'",
                    "hookPoints": ["tool_call_start", "ToolCallEnd"]
                }
            }
        })
        .to_string();
        let (registry, executor) = build_hooks_from_json_str(&config).expect("build hooks");
        assert_eq!(executor.hook_names(), vec!["guard"]);
        let descriptors = registry.list_for_hook_point(&TurnHookPoint::ToolCallStart);
        assert_eq!(
            descriptors.len(),
            1,
            "配置的钩子必须注册进 registry，否则永远不触发"
        );
        assert_eq!(descriptors[0].name, "guard");
        assert_eq!(
            registry.list_for_hook_point(&TurnHookPoint::ToolCallEnd).len(),
            1
        );
    }

    #[test]
    fn parses_config_and_rejects_empty_command() {
        let executor = CommandHookExecutor::from_json_str(
            r#"{"hooks":{"guard":{"command":"echo hi","timeoutMs":1234}}}"#,
        )
        .expect("parse");
        assert_eq!(executor.hook_names(), vec!["guard"]);
        assert!(CommandHookExecutor::from_json_str(r#"{"hooks":{"bad":{"command":"  "}}}"#).is_err());
    }

    #[test]
    fn allow_decision_is_returned() {
        let dir = temp_dir("allow");
        let script = write_hook_script(
            &dir,
            "allow",
            "#!/bin/sh\ncat >/dev/null\necho '{\"decision\":\"allow\",\"summary\":\"ok\"}'\n",
            "echo {\"decision\":\"allow\",\"summary\":\"ok\"}\r\n",
        );
        let mut specs = BTreeMap::new();
        specs.insert(
            "guard".to_string(),
            CommandHookSpec::new(script.to_string_lossy().to_string()),
        );
        let executor = CommandHookExecutor::new(specs);
        let result = executor
            .execute(&descriptor("guard", TurnHookPoint::ToolCallStart), TurnHookPoint::ToolCallStart)
            .expect("execute");
        assert_eq!(result.result_kind, HookResultKind::Allow);
        assert!(!result.blocked);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn deny_decision_blocks() {
        let dir = temp_dir("deny");
        let script = write_hook_script(
            &dir,
            "deny",
            "#!/bin/sh\ncat >/dev/null\necho '{\"decision\":\"deny\",\"reason\":\"blocked by policy\"}'\n",
            "echo {\"decision\":\"deny\",\"reason\":\"blocked by policy\"}\r\n",
        );
        let mut specs = BTreeMap::new();
        specs.insert("guard".to_string(), CommandHookSpec::new(script.to_string_lossy().to_string()));
        let executor = CommandHookExecutor::new(specs);
        let result = executor
            .execute(&descriptor("guard", TurnHookPoint::ToolCallStart), TurnHookPoint::ToolCallStart)
            .expect("execute");
        assert_eq!(result.result_kind, HookResultKind::Deny);
        assert!(result.blocked);
        assert!(matches!(result.structured_result, HookStructuredResult::Deny(_)));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn timeout_kills_child_and_errors() {
        let dir = temp_dir("timeout");
        let script = write_hook_script(
            &dir,
            "slow",
            "#!/bin/sh\nsleep 5\n",
            "ping 127.0.0.1 -n 6 >nul\r\n",
        );
        let mut specs = BTreeMap::new();
        specs.insert(
            "guard".to_string(),
            CommandHookSpec::new(script.to_string_lossy().to_string()).with_timeout_ms(200),
        );
        let executor = CommandHookExecutor::new(specs);
        let started = Instant::now();
        let err = executor
            .execute(&descriptor("guard", TurnHookPoint::ToolCallStart), TurnHookPoint::ToolCallStart)
            .expect_err("timeout must error");
        assert!(err.contains("超时"), "{err}");
        assert!(started.elapsed() < Duration::from_secs(2));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn invalid_output_errors() {
        let dir = temp_dir("invalid");
        let script = write_hook_script(
            &dir,
            "bad",
            "#!/bin/sh\ncat >/dev/null\necho 'not json'\n",
            "echo not json\r\n",
        );
        let mut specs = BTreeMap::new();
        specs.insert("guard".to_string(), CommandHookSpec::new(script.to_string_lossy().to_string()));
        let executor = CommandHookExecutor::new(specs);
        let err = executor
            .execute(&descriptor("guard", TurnHookPoint::ToolCallStart), TurnHookPoint::ToolCallStart)
            .expect_err("invalid json must error");
        assert!(err.contains("JSON"), "{err}");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn unconfigured_hook_falls_back_to_noop_allow() {
        let executor = CommandHookExecutor::new(BTreeMap::new());
        let result = executor
            .execute(
                &descriptor("unknown", TurnHookPoint::ToolCallStart),
                TurnHookPoint::ToolCallStart,
            )
            .expect("noop fallback");
        assert!(!result.blocked);
    }

    #[test]
    fn payload_carries_hook_name_and_point() {
        let dir = temp_dir("payload");
        let capture = dir.join("payload.json");
        let script = write_hook_script(
            &dir,
            "capture",
            &format!("#!/bin/sh\ncat > '{}'\necho '{{\"decision\":\"allow\"}}'\n", capture.display()),
            &format!("more > \"{}\"\r\necho {{\"decision\":\"allow\"}}\r\n", capture.display()),
        );
        let mut specs = BTreeMap::new();
        specs.insert("guard".to_string(), CommandHookSpec::new(script.to_string_lossy().to_string()));
        let executor = CommandHookExecutor::new(specs);
        executor
            .execute(&descriptor("guard", TurnHookPoint::ToolCallStart), TurnHookPoint::ToolCallStart)
            .expect("execute");
        let payload = std::fs::read_to_string(&capture).expect("payload file");
        let value: Value = serde_json::from_str(&payload).expect("payload json");
        assert_eq!(value.get("hookName").and_then(Value::as_str), Some("guard"));
        assert!(value.get("hookPoint").and_then(Value::as_str).is_some());
        let _ = std::fs::remove_dir_all(dir);
    }
}
