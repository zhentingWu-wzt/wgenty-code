# Design — Welcome Comet Workflow Guide

## Decision

Replace the gray usage-guide block (two `Span::styled` lines colored `Rgb(120, 120, 140)`) with two Comet-focused onboarding lines:

1. A primary call-to-action: "Type /comet to start a spec-driven workflow, or just begin typing."
2. A secondary command reference: "/comet-tweak · small change   /comet-hotfix · urgent fix   /help · commands"

## Color

The original text used gray `Rgb(120, 120, 140)`. The replacement uses a soft lavender `Rgb(150, 140, 185)` so it is no longer "gray" but stays subtle and readable, harmonizing with the existing purple Comet highlight line (`Rgb(160, 140, 200)`).

## Layout

The banner paragraph uses `Constraint::Length(16)`. The swap is line-for-line (2 lines out, 2 lines in), so the constraint stays at 16 and no layout adjustment is needed.

## Risk

Text-only change in a single render function. No logic, state, or API surface affected.
