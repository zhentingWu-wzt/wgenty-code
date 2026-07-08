# Welcome Comet Workflow Guide

## Why

The TUI welcome banner currently shows generic gray usage text ("Type your message and press Enter to start." plus "/help · /plan · /clear · /compact"). This text does not point users toward the Comet spec-driven workflow, which is the primary way to drive structured changes in Wgenty Code. New users landing in the REPL see no hint that `/comet` exists.

## What

Replace the two gray usage-guide lines in `src/tui/components/welcome.rs` with concise Comet workflow onboarding text that tells users how to start the workflow and lists the most relevant entry commands. The existing purple "Comet spec-driven workflow · open → design → build → verify → archive" line stays unchanged.

## Non-goals

- No change to the ASCII logo, gradient, model name line, or layout sizing beyond keeping the banner line count stable.
- No i18n changes (the banner is already hard-coded English/Chinese).
