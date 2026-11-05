// capability_commands: capability / skill 列表、inspect 与源接入命令。
// 由 control_plane/mod.rs 的 HostControlPlane 巨型 impl 拆分而来，行为与结构保持一致。
use super::*;

impl HostControlPlane {
    pub fn list_capability_sources(&self) -> Vec<CapabilitySourceView> {
        self.capability_registry
            .read()
            .unwrap_or_else(|e| {
                eprintln!("[pony-agent] capability registry lock poisoned: {e}, recovering");
                e.into_inner()
            })
            .list_sources()
    }

    pub fn list_capabilities(&self, query: CapabilityListQuery) -> Vec<CapabilityView> {
        self.capability_registry
            .read()
            .unwrap_or_else(|e| {
                eprintln!("[pony-agent] capability registry lock poisoned: {e}, recovering");
                e.into_inner()
            })
            .list_capabilities(query.source_id.as_deref(), query.kind.as_deref())
    }

    pub fn inspect_capability(&self, query: CapabilityInspectionQuery) -> Option<CapabilityView> {
        self.capability_registry
            .read()
            .unwrap_or_else(|e| {
                eprintln!("[pony-agent] capability registry lock poisoned: {e}, recovering");
                e.into_inner()
            })
            .inspect_capability(&query.capability_id)
    }

    pub fn inspect_capability_source(
        &self,
        query: CapabilitySourceInspectionQuery,
    ) -> Option<CapabilitySourceView> {
        self.capability_registry
            .read()
            .unwrap_or_else(|e| {
                eprintln!("[pony-agent] capability registry lock poisoned: {e}, recovering");
                e.into_inner()
            })
            .inspect_source(&query.source_id)
    }

    pub fn inspect_skill_source(
        &self,
        query: SkillSourceInspectionQuery,
    ) -> Option<SkillSourceView> {
        self.capability_registry
            .read()
            .unwrap_or_else(|e| {
                eprintln!("[pony-agent] capability registry lock poisoned: {e}, recovering");
                e.into_inner()
            })
            .inspect_skill_source(&query.source_id)
    }

    pub fn list_skills(&self, query: SkillListQuery) -> Vec<SkillDescriptor> {
        self.capability_registry
            .read()
            .unwrap_or_else(|e| {
                eprintln!("[pony-agent] capability registry lock poisoned: {e}, recovering");
                e.into_inner()
            })
            .list_skills(query.source_id.as_deref())
    }

    pub fn inspect_skill(&self, query: SkillInspectionQuery) -> Option<SkillDescriptor> {
        self.capability_registry
            .read()
            .unwrap_or_else(|e| {
                eprintln!("[pony-agent] capability registry lock poisoned: {e}, recovering");
                e.into_inner()
            })
            .inspect_skill(&query.skill_id)
    }

    pub fn apply_mcp_source_snapshot(
        &self,
        command: ApplyMcpSourceSnapshotCommand,
    ) -> Result<CapabilitySourceView, String> {
        validate_mcp_source_snapshot(&command.snapshot)?;

        {
            let mut runtime = self.runtime.write().expect("runtime lock poisoned");
            runtime.dispatch_mcp_source_ingress_hooks(&command.snapshot)?;
            runtime.apply_mcp_source_snapshot(command.snapshot.clone());
        }

        Ok(command.snapshot.source)
    }

    // ── MCP 通道（ADR 0066 / T2） ────────────────────────────────────────────────────────
    //
    // 只经 host command 暴露：前端不直连 MCP（AGENTS.md「模型网络流量严禁直接走前端」）。
    // 配置坏只禁用该 server（fail-soft），注册失败只体现在 health 里，不阻断 app 启动。

    /// MCP 通道健康快照（进程级通道；未接入时为空）。
    pub fn mcp_health(&self) -> Vec<crate::agent::mcp_config::McpHealthView> {
        crate::agent::mcp_config::mcp_health()
            .into_iter()
            .map(crate::agent::mcp_config::McpHealthView::from)
            .collect()
    }

    /// 按 `.mcp.json`（项目层 + 全局层）加载并连接 MCP server，然后：
    /// 1) 把每个 server 注册进治理分派器（`register_tool_provider`，追加语义，不走整面换入）；
    /// 2) 安装为进程级通道（模型工具面在下一轮生效）。
    pub fn load_mcp_sources(
        &self,
        workspace_root: Option<String>,
    ) -> crate::agent::mcp_config::McpSourceLoadSummary {
        let root = workspace_root
            .map(std::path::PathBuf::from)
            .or_else(crate::agent::workspace::compute_default_workspace_root)
            .unwrap_or_else(|| std::path::PathBuf::from("."));
        let (load, mut channel) = crate::agent::mcp_config::load_and_connect(
            &root,
            crate::agent::mcp_client::DEFAULT_TIMEOUT,
        );
        // 注册进共享治理分派器：外部 descriptor 与 handler 同批发布（T1 的原子单元）。
        let health = channel.register_tool_providers(self.ask_dispatcher.as_ref());
        crate::agent::mcp_config::install_mcp_tool_channel(std::sync::Arc::new(channel));

        crate::agent::mcp_config::McpSourceLoadSummary {
            workspace_root: root.display().to_string(),
            servers: load
                .servers
                .iter()
                .map(|entry| crate::agent::mcp_config::McpServerConfigView {
                    server_id: entry.id.clone(),
                    source: entry.source.as_str().to_string(),
                    command: entry.spec.command.clone(),
                    arg_count: entry.spec.args.len(),
                    env_keys: entry.spec.env.iter().map(|(key, _)| key.clone()).collect(),
                    requires_approval: entry.requires_approval,
                })
                .collect(),
            diagnostics: load
                .diagnostics
                .iter()
                .map(|diagnostic| crate::agent::mcp_config::McpConfigDiagnosticView {
                    server: diagnostic.server.clone(),
                    code: diagnostic.code.clone(),
                    message: diagnostic.message.clone(),
                })
                .collect(),
            health: health.into_iter().map(crate::agent::mcp_config::McpHealthView::from).collect(),
        }
    }

    pub fn apply_skill_source_snapshot(
        &self,
        command: ApplySkillSourceSnapshotCommand,
    ) -> Result<SkillSourceView, String> {
        validate_skill_source_snapshot(&command.snapshot)?;

        let normalized_snapshot = {
            let registry = self.capability_registry.read().unwrap_or_else(|e| {
                eprintln!("[pony-agent] capability registry lock poisoned: {e}, recovering");
                e.into_inner()
            });
            normalize_skill_source_snapshot_against_capabilities(
                &registry,
                command.snapshot.clone(),
            )?
        };

        {
            let mut runtime = self.runtime.write().expect("runtime lock poisoned");
            runtime.dispatch_skill_source_ingress_hooks(&normalized_snapshot)?;
            runtime.apply_skill_source_snapshot(normalized_snapshot.clone())?;
        }

        Ok(normalized_snapshot.source)
    }
}
