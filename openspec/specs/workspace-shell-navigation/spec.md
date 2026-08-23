# Workspace Shell Navigation

## Requirements

### Requirement: Conversation right sidebar SHALL host only conversation-process panels
The home right sidebar SHALL render the session status panel, plan panel, and debug panel only; the tools catalog and the turn trace explorer SHALL NOT be rendered inside the conversation right sidebar.

#### Scenario: User opens the conversation page in coding mode
- **WHEN** the home page is rendered in coding mode
- **THEN** the right sidebar contains the status, plan, and debug panels
- **AND** no tools catalog section and no trace explorer section exist in the sidebar DOM

#### Scenario: User opens the conversation page in work mode
- **WHEN** the home page is rendered in work mode
- **THEN** the right sidebar contains the status, plan, and debug panels
- **AND** no trace entry point of any kind is rendered in the sidebar

### Requirement: Trace and metrics SHALL live in a dedicated second-level telemetry page
The system SHALL provide a telemetry page that hosts the turn trace explorer and the model metrics dashboard as tabs; it SHALL be reachable only through an explicit navigation action and SHALL NOT be part of the conversation working surface.

#### Scenario: Coding-mode user opens telemetry
- **WHEN** a coding-mode user activates the telemetry entry in the left sidebar
- **THEN** the telemetry page SHALL render with a Trace tab (default) and a metrics tab
- **AND** the Trace tab SHALL present the full turn trace explorer at page height

#### Scenario: User returns from telemetry
- **WHEN** the user activates the back control on the telemetry page
- **THEN** the app SHALL navigate back to the conversation home page

### Requirement: Trace visibility SHALL be gated to coding mode while metrics stay available in both modes
The telemetry entry SHALL be labeled "遥测" in coding mode and "指标" in work mode; the Trace tab SHALL be available only in coding mode, and the metrics tab SHALL be available in both modes.

#### Scenario: Work-mode user opens the telemetry entry
- **WHEN** a work-mode user activates the telemetry entry
- **THEN** the telemetry page SHALL render only the metrics tab
- **AND** no turn trace explorer content SHALL be rendered

#### Scenario: Workspace mode resolves from coding to work while the user views the Trace tab
- **WHEN** persisted settings resolve to work mode while the telemetry page shows the Trace tab
- **THEN** the active tab SHALL converge to the first available tab (metrics)
- **AND** the Trace tab SHALL disappear from the tab list

### Requirement: Model configuration SHALL be a first-level sidebar destination
The left sidebar SHALL expose model configuration as a top-level navigation item that opens the configuration page with the models tab active; model configuration SHALL NOT be nested inside a collapsible group.

#### Scenario: User activates the model configuration entry
- **WHEN** the user activates the "模型配置" sidebar item
- **THEN** the configuration page SHALL be displayed with the models tab active
- **AND** the models tab SHALL host the existing provider/model management surface

### Requirement: The workspace section SHALL be the first priority section in the left sidebar
The left sidebar SHALL place the workspace section above the conversation list, and the section header SHALL surface the active workspace name; the workspace badge in the action row SHALL be removed to avoid duplicated toggles.

#### Scenario: User scans the left sidebar
- **WHEN** the left sidebar is rendered in expanded mode
- **THEN** the workspace section appears before the conversation section in DOM order
- **AND** the workspace section header shows the active workspace name

### Requirement: The configuration page SHALL organize settings into tabs
The configuration page SHALL provide tabs for general settings (workspace mode, service keys), model configuration, and the tools catalog; the active tab SHALL be a controlled in-session state driven by explicit sidebar destinations and unknown tab values SHALL fall back to the general tab.

#### Scenario: User switches configuration tabs
- **WHEN** the user activates each configuration tab in turn
- **THEN** the corresponding tab panel renders and other panels stay unmounted

#### Scenario: User opens configuration from a first-level sidebar destination
- **WHEN** the user activates "模型配置" or "设置" in the left sidebar
- **THEN** the models tab or the general tab SHALL be active respectively

### Requirement: Tools catalog SHALL be presented in the configuration page
The tools catalog (available tools with permission summaries) SHALL render as a static list in the configuration page tools tab, with Chinese display names preferred and an explicit empty state.

#### Scenario: User inspects available tools
- **WHEN** the tools tab is opened
- **THEN** each available tool lists its display name (Chinese short name first), kind, description, and permission/approval/source summary
- **AND** when no tools are available an explicit empty-state message renders

### Requirement: Telemetry page tabs and configuration tabs SHALL meet APG tab accessibility
Tab strips SHALL use role tablist/tab/tabpanel with aria-selected and aria-controls wiring, roving tabindex, and arrow-key navigation; page transitions SHALL move focus to the new page heading.

#### Scenario: Keyboard user navigates a tab strip
- **WHEN** a tab strip has focus and the user presses ArrowLeft/ArrowRight/Home/End
- **THEN** selection moves according to WAI-ARIA APG tab semantics
