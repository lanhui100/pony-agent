# workspace-read-document Delta

## ADDED Requirements

### Requirement: Workspace document to Markdown conversion

The builtin tool `workspace_read_document` SHALL convert a workspace-scoped office document (Word, PowerPoint, Excel, OpenDocument, RTF, EPUB, CSV, PDF) into GitHub-Flavored Markdown text and return it to the model.

#### Scenario: Reading a supported document

- GIVEN a supported office document inside the workspace (for example `.docx`, `.pptx`, `.xlsx`, `.pdf`, `.csv`)
- WHEN the model invokes `workspace_read_document` with its relative path
- THEN the tool SHALL return the converted Markdown content
- AND the tool SHALL return the detected format
- AND the tool SHALL return the canonical absolute path of the document

#### Scenario: Format detected from content

- WHEN a document's extension is missing or misleading
- THEN the tool SHALL still detect the format from the file bytes (content markers)
- AND convert the document successfully when the content is supported

#### Scenario: Unsupported or non-document input

- WHEN the target is not a supported document format
- THEN the tool SHALL fail closed with an explicit error message
- AND the tool SHALL NOT return partial garbage text

### Requirement: Workspace path confinement

`workspace_read_document` SHALL only read files whose canonical path resolves inside the workspace root.

#### Scenario: Path escapes the workspace

- WHEN the given path canonicalizes outside the workspace root (for example via `..` or an absolute path)
- THEN the tool SHALL reject the call with a clear error
- AND the file SHALL NOT be read

#### Scenario: Target is not a regular file

- WHEN the path points to a directory or a missing file
- THEN the tool SHALL fail with an explicit error

### Requirement: Bounded output and explicit truncation

The tool SHALL enforce explicit byte budgets and never silently truncate.

#### Scenario: Output exceeds the byte budget

- WHEN the converted Markdown exceeds `maxOutputBytes`
- THEN the tool SHALL return a truncated Markdown payload
- AND the tool SHALL set `truncated: true` and report the full output length

#### Scenario: Input exceeds the input budget

- WHEN the source document exceeds the configured input byte budget
- THEN the tool SHALL reject the call before conversion

### Requirement: Governed registration and observability

The tool SHALL be registered as a builtin `ModelVisible` descriptor and execute through the governed dispatcher.

#### Scenario: Model-visible contract

- WHEN the provider contract is projected from the registry
- THEN `workspace_read_document` SHALL appear with its input schema and workspace.read permission declaration

#### Scenario: Failure surfaces as structured error

- WHEN conversion fails (for example scanned PDF without OCR support)
- THEN the tool SHALL return a structured error with a clear message explaining the limit
