# add-chat-file-attachment-entry Delta

## ADDED Requirements

### Requirement: Composer attachment entry point

The conversation composer SHALL expose a visible attachment (paperclip) button that opens a file picker, and SHALL render the list of files selected for the next send.

#### Scenario: Picking a supported file

- GIVEN the composer attachment button
- WHEN the user clicks it and selects a file whose type is in the supported registry
- THEN the file SHALL be added to the pending attachment list
- AND the attachment SHALL show its name and size

#### Scenario: Picking an unsupported file

- WHEN the user selects a file whose extension/MIME is not in the supported registry
- THEN the file SHALL NOT be added to the pending list
- AND a clear "暂不支持该文件类型" message SHALL be shown

#### Scenario: Removing a pending attachment

- WHEN the user clicks remove on a pending attachment
- THEN the attachment SHALL be removed from the pending list

#### Scenario: No message with attachments

- WHEN the user sends with only attachments and no text
- THEN the send SHALL proceed with an auto-generated summary message listing the attachments

#### Scenario: Duplicate attachment

- WHEN the user adds a file already in the pending list (same path, name and size)
- THEN the file SHALL NOT be added again
- AND a "已添加" message SHALL be shown

#### Scenario: Attachment count limit

- WHEN the user adds a 4th image attachment
- THEN the image SHALL be rejected with an explicit message (limit 3, matching the backend `MAX_TURN_IMAGES`)

#### Scenario: Pending list cleared after send

- WHEN a turn is sent
- THEN the pending attachment list SHALL be cleared

#### Scenario: Pending list cleared on session switch

- WHEN the user switches session or creates a new one while attachments are pending
- THEN the pending attachment list SHALL be cleared

### Requirement: File type registry

The project SHALL maintain a single data-driven registry mapping file types to handlers, so that adding a new supported type only requires a registry entry. The registry table in this spec is canonical; `design.md` copies it verbatim.

#### Scenario: Registry drives routing

- WHEN a file is added
- THEN its route (image / text / document) SHALL be resolved from the registry by extension first with MIME fallback
- AND the registry SHALL be the single source of truth for supported types

#### Scenario: Registry entries

- GIVEN the registry
- THEN images (png/jpg/jpeg/webp/gif) SHALL route to the image handler (multimodal input)
- AND text formats (md/txt/json/csv and common source files) SHALL route to the text handler (content injection)
- AND binary documents (pdf/docx/pptx/xlsx) SHALL route to the document handler (reference-only)
- AND unknown types SHALL be rejected with the unsupported message

#### Scenario: MIME conflict

- WHEN a file's extension is supported but its MIME is not (or vice versa)
- THEN the extension SHALL win for routing
- AND an image whose magic bytes do not match its declared type SHALL be rejected

### Requirement: File processing routing

Each accepted file SHALL be processed by its registry handler before send: images become multimodal `TurnInputImage` entries; text is read and injected with a content budget; binary documents are attached by reference (path + MIME) without content injection.

#### Scenario: Image file

- WHEN an image file is added
- THEN it SHALL be converted to a `TurnInputImage` (dataUrl + mime + name)
- AND it SHALL be sent through the existing `submitTurn({ images })` path

#### Scenario: Text file content injection

- WHEN a text file is added
- THEN its content SHALL be read (budget 64 KiB)
- AND the content SHALL be attached to the message so the model can process it
- AND the file path SHALL be recorded in the attachment metadata

#### Scenario: Oversized text truncation

- WHEN a text file's content exceeds the 64 KiB injection budget
- THEN the injected content SHALL be truncated with an explicit truncation marker

#### Scenario: Binary document reference

- WHEN a binary document (pdf/docx/pptx/xlsx) is added
- THEN no content SHALL be injected
- AND its path and MIME SHALL be attached as reference metadata so the model can call `workspace_read_document`

#### Scenario: Oversized file

- WHEN a file exceeds the configured size budget
- THEN it SHALL be rejected with an explicit size message
- AND it SHALL NOT enter the pending list

### Requirement: External file import

Files SHALL be imported by the host into a controlled directory before processing; the frontend SHALL NOT write to the host filesystem directly. The host SHALL resolve the import target root from the `workspaceId` argument; this round only the default workspace (or absent) is accepted, and a non-default `workspaceId` SHALL be rejected explicitly (PA-079 注册表落地后改为按注册表解析)。

#### Scenario: Import succeeds

- WHEN the host copies the submitted bytes successfully
- THEN the copied path SHALL be used as the attachment source
- AND the path SHALL be under the controlled import directory of the session's workspace (`<session.workspace_root>/.tmp/imports/`, or the stable non-PID temp fallback)
- AND the host SHALL return the reference path relative to the workspace root (`.tmp/imports/<name>`), or null when the file landed in the temp fallback

#### Scenario: Non-default workspace rejected

- WHEN an import is submitted with a `workspaceId` that is not the default
- THEN the host SHALL reject it with an explicit structured error
- AND no bytes SHALL be written
- AND a file attached under an active non-default workspace SHALL remain rejected until PA-079 wires registry-based root resolution

#### Scenario: Import rejects unsafe name

- WHEN the import `name` contains a path separator or a `..` component
- THEN the host SHALL reject it with a structured error
- AND no bytes SHALL be written

#### Scenario: Import fails

- WHEN the host copy fails
- THEN the file SHALL appear in the pending list in an error state (removable, and SHALL NOT be included in send)
- AND the pending list SHALL otherwise remain unchanged

#### Scenario: Browser mode (no host filesystem)

- WHEN running in browser mode where no host is available to copy
- THEN every selected file SHALL be treated as external
- AND the attachment SHALL be referenced in-memory only (no persisted path)
- AND a binary document SHALL be rejected with a "预览模式暂不支持" message
