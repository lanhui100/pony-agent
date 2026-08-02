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

        let mut registry = self.capability_registry.write().unwrap_or_else(|e| {
            eprintln!("[pony-agent] capability registry lock poisoned: {e}, recovering");
            e.into_inner()
        });
        registry.replace_mcp_source_snapshot(command.snapshot.clone());

        Ok(command.snapshot.source)
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

        // Re-acquire as write lock and re-normalize: the registry may have changed
        // between the read-lock normalization above and this write-lock application,
        // so we re-normalize under the write lock to maintain consistency.
        let mut registry = self.capability_registry.write().unwrap_or_else(|e| {
            eprintln!("[pony-agent] capability registry lock poisoned: {e}, recovering");
            e.into_inner()
        });
        let normalized_snapshot =
            normalize_skill_source_snapshot_against_capabilities(&registry, command.snapshot)?;
        registry.replace_skill_source_snapshot(normalized_snapshot.clone())?;

        Ok(normalized_snapshot.source)
    }
}
