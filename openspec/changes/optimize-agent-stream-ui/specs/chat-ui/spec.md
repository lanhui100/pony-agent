# chat-ui Delta

## ADDED Requirements

### Requirement: Plain assistant transcript rendering

The main conversation assistant response area SHALL display assistant message content as raw plain text without markdown-to-HTML rendering.

#### Scenario: Assistant returns markdown-like text

- WHEN an assistant message content contains markdown markers such as `#`, `**`, or fenced code
- THEN the conversation response body displays those markers as text
- AND no markdown renderer is required for the response body

### Requirement: Buffered streaming presentation

Assistant streaming text SHALL be buffered and flushed to the frontend only when a character threshold or time threshold is reached.

#### Scenario: Small stream deltas arrive rapidly

- WHEN multiple small assistant deltas arrive before the threshold
- THEN the UI does not update for every individual delta
- AND the next flushed batch appears through the existing fade-in presentation layer

### Requirement: Terminal-only assistant actions

Assistant message action controls SHALL be hidden while the assistant message is pending and shown once the message is no longer pending.

#### Scenario: Assistant is streaming

- WHEN an assistant message status is `pending`
- THEN copy controls under that assistant message are not rendered

#### Scenario: Assistant is complete

- WHEN an assistant message status is `done` or `error`
- THEN copy controls under that assistant message may be rendered

### Requirement: Plain and concise first-turn guidance

The stable system prompt SHALL instruct the model to avoid markdown in replies and keep responses concise by default unless the user asks for detail.
