## ADDED Requirements

### Requirement: Overview API

The daemon SHALL expose `GET /api/v1/overview` on the protected router, returning a JSON summary for the current project-bound daemon instance.

#### Scenario: Overview success

- **WHEN** an authenticated client calls `GET /api/v1/overview`
- **THEN** the response includes project root path, session count, memory status summary fields (including project and global counts when available), main model name, and daemon/app version string

#### Scenario: Overview requires auth

- **WHEN** an unauthenticated client calls `GET /api/v1/overview`
- **THEN** the request is rejected with the same auth failure behavior as other protected routes

### Requirement: Memory status API

The daemon SHALL expose `GET /api/v1/memory/status` that returns the current `MemoryStatus` (or equivalent JSON) from `MemoryManager`.

#### Scenario: Status reflects dual pools

- **WHEN** project and global memories exist
- **THEN** the status payload reports both project and global counts

### Requirement: Memory list API

The daemon SHALL expose `GET /api/v1/memory` to list memories with optional filters.

#### Scenario: Default list

- **WHEN** an authenticated client calls `GET /api/v1/memory` without filters
- **THEN** the response is a JSON list (or `{ items: [...] }`) of memory summaries including id, type, importance, timestamp, origin (project|global), and a content preview or full content consistent with size limits documented by implementation

#### Scenario: Scope filter

- **WHEN** the client passes `scope=project` or `scope=global`
- **THEN** only memories from that origin are returned

#### Scenario: Importance and pagination

- **WHEN** the client passes `min_importance`, `limit`, and `offset`
- **THEN** results respect those constraints

### Requirement: Memory get by id API

The daemon SHALL expose `GET /api/v1/memory/:id` returning one memory including origin.

#### Scenario: Found

- **WHEN** the id exists
- **THEN** the response includes full content, metadata/tags when present, and origin

#### Scenario: Not found

- **WHEN** the id does not exist
- **THEN** the API returns a 404-class error response

### Requirement: Memory prune API

The daemon SHALL expose `POST /api/v1/memory/prune` that invokes existing prune logic and returns a structured prune result.

#### Scenario: Prune executes

- **WHEN** an authenticated client posts to `/api/v1/memory/prune`
- **THEN** the response includes before/after/removed counts (including per-pool fields when available)

### Requirement: Expanded read-only config API

`GET /api/v1/config` SHALL return an ops-oriented read-only DTO broader than model transport alone, and MUST redact secrets.

#### Scenario: Grouped safe fields

- **WHEN** an authenticated client calls `GET /api/v1/config`
- **THEN** the response includes safe summaries for models (names/base URLs without raw api_key), transport, agent toggles/budgets summary, guardian/sandbox enablement summary, and memory storage thresholds summary

#### Scenario: Secrets redacted

- **WHEN** settings contain an API key
- **THEN** the config JSON does not include the raw api_key string

#### Scenario: Read only in P0

- **WHEN** a client attempts to modify configuration via the ops console API surface in P0
- **THEN** no successful config-write endpoint is provided for that purpose (no PUT/PATCH `/api/v1/config` requirement in P0)

### Requirement: Sessions API reuse

The ops console backend MUST continue to expose existing session list/search/get/delete endpoints for console consumption without breaking current response fields used by clients.

#### Scenario: Session detail includes messages

- **WHEN** an authenticated client gets a session by id
- **THEN** the response includes message history suitable for read-only display
