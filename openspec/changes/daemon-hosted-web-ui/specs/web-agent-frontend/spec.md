# web-agent-frontend Specification（Delta）

## MODIFIED Requirements

### Requirement: Browser agent frontend

The frontend SHALL be a browser application (React + Vite + TS in `web/`) that drives the agent by consuming the daemon's `/api/v1/*` HTTP+SSE surface. It is a parallel thin client of `src/tui`, not a new backend.

#### Scenario: Standalone dev server

- **WHEN** a developer runs the Vite dev server against a running daemon
- **THEN** the browser at the dev port can stream chat, execute tools, and approve permissions without any daemon code change beyond what already exists

#### Scenario: Client-side agent loop

- **WHEN** the model emits tool calls during a chat stream round
- **THEN** the frontend executes each via `POST /api/v1/tools/execute`, appends results, and re-streams the next round until a round produces no tool calls (the daemon never runs the loop)

#### Scenario: Daemon-hosted production build

- **WHEN** a production build is embedded in the daemon binary (Tier 3)
- **THEN** the browser opening the daemon origin gets the full UI with no Node/Vite toolchain required, and the page authenticates to the API via the daemon's same-origin bootstrap flow

### Requirement: Token-gated API access

The frontend SHALL authenticate to protected `/api/v1/*` endpoints with the daemon bearer token and MUST NOT embed the token in committed source or served HTML.

#### Scenario: Dev server token injection

- **WHEN** the Vite dev server proxies an `/api` request
- **THEN** the bearer token (read from `~/.wgenty-code/daemon.token`) is injected server-side by the proxy, never reaching browser bundle code

#### Scenario: No token in client bundle

- **WHEN** the production frontend bundle is inspected
- **THEN** it contains no hardcoded daemon token

#### Scenario: Daemon-hosted bootstrap token acquisition

- **WHEN** the page is served by the daemon itself (same-origin) and needs the bearer token
- **THEN** it obtains the token from the daemon's same-origin bootstrap endpoint at startup and attaches it to protected API calls, with no token embedded in the served HTML
