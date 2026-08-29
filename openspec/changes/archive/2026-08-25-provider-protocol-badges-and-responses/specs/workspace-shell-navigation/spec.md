# workspace-shell-navigation Delta

## MODIFIED Requirements

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
