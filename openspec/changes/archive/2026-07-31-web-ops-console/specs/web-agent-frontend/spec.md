## ADDED Requirements

### Requirement: Browser agent frontend

The frontend SHALL be a browser application (React + Vite + TS in `web/`) that drives the agent by consuming the daemon's `/api/v1/*` HTTP+SSE surface. It is a parallel thin client of `src/tui`, not a new backend.

#### Scenario: Standalone dev server

- **WHEN** a developer runs the Vite dev server against a running daemon
- **THEN** the browser at the dev port can stream chat, execute tools, and approve permissions without any daemon code change beyond what already exists

#### Scenario: Client-side agent loop

- **WHEN** the model emits tool calls during a chat stream round
- **THEN** the frontend executes each via `POST /api/v1/tools/execute`, appends results, and re-streams the next round until a round produces no tool calls (the daemon never runs the loop)

#### Scenario: Optional daemon-hosted production build

- **WHEN** a production build is served (Tier 3)
- **THEN** it MAY be hosted by the daemon as static assets, reusing the same bearer-token auth as the API

### Requirement: Token-gated API access

The frontend SHALL authenticate to protected `/api/v1/*` endpoints with the daemon bearer token and MUST NOT embed the token in committed source or served HTML.

#### Scenario: Dev server token injection

- **WHEN** the Vite dev server proxies an `/api` request
- **THEN** the bearer token (read from `~/.wgenty-code/daemon.token`) is injected server-side by the proxy, never reaching browser bundle code

#### Scenario: No token in client bundle

- **WHEN** the production frontend bundle is inspected
- **THEN** it contains no hardcoded daemon token

### Requirement: Rich content rendering

The frontend SHALL render assistant output as GFM Markdown with syntax-highlighted code blocks, and SHALL render `file_edit`/`apply_patch` tool results as diffs.

#### Scenario: Markdown rendering

- **WHEN** the assistant streams Markdown content (headings, lists, fenced code)
- **THEN** the UI renders it as formatted Markdown, not raw text

#### Scenario: Code block highlighting

- **WHEN** a fenced code block is rendered
- **THEN** its syntax is highlighted

#### Scenario: Tool diff preview

- **WHEN** a `file_edit` or `apply_patch` tool result is displayed
- **THEN** the UI shows a unified diff view rather than raw tool output text

### Requirement: Interruptible agent turns

The frontend SHALL allow the user to interrupt a running agent turn.

#### Scenario: Stop between rounds

- **WHEN** the user activates the stop control while a turn is running
- **THEN** the in-flight stream is aborted at the next round boundary and the conversation is left in a consistent state

### Requirement: Permission approval UX

The frontend SHALL surface both root-tool synchronous permission prompts and subagent asynchronous permission prompts, resolving each via the appropriate endpoint.

#### Scenario: Root-tool approval

- **WHEN** `POST /api/v1/tools/execute` returns `permission_required`
- **THEN** the UI presents a modal offering Allow once / Always allow / Deny, and on approval follows the approve → execute → (optionally unapprove) sequence

#### Scenario: Subagent async approval

- **WHEN** a long-running `task`/`delegate` tool causes pending subagent permissions
- **THEN** the frontend polls `GET /api/v1/tools/pending-permissions` and surfaces each request, resolving via `POST /api/v1/tools/resolve-permission`

### Requirement: Session management UI

The frontend SHALL provide session list, search, open, and delete-with-confirmation using existing session APIs.

#### Scenario: List and open session

- **WHEN** the user opens the sessions view
- **THEN** the UI lists sessions and can open one, loading its history into the chat

#### Scenario: Persist and delete

- **WHEN** a turn completes or the user deletes a session
- **THEN** the conversation is persisted via `PUT /api/v1/sessions/:id`, and deletion requires explicit confirmation

### Requirement: Live side panels

The frontend SHALL render Todo, Task progress, and Model picker panels from existing APIs.

#### Scenario: Todo panel

- **WHEN** the agent is running
- **THEN** the UI shows live todos from `GET /api/v1/todos` with their statuses

#### Scenario: Model switching

- **WHEN** the user opens the model picker
- **THEN** the UI lists profiles from `GET /api/v1/models` and switching calls `POST /api/v1/model/switch`
