# Workspace Shell Navigation

## MODIFIED Requirements

### Requirement: Trace and metrics SHALL live in a dedicated second-level observation page
The system SHALL provide an observation page (原"遥测页"，更名"观测") that hosts the turn trace explorer and the model metrics dashboard as tabs; it SHALL be reachable only through an explicit navigation action from a floating icon-only entry at the top-right of the conversation page's right rail and SHALL NOT be part of the conversation working surface.

#### Scenario: User opens the observation page
- **WHEN** the user activates the "观测" icon button to the left of the right-rail collapse toggle
- **THEN** the observation page SHALL render with a Trace tab (default, coding mode) and a metrics tab
- **AND** the Trace tab SHALL present the full turn trace explorer at page height

#### Scenario: Narrow window keeps the observation entry reachable
- **WHEN** the window is narrower than 1000px so the right sidebar auto-closes and its collapse toggle disappears
- **THEN** the observation icon button SHALL remain visible, repositioned to the collapse-toggle slot
- **AND** activating it SHALL still open the observation page

#### Scenario: User returns from the observation page
- **WHEN** the user activates the back control on the observation page
- **THEN** the app SHALL navigate back to the conversation home page

### Requirement: Trace visibility SHALL be gated to coding mode while metrics stay available in both modes
The observation entry SHALL be labeled "观测" in both workspace modes; the Trace tab SHALL be available only in coding mode, and the metrics tab SHALL be available in both modes. The trace explorer consumes the current conversation's turn data while the metrics dashboard aggregates across sessions with per-session drilldown.

#### Scenario: Work-mode user opens the observation page
- **WHEN** a work-mode user activates the observation entry
- **THEN** the observation page SHALL render only the metrics tab
- **AND** no turn trace explorer content SHALL be rendered

#### Scenario: Workspace mode resolves from coding to work while the user views the Trace tab
- **WHEN** persisted settings resolve to work mode while the observation page shows the Trace tab
- **THEN** the active tab SHALL converge to the first available tab (metrics)
- **AND** the Trace tab SHALL disappear from the tab list

### Requirement: The configuration page SHALL organize settings into tabs
The configuration page SHALL provide tabs for general settings (workspace mode, service keys), model configuration, and the tools catalog; the active tab SHALL be a controlled in-session state driven by explicit destinations and unknown tab values SHALL fall back to the general tab.

#### Scenario: User switches configuration tabs
- **WHEN** the user activates each configuration tab in turn
- **THEN** the corresponding tab panel renders and other panels stay unmounted

#### Scenario: User opens configuration from the remaining first-level sidebar destination
- **WHEN** the user activates "设置" in the left sidebar
- **THEN** the general tab SHALL be active
- **AND** model configuration SHALL be reached via the in-page models tab only

## REMOVED Requirements

### Requirement: Model configuration SHALL be a first-level sidebar destination
**Reason**: 冗余入口——配置页已承载"模型"tab；左栏一级键与折叠态图标删除，目的地收敛为配置页内 tab（由 MODIFIED 的 configuration-tabs requirement 覆盖）。

## ADDED Requirements

### Requirement: Provider management SHALL present a flat provider list with two hierarchical collapsible detail sections
The provider management surface SHALL render providers as a flat selectable list without accordion collapsing; its detail pane SHALL contain exactly two first-level collapsible sections labeled 提供商详情 and 模型列表; expanding a model row within the list SHALL reveal that model's configuration details in place.

#### Scenario: User selects a provider
- **WHEN** the user clicks a provider row in the flat list
- **THEN** the provider becomes selected with visible highlight and no expansion step
- **AND** both detail sections follow the selection

#### Scenario: User expands a model row
- **WHEN** the user activates an expanded-list model row
- **THEN** the row reveals its model config details (info, capabilities, parameters) in place
- **AND** activating it again collapses back to the provider view

#### Scenario: Editing locks folding and cross-row clicks
- **WHEN** a provider or model form is being edited
- **THEN** the owning section's fold toggle and sibling model rows SHALL be disabled
- **AND** save/cancel controls remain attached to the visible form's section or row

#### Scenario: Creating a provider hides the model list section
- **WHEN** the user starts creating a new provider
- **THEN** only the provider detail section renders with the create form
- **AND** the model list section is absent until the provider exists

#### Scenario: Adding a model from the list section
- **WHEN** the user activates 新增模型 on the model list section header
- **THEN** a create-model form card renders inside the list section
- **AND** saving returns to the new model's expanded detail view
