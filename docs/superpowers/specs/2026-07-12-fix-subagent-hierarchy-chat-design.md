# Fix Scoped Subagent Hierarchy and Chat Design

## Context

The TUI recently migrated from a flattened subagent tree to daemon-provided
local views. Each local view contains the viewed agent and its direct children,
and navigation into a child is authorized by an opaque capability.

The current implementation has several connected defects:

- a viewed agent with children and no locally populated messages is classified
  as a grouping node and omitted from the selector;
- `SelfAgentResponse` contains only identity and lifecycle status, so navigating
  into a child replaces its previously visible child projection with an empty
  self projection and clears the conversation area;
- periodic root polling replaces the currently navigated child view, causing
  the selector and conversation to fall back to root-scoped data;
- the navigation endpoint discovers capability targets by scanning only the
  root's direct children, so a capability issued for a grandchild cannot be
  exercised;
- selector state and conversation state can temporarily refer to different
  agents after a scoped-view replacement.

## Goals

- The root window shows `main` and main's direct children.
- A child window shows `main`, the currently viewed child, and that child's
  direct children.
- Deeper navigation repeats the same rule without flattening descendants.
- A breadcrumb such as `main > child > grandchild` communicates the current
  hierarchy.
- The conversation area always displays the messages of the agent selected in
  the selector.
- Scoped views remain live while subagents run.
- Navigation continues to enforce capability-based isolation.

## Non-goals

- Returning a complete agent tree to the TUI.
- Allowing navigation by an arbitrary raw agent ID.
- Showing ancestors other than `main` as selectable hierarchy entries. The
  intermediate path is represented by the breadcrumb and navigation history.
- Mixing the main conversation history into a selected subagent's conversation.

## Selected Approach

Use complete scoped view projections and UI-owned navigation state. The daemon
continues to return only one agent plus its direct children. Each projection
contains enough display data to render that agent's conversation, and the TUI
refreshes the currently active scope rather than replacing it with root data.

This preserves strict isolation while supporting arbitrary depth. It is
preferred over copying messages from the previous frame because copied data
would become stale, and over returning a flattened tree because that would
weaken the security boundary.

## View Model

### Self projection

`SelfAgentResponse` will carry the same display-oriented fields needed by the
focus view as a direct child projection:

- agent ID;
- lifecycle status;
- display label;
- latest text snapshot;
- cumulative token count;
- captured chat messages.

The daemon obtains these fields from the coordinator/store and the existing
progress projection. These fields are for trusted UI rendering and are never
inserted into model input.

### Local view

A `LocalAgentViewResponse` remains strictly scoped to:

- `self_view`: the currently viewed agent;
- `children`: only that agent's direct children.

No parent ID, sibling, arbitrary descendant, or other-branch record is added.

## Selector Semantics

The selector is constructed from explicit local-view roles, not from
`real_node_list()` or the grouping-node heuristic.

At the root scope it contains:

1. `main`, representing the root agent;
2. each direct child of main.

At a non-root scope it contains:

1. `main`, representing the root agent and providing a direct return action;
2. the currently viewed agent;
3. each direct child of the currently viewed agent.

The current agent receives a visible current marker. Completed children may
still follow the existing delayed-removal policy, but the current agent is
never filtered out.

The selector must not duplicate the root self projection: in the root scope,
the `main` entry is the root agent.

## Conversation Invariant

The selector selection is the sole source of truth for the conversation area:

> The conversation area displays exactly the selected agent's captured chat
> messages.

Consequences:

- moving the selector to the current scoped agent immediately displays
  `self_view.messages`;
- moving the selector to a direct child immediately displays that child's
  `messages`; Enter then changes the active navigation scope to that child
  without changing which agent's conversation is displayed;
- after Enter navigates into a child, the returned `self_view.messages` continue
  to display the same child's conversation;
- moving the selector to `main` immediately displays the TUI's existing main
  conversation history; Enter returns to the root scope;
- rebuilding or refreshing a scoped view re-resolves the selected agent and
  updates the conversation atomically, preventing a selector/chat mismatch;
- if the selected agent disappears from a refreshed view, selection falls back
  deterministically to the current scoped agent, or to `main` at the root.

The main conversation continues to come from the TUI's existing root
conversation history. Scoped self and child conversations come from their
daemon projections. An agent with no captured messages renders the existing
waiting/empty state; messages from another agent are never used as a fallback.

## Navigation and Refresh

`AgentNavigationState` owns:

- the root frame;
- the current frame;
- a bounded back stack;
- the capability used to enter each non-root frame;
- breadcrumb labels for the trusted path already traversed.

Root polling updates the cached root frame. It replaces the displayed tree only
when the root frame is active. While a child frame is active, the TUI refreshes
that frame through its navigation capability and leaves root updates cached.

The breadcrumb is derived only from UI navigation history. It is display-only
and must not be appended to any agent transcript or model message.

`Backspace` pops one frame. Selecting `main` clears the back stack and restores
the cached root frame. Both operations rebuild selector and conversation state
from the restored frame.

## Capability Resolution

The daemon will resolve a navigation token through the capability service's
stored grant instead of guessing the target by scanning root children. The
resolution verifies:

- viewer identity;
- session;
- `Navigate` operation;
- generation;
- expiration.

After verification, the trusted daemon loads the bound target's local view.
The raw target ID remains unavailable as a client-controlled navigation input.
Fresh direct-child capabilities are issued for the returned scope, allowing
`main -> child -> grandchild` traversal without widening visibility.

Every invalid, expired, cross-viewer, cross-session, or wrong-operation token
continues to produce the same not-visible response.

## Error Handling

- A failed navigation leaves the current frame, selection, and conversation
  unchanged and records a diagnostic warning without logging the capability.
- A failed refresh preserves the last trusted frame and retries on the normal
  polling cadence.
- An expired viewer token follows the existing one-time viewer recreation and
  request retry behavior.
- An expired navigation capability does not fall back to raw-ID navigation.
  The UI keeps the cached frame and requires a valid refreshed path before
  descending further.

## Testing

### Component tests

- Root selector is `main + direct children`, without a duplicated root.
- Child selector is `main + self + direct children`.
- A self node with children is never hidden as a grouping node.
- Selecting each entry displays that entry's messages.
- Refreshing a view keeps selector and conversation aligned.
- A missing selected child falls back deterministically.
- Breadcrumbs reflect the current navigation stack.

### Daemon and integration tests

- A self projection includes its own messages and display metadata.
- Root-to-child and child-to-grandchild capability navigation both succeed.
- Grandchildren never appear in the root local view.
- Wrong-viewer, wrong-session, expired, and forged capabilities remain denied.
- Root polling does not replace an active child frame.
- Child-scope refresh updates the selected child's conversation.
- Back navigation restores the previous trusted frame and its selected chat.

### Verification

Run targeted TUI, daemon, capability, and strict-isolation tests first, followed
by formatting, Clippy with warnings denied, and the complete test suite.

## Security Impact

This change affects capability-based hierarchy navigation. It does not widen
the information visible in a local view. Capability resolution reads the target
from the server-side stored grant only after all authority dimensions are
verified. Display metadata remains inside the trusted daemon-to-TUI boundary
and is never included in model context.
