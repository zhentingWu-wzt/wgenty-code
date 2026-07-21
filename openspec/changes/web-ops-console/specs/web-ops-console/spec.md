## ADDED Requirements

### Requirement: Ops console static shell

The daemon SHALL serve a browser-accessible ops console shell on the same listen address as the API (default loopback), including an entry document and client assets required for P0 navigation.

#### Scenario: Open console root

- **WHEN** a user requests `GET /` on the daemon HTTP port
- **THEN** the response is an HTML document for the ops console shell (not an API error)

#### Scenario: Hash navigation without server routes

- **WHEN** the user navigates between Overview, Sessions, Memory, and Config views in the shell
- **THEN** navigation works without requiring additional authenticated document routes beyond the static shell (e.g. hash routing or equivalent SPA fallback)

### Requirement: Token-gated API access from the console

The ops console SHALL require the daemon API token for calling protected `/api/v1/*` endpoints and MUST NOT embed the API token in served HTML or static assets.

#### Scenario: First visit without token

- **WHEN** the console has no token stored in the browser session
- **THEN** the UI prompts for a token before loading protected overview data

#### Scenario: Authenticated fetch

- **WHEN** the user has provided a valid token
- **THEN** subsequent API requests include `Authorization: Bearer <token>`

### Requirement: Overview page

The console SHALL provide an Overview view that displays project operations summary data from the ops API.

#### Scenario: Overview renders summary

- **WHEN** the Overview view loads with a valid token
- **THEN** the UI shows project root, session count, memory project/global counts (or equivalent status fields), and current main model name

### Requirement: Sessions management UI

The console SHALL provide session list, search, read-only detail (including messages), and delete-with-confirmation using existing session APIs.

#### Scenario: List and open session

- **WHEN** the user opens the Sessions view
- **THEN** the UI lists sessions and can open a session detail showing messages read-only

#### Scenario: Delete session requires confirmation

- **WHEN** the user chooses to delete a session
- **THEN** the UI asks for explicit confirmation before calling the delete API

### Requirement: Memory management UI

The console SHALL provide memory status/list/detail views and prune-with-confirmation.

#### Scenario: Filter memory list

- **WHEN** the user filters by scope and/or minimum importance
- **THEN** the list reflects the filter criteria via the memory list API

#### Scenario: Prune requires confirmation

- **WHEN** the user triggers memory prune
- **THEN** the UI asks for explicit confirmation before calling the prune API

### Requirement: Read-only config UI

The console SHALL display a read-only configuration dashboard from the ops config API and MUST NOT offer config write controls in P0.

#### Scenario: Config masks secrets

- **WHEN** the Config view renders model endpoint settings
- **THEN** API keys and tokens are shown only as masked/redacted indicators, never as full secrets

#### Scenario: No write controls

- **WHEN** the user views the Config page in P0
- **THEN** there is no control that submits a config update request
