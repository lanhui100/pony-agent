//! Deferred ToolSearch elevation (phase-7 task 7.3).
//!
//! ToolSearch searches a [`ToolRegistrySnapshot`] for `Deferred` descriptors and returns stable
//! candidates. A candidate can only be elevated for the **current** turn: the descriptor must
//! still exist in the current snapshot, still be `Deferred`, and carry the same `source_revision`
//! as the candidate that was surfaced to the model. Elevation mutates the [`TurnToolView`] via
//! `elevate_from_registry` so the full schema appears in the next provider hop, and every
//! successful elevation is recorded with the candidate id, the snapshot id, the source revision,
//! and a mutation reason for trace evidence.

use crate::agent::tool_runtime::TurnToolView;
use crate::agent::tools::{ToolDescriptor, ToolExposure, ToolRegistrySnapshot};
use serde::{Deserialize, Serialize};

/// A stable candidate surfaced by ToolSearch. `source_revision` is the version of the snapshot
/// the candidate came from and gates elevation: a candidate whose revision no longer matches the
/// current snapshot is stale and must be rejected.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CandidateTool {
    pub descriptor_id: String,
    pub model_name: String,
    pub summary: String,
    pub source_revision: String,
}

/// Trace evidence for one governed deferred elevation.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ElevationRecord {
    pub descriptor_id: String,
    pub snapshot_id: String,
    pub source_revision: String,
    pub mutation_reason: String,
}

/// Turn-scoped ToolSearch elevator. Holds the registry snapshot and turn tool view it was built
/// from; `elevate` always re-validates against the *current* snapshot and view passed in, so a
/// later source replacement invalidates stale candidates even after `search` was already called.
#[derive(Clone, Debug)]
pub struct ToolSearchElevator {
    registry: ToolRegistrySnapshot,
    view: TurnToolView,
    records: Vec<ElevationRecord>,
}

impl ToolSearchElevator {
    pub fn new(registry: ToolRegistrySnapshot, view: TurnToolView) -> Self {
        Self {
            registry,
            view,
            records: Vec::new(),
        }
    }

    pub fn registry(&self) -> &ToolRegistrySnapshot {
        &self.registry
    }

    pub fn view(&self) -> &TurnToolView {
        &self.view
    }

    pub fn snapshot_id(&self) -> &str {
        &self.registry.snapshot_id
    }

    /// Trace evidence collected so far for this turn.
    pub fn records(&self) -> &[ElevationRecord] {
        &self.records
    }

    /// Search `Deferred` descriptors in the elevator's snapshot and return stable candidates
    /// (descriptor id + model name + summary + source revision). An empty query returns all
    /// deferred candidates in stable order; a non-empty query ranks name matches first.
    pub fn search(&self, query: &str) -> Vec<CandidateTool> {
        let normalized = query.trim().to_lowercase();
        let mut candidates = self
            .registry
            .descriptors
            .iter()
            .filter(|descriptor| descriptor.exposure == ToolExposure::Deferred)
            .filter(|descriptor| normalized.is_empty() || self.matches_query(descriptor, &normalized))
            .map(|descriptor| CandidateTool {
                descriptor_id: descriptor.identity.descriptor_id.clone(),
                model_name: descriptor.identity.model_name.clone(),
                summary: descriptor.description.clone(),
                source_revision: descriptor.source_revision.clone(),
            })
            .collect::<Vec<_>>();
        candidates.sort_by(|left, right| {
            let left_rank = self.rank(left, &normalized);
            let right_rank = self.rank(right, &normalized);
            right_rank
                .cmp(&left_rank)
                .then_with(|| left.descriptor_id.cmp(&right.descriptor_id))
        });
        candidates
    }

    /// Elevate a candidate against the *current* snapshot and turn view.
    ///
    /// Fails closed when the descriptor is missing from the current snapshot (source-revision
    /// invalidation), is no longer `Deferred`, or carries a `source_revision` different from the
    /// candidate that was surfaced to the model. On success the view is mutated via
    /// `elevate_from_registry` and an [`ElevationRecord`] is appended for trace evidence.
    pub fn elevate(
        &mut self,
        snapshot: &ToolRegistrySnapshot,
        view: &mut TurnToolView,
        descriptor_id: &str,
    ) -> Result<ElevationRecord, String> {
        if descriptor_id.trim().is_empty() {
            return Err("tool descriptor id cannot be empty".to_string());
        }
        let descriptor = snapshot
            .descriptors
            .iter()
            .find(|descriptor| descriptor.identity.descriptor_id == descriptor_id)
            .ok_or_else(|| {
                format!(
                    "tool descriptor `{descriptor_id}` is not present in the current registry snapshot `{}`",
                    snapshot.snapshot_id
                )
            })?;
        if descriptor.exposure != ToolExposure::Deferred {
            return Err(format!(
                "tool descriptor `{descriptor_id}` is not deferred and cannot be elevated"
            ));
        }
        let candidate = self
            .search("")
            .into_iter()
            .find(|candidate| candidate.descriptor_id == descriptor_id)
            .ok_or_else(|| {
                format!("tool descriptor `{descriptor_id}` is not a searchable deferred candidate")
            })?;
        if candidate.source_revision != descriptor.source_revision {
            return Err(format!(
                "tool descriptor `{descriptor_id}` has a stale source revision: candidate holds `{}`, current snapshot holds `{}`",
                candidate.source_revision, descriptor.source_revision
            ));
        }
        view.elevate_from_registry(snapshot, descriptor_id)?;
        let record = ElevationRecord {
            descriptor_id: descriptor_id.to_string(),
            snapshot_id: snapshot.snapshot_id.clone(),
            source_revision: descriptor.source_revision.clone(),
            mutation_reason: format!(
                "model requested governed elevation of `{descriptor_id}` into the next provider hop"
            ),
        };
        self.records.push(record.clone());
        self.view = view.clone();
        Ok(record)
    }

    fn matches_query(&self, descriptor: &ToolDescriptor, query: &str) -> bool {
        descriptor.identity.model_name.to_lowercase().contains(query)
            || descriptor.identity.canonical_name.to_lowercase().contains(query)
            || descriptor.identity.primitive_name.to_lowercase().contains(query)
            || descriptor
                .aliases
                .iter()
                .any(|alias| alias.to_lowercase().contains(query))
            || descriptor.description.to_lowercase().contains(query)
    }

    fn rank(&self, candidate: &CandidateTool, query: &str) -> u8 {
        if query.is_empty() {
            return 0;
        }
        if candidate.model_name.to_lowercase() == query {
            3
        } else if candidate.model_name.to_lowercase().contains(query) {
            2
        } else if candidate.summary.to_lowercase().contains(query) {
            1
        } else {
            0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::tools::{
        ToolDescriptorSource, ToolDisplayMetadata, ToolExecutionPolicy, ToolHandlerProvenance,
        ToolIdentity, ToolKind, ToolPermissionDeclaration,
    };
    use serde_json::json;

    fn descriptor(id: &str, model_name: &str, exposure: ToolExposure, revision: &str) -> ToolDescriptor {
        ToolDescriptor {
            identity: ToolIdentity {
                descriptor_id: id.to_string(),
                model_name: model_name.to_string(),
                canonical_name: model_name.to_string(),
                primitive_name: id.to_string(),
                source: ToolDescriptorSource::Dynamic,
            },
            aliases: vec![format!("fixture.{model_name}")],
            description: format!("{model_name} description"),
            input_schema: json!({ "type": "object", "properties": {} }),
            kind: ToolKind::Read,
            exposure,
            permission_declaration: ToolPermissionDeclaration::default(),
            execution_policy: ToolExecutionPolicy::default(),
            display_metadata: ToolDisplayMetadata::default(),
            handler_provenance: ToolHandlerProvenance {
                handler_kind: "test".to_string(),
                source_id: "dynamic-source".to_string(),
            },
            source_revision: revision.to_string(),
            composed_descriptor_ids: Vec::new(),
        }
    }

    fn test_registry(snapshot_id: &str, descriptors: Vec<ToolDescriptor>) -> ToolRegistrySnapshot {
        ToolRegistrySnapshot::from_descriptors(snapshot_id, descriptors)
            .expect("test registry should build")
    }

    fn elevator_for(registry: &ToolRegistrySnapshot) -> ToolSearchElevator {
        ToolSearchElevator::new(registry.clone(), TurnToolView::from_registry(registry))
    }

    #[test]
    fn search_returns_only_deferred_candidates() {
        let registry = test_registry(
            "snapshot-v1",
            vec![
                descriptor("dynamic:visible", "VisibleTool", ToolExposure::ModelVisible, "v1"),
                descriptor("dynamic:internal", "InternalTool", ToolExposure::Internal, "v1"),
                descriptor("dynamic:deferred", "DeferredTool", ToolExposure::Deferred, "v1"),
            ],
        );
        let elevator = elevator_for(&registry);

        let candidates = elevator.search("");
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].descriptor_id, "dynamic:deferred");
        assert_eq!(candidates[0].model_name, "DeferredTool");
        assert_eq!(candidates[0].summary, "DeferredTool description");
        assert_eq!(candidates[0].source_revision, "v1");
    }

    #[test]
    fn search_query_filters_and_ranks_stable_candidates() {
        let registry = test_registry(
            "snapshot-v1",
            vec![
                descriptor("dynamic:image_view", "ImageView", ToolExposure::Deferred, "v1"),
                descriptor("dynamic:image_edit", "ImageEdit", ToolExposure::Deferred, "v1"),
                descriptor("dynamic:search_files", "SearchFiles", ToolExposure::Deferred, "v1"),
            ],
        );
        let elevator = elevator_for(&registry);

        let candidates = elevator.search("image");
        assert_eq!(candidates.len(), 2);
        // Both are model-name matches; the tie-break is the stable descriptor-id order.
        assert_eq!(candidates[0].descriptor_id, "dynamic:image_edit");
        assert_eq!(candidates[1].descriptor_id, "dynamic:image_view");
    }

    #[test]
    fn elevate_deferred_candidate_mutates_view_and_records_evidence() {
        let registry = test_registry(
            "snapshot-v1",
            vec![descriptor(
                "dynamic:image_view",
                "ImageView",
                ToolExposure::Deferred,
                "v1",
            )],
        );
        let mut elevator = elevator_for(&registry);
        let mut view = TurnToolView::from_registry(&registry);
        assert!(!view.allows_model_descriptor("dynamic:image_view"));

        let record = elevator
            .elevate(&registry, &mut view, "dynamic:image_view")
            .expect("deferred candidate should elevate");

        assert!(view.allows_model_descriptor("dynamic:image_view"));
        let contracts = view
            .provider_contract_views(&registry)
            .expect("matching snapshot should project provider contracts");
        assert!(contracts
            .iter()
            .any(|contract| contract.execution_primitive == "dynamic:image_view"));

        assert_eq!(record.descriptor_id, "dynamic:image_view");
        assert_eq!(record.snapshot_id, "snapshot-v1");
        assert_eq!(record.source_revision, "v1");
        assert!(record.mutation_reason.contains("governed elevation"));
        assert_eq!(elevator.records().len(), 1);
        assert_eq!(elevator.view().elevated_descriptor_ids.len(), 1);
    }

    #[test]
    fn elevating_non_deferred_candidate_fails_without_mutating_view() {
        let registry = test_registry(
            "snapshot-v1",
            vec![descriptor(
                "dynamic:internal",
                "InternalTool",
                ToolExposure::Internal,
                "v1",
            )],
        );
        let mut elevator = elevator_for(&registry);
        let mut view = TurnToolView::from_registry(&registry);

        let error = elevator
            .elevate(&registry, &mut view, "dynamic:internal")
            .expect_err("non-deferred descriptor must not elevate");
        assert!(error.contains("not deferred"), "{error}");
        assert!(!view.allows_model_descriptor("dynamic:internal"));
        assert!(!view.elevated_descriptor_ids.contains("dynamic:internal"));
        assert!(elevator.records().is_empty());
    }

    #[test]
    fn stale_source_revision_candidate_is_rejected() {
        let original = test_registry(
            "snapshot-v1",
            vec![descriptor(
                "dynamic:image_view",
                "ImageView",
                ToolExposure::Deferred,
                "v1",
            )],
        );
        let mut elevator = elevator_for(&original);
        let candidates = elevator.search("");
        let candidate = candidates
            .iter()
            .find(|candidate| candidate.descriptor_id == "dynamic:image_view")
            .expect("deferred candidate should be searchable");
        assert_eq!(candidate.source_revision, "v1");

        // The source is replaced: same descriptor id, new snapshot id, bumped revision.
        let current = test_registry(
            "snapshot-v2",
            vec![descriptor(
                "dynamic:image_view",
                "ImageView",
                ToolExposure::Deferred,
                "v2",
            )],
        );
        let mut view = TurnToolView::from_registry(&current);

        let error = elevator
            .elevate(&current, &mut view, &candidate.descriptor_id)
            .expect_err("stale source revision must fail closed");
        assert!(error.contains("stale source revision"), "{error}");
        assert!(!view.allows_model_descriptor("dynamic:image_view"));
        assert!(elevator.records().is_empty());
    }

    #[test]
    fn fresh_snapshot_that_dropped_descriptor_invalidates_candidate() {
        let original = test_registry(
            "snapshot-v1",
            vec![descriptor(
                "dynamic:image_view",
                "ImageView",
                ToolExposure::Deferred,
                "v1",
            )],
        );
        let mut elevator = elevator_for(&original);
        let candidates = elevator.search("");
        assert!(candidates
            .iter()
            .any(|candidate| candidate.descriptor_id == "dynamic:image_view"));

        // The current snapshot no longer contains the descriptor at all.
        let current = test_registry("snapshot-v2", Vec::new());
        let mut view = TurnToolView::from_registry(&current);

        let error = elevator
            .elevate(&current, &mut view, "dynamic:image_view")
            .expect_err("a candidate dropped from the current snapshot must fail closed");
        assert!(error.contains("not present in the current registry snapshot"), "{error}");
        assert!(!view.allows_model_descriptor("dynamic:image_view"));
        assert!(elevator.records().is_empty());
    }

    #[test]
    fn elevation_against_stale_turn_view_fails_closed() {
        let registry = test_registry(
            "snapshot-v1",
            vec![descriptor(
                "dynamic:image_view",
                "ImageView",
                ToolExposure::Deferred,
                "v1",
            )],
        );
        let mut elevator = elevator_for(&registry);

        // View bound to a different snapshot must be rejected before mutation.
        let other = test_registry(
            "snapshot-other",
            vec![descriptor(
                "dynamic:image_view",
                "ImageView",
                ToolExposure::Deferred,
                "v1",
            )],
        );
        let mut stale_view = TurnToolView::from_registry(&other);

        let error = elevator
            .elevate(&registry, &mut stale_view, "dynamic:image_view")
            .expect_err("view snapshot mismatch must fail closed");
        assert!(error.contains("does not match registry snapshot"), "{error}");
        assert!(!stale_view.allows_model_descriptor("dynamic:image_view"));
        assert!(elevator.records().is_empty());
    }

    #[test]
    fn multiple_elevations_append_trace_records() {
        let registry = test_registry(
            "snapshot-v1",
            vec![
                descriptor("dynamic:image_view", "ImageView", ToolExposure::Deferred, "v1"),
                descriptor("dynamic:config_read", "ConfigRead", ToolExposure::Deferred, "v1"),
            ],
        );
        let mut elevator = elevator_for(&registry);
        let mut view = TurnToolView::from_registry(&registry);

        elevator
            .elevate(&registry, &mut view, "dynamic:image_view")
            .expect("first elevation should succeed");
        elevator
            .elevate(&registry, &mut view, "dynamic:config_read")
            .expect("second elevation should succeed");

        assert_eq!(elevator.records().len(), 2);
        assert_eq!(elevator.records()[0].descriptor_id, "dynamic:image_view");
        assert_eq!(elevator.records()[1].descriptor_id, "dynamic:config_read");
        assert!(view.allows_model_descriptor("dynamic:image_view"));
        assert!(view.allows_model_descriptor("dynamic:config_read"));
    }
}
