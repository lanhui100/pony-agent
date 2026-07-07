# chat-ui Delta

## ADDED Requirements

### Requirement: Pending assistant rendering SHALL prefer low-reflow streaming presentation

The main conversation assistant response area SHALL avoid full markdown-to-HTML re-rendering on every streaming flush while an assistant message is pending.

#### Scenario: Assistant message is pending and receives more text

- WHEN an assistant message is still streaming
- THEN the UI SHALL render the newly visible assistant text through a lightweight streaming presentation path
- AND the transcript body SHALL NOT require a full markdown DOM replacement for every flushed delta
- AND this lightweight path SHALL render the pending assistant main text as plain text inside the existing assistant shell container
- AND this lightweight path SHALL NOT call the full completed-message MarkdownRenderer for each flushed delta

#### Scenario: Assistant message reaches a terminal state

- WHEN an assistant message transitions from `pending` to `done` or `error`
- THEN the final assistant content SHALL still be rendered through the normal completed-message render path
- AND the terminal handoff SHALL preserve visually stable layout boundaries

### Requirement: Pending and terminal assistant states SHALL reuse a stable shell

The assistant response panel SHALL keep one stable outer content shell across pending and terminal states so the terminal handoff does not replace the whole content subtree.

#### Scenario: Assistant transitions from pending to done

- WHEN a pending assistant message becomes `done`
- THEN the outer assistant response shell SHALL remain mounted
- AND only the inner content mode MAY switch from pending plain-text presentation to completed markdown presentation

#### Scenario: Assistant transitions from pending to error

- WHEN a pending assistant message becomes `error`
- THEN the outer assistant response shell SHALL remain mounted
- AND the handoff SHALL still avoid a transient empty visual gap

### Requirement: Streaming handoff SHALL avoid fade/stable visibility gaps

The fade layer and stable layer SHALL hand off without a frame where the outgoing delta disappears before the stable layer is ready.

#### Scenario: A buffered streaming batch flushes into the stable transcript

- WHEN the frontend advances a pending assistant batch from fade content into stable content
- THEN the stable layer SHALL already contain the corresponding visible text before the fade layer fully disappears
- AND the user SHALL observe continuous upward growth rather than a drop-and-reappear jump

#### Scenario: Stable content is not ready yet

- WHEN the fade layer has content to hand off but the next stable render step is not yet ready
- THEN the fade layer SHALL remain visible until the stable layer is ready for the same visible text
- AND the UI SHALL NOT show a transient empty gap between fade and stable content

### Requirement: Streaming auto-follow SHALL behave like anchored following

The conversation viewport SHALL follow the newest streaming assistant growth as a bottom-anchored movement instead of repeatedly compensating after independent layout jumps.

#### Scenario: Streaming assistant content grows while auto-follow is enabled

- WHEN the user has not opted out of following the latest content
- THEN the viewport SHALL move in a single forward-follow direction as content grows
- AND repeated resize compensation SHALL NOT cause visible up-down wobble during normal streaming

#### Scenario: Markdown render completion is available during streaming

- WHEN the markdown renderer reports that a pending render step has completed
- THEN the auto-follow controller SHALL be able to use that signal directly
- AND the workspace SHALL wire that signal into the auto-follow controller directly
- AND it SHALL NOT rely only on generic ResizeObserver callbacks to infer layout completion

#### Scenario: Streaming follow signal priority

- WHEN both a markdown render-complete signal and a generic resize signal are available during streaming
- THEN the render-complete signal SHALL be treated as the primary follow trigger for the streaming content handoff
- AND generic resize callbacks MAY remain as safety fallback but SHALL NOT drive repeated competing catch-up scrolls during the same handoff window

### Requirement: Assistant action controls SHALL remain terminal-only

Assistant message action controls SHALL remain hidden while the assistant message is pending and shown once the message is no longer pending.

#### Scenario: Assistant is streaming

- WHEN an assistant message status is `pending`
- THEN copy controls under that assistant message are not rendered

#### Scenario: Assistant is complete

- WHEN an assistant message status is `done` or `error`
- THEN copy controls under that assistant message may be rendered

## Validation Notes

- Automated tests SHOULD verify render-path selection, terminal handoff state transitions, revision-mismatch fallback, and direct render-complete signal consumption.
- Manual verification MAY be used for the final visual smoothness judgement, because continuous upward motion and visible wobble are not fully expressible through jsdom assertions alone.
