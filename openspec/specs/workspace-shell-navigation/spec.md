# workspace-shell-navigation Specification

## Purpose

规范 Pony Agent 工作台外壳的信息架构与导航契约：对话页右侧栏只承载会话过程面板（状态 / Plan / Debug），Turn trace 与模型指标收敛为由右栏浮动图标进入的二级「观测」页（Trace tab 仅 coding 模式，指标 tab 双模式可见），工具目录与设置迁入 tab 化配置页，左侧栏以工作区区段为第一优先。

同时规范提供商管理页的扁平提供商列表与「提供商详情 / 模型列表」两级折叠详情交互（折叠即取消编辑态、空闲行尾操作仅悬停或键盘聚焦时可见），以及页签与配置页的 WAI-ARIA APG 键盘漫游与页面切换焦点落点。

对应任务卡 `PA-096` 与 OpenSpec change `openspec/changes/archive/2026-08-22-workspace-shell-layout-optimization/`，以及后续观测入口与提供商层级变更 `openspec/changes/archive/2026-08-24-workspace-nav-observation-entry-and-provider-hierarchy/`。

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

### Requirement: Provider management SHALL present a flat provider list with two hierarchical collapsible detail sections
The provider management surface SHALL render providers as a flat selectable list without accordion collapsing; its detail pane SHALL contain exactly two first-level collapsible sections labeled 提供商详情 and 模型列表; expanding a model row within the list SHALL reveal that model's configuration details in place. Both first-level section headers and model rows SHALL show a pointer cursor across the entire row, and idle trailing icon actions SHALL be revealed only while the row is hovered or keyboard-focused.

#### Scenario: User selects a provider
- **WHEN** the user clicks a provider row in the flat list
- **THEN** the provider becomes selected with visible highlight and no expansion step
- **AND** both detail sections follow the selection

#### Scenario: User expands a model row
- **WHEN** the user activates an expanded-list model row
- **THEN** the row reveals its model config details (info, capabilities, parameters) in place
- **AND** activating it again collapses back to the provider view

#### Scenario: Collapsing a section hosting unsaved edits cancels the editing state
- **WHEN** any fold toggle (including an accordion cross-collapse triggered from the other section) causes the section or expanded row that hosts an active create/edit form to collapse
- **THEN** the editing state SHALL be cancelled first (returning to view mode for that entity)
- **AND** no save/cancel control SHALL remain attached to a hidden form
- **AND** cancelling a create-provider form SHALL discard and reset the form rather than keeping it alive behind the fold

#### Scenario: Idle trailing actions appear on hover only
- **WHEN** a first-level section header or model row is in idle (non-editing) state
- **THEN** its trailing icon actions are hidden until the row is hovered or receives keyboard focus within
- **AND** active edit-state controls (取消/保存) remain always visible regardless of hover

#### Scenario: Editing no longer locks folding
- **WHEN** a provider or model form is being edited
- **THEN** fold toggles and sibling rows stay interactive
- **AND** activating them dismisses the editing state as defined above instead of being disabled

#### Scenario: Creating a provider hides the model list section
- **WHEN** the user starts creating a new provider
- **THEN** only the provider detail section renders with the create form
- **AND** the model list section is absent until the provider exists

#### Scenario: Adding a model from the list section
- **WHEN** the user activates 新增模型 on the model list section header
- **THEN** a create-model form card renders inside the list section
- **AND** saving returns to the new model's expanded detail view

### Requirement: The workspace section SHALL be the first priority section in the left sidebar
The left sidebar SHALL place the workspace section above the conversation list, and the section header SHALL surface the active workspace name; the workspace badge in the action row SHALL be removed to avoid duplicated toggles.

#### Scenario: User scans the left sidebar
- **WHEN** the left sidebar is rendered in expanded mode
- **THEN** the workspace section appears before the conversation section in DOM order
- **AND** the workspace section header shows the active workspace name

### Requirement: The configuration page SHALL organize settings into tabs
The configuration page SHALL provide tabs for general settings (workspace mode, service keys), model configuration, and the tools catalog; the active tab SHALL be a controlled in-session state driven by explicit destinations and unknown tab values SHALL fall back to the general tab.

#### Scenario: User switches configuration tabs
- **WHEN** the user activates each configuration tab in turn
- **THEN** the corresponding tab panel renders and other panels stay unmounted

#### Scenario: User opens configuration from the remaining first-level sidebar destination
- **WHEN** the user activates "设置" in the left sidebar
- **THEN** the general tab SHALL be active
- **AND** model configuration SHALL be reached via the in-page models tab only

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
