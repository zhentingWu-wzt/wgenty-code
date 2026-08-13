# DeepSWE Evaluation Profile Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the DeepSWE Pier driver generate repository-aware, verification-focused instructions without affecting normal Wgenty Code behavior.

**Architecture:** Keep all behavior in `eval/deepswe/wgenty_code_agent.py`. A pure `ProjectProfile` detector reads bounded repository markers and provides ecosystem-specific command guidance; a pure prompt renderer incorporates that guidance into the temporary evaluation settings. The asynchronous driver only invokes these helpers and preserves the existing Pier lifecycle.

**Tech Stack:** Python 3 standard library (`dataclasses`, `json`, `pathlib`, `unittest`), Pier installed-agent driver.

## Global Constraints

- Modify only the DeepSWE evaluation driver and its tests; do not change normal CLI/runtime behavior.
- Do not add Python runtime dependencies.
- Do not run an automatic full test suite from the driver in this phase.
- Unknown or malformed repository metadata must degrade to safe generic guidance, never fail a trial.
- Preserve the existing base-commit, CodeGraph initialization, commit, and `model.patch` protocol.

---

### Task 1: Add pure repository-profile detection

**Files:**

- Modify: `eval/deepswe/wgenty_code_agent.py`
- Create: `eval/deepswe/test_wgenty_code_agent.py`

**Interfaces:**

- `ProjectProfile(ecosystem: str, test_command: str | None, focused_test_hint: str, package_hint: str)` is an immutable value object.
- `detect_project_profile(repo_root: Path) -> ProjectProfile` is a pure, non-throwing detector.
- `ProjectProfile.generic()` represents an unknown repository and has `test_command is None`.

- [ ] **Step 1: Write failing detector tests**

```python
class ProjectProfileTests(unittest.TestCase):
    def test_cargo_manifest_selects_rust_test_command(self):
        with temporary_repo({"Cargo.toml": "[package]\\nname = 'demo'"}) as root:
            profile = driver.detect_project_profile(root)
        self.assertEqual(profile.ecosystem, "rust")
        self.assertEqual(profile.test_command, "cargo test")

    def test_go_manifest_beats_package_json(self):
        with temporary_repo({"go.mod": "module demo", "package.json": "{}"}) as root:
            profile = driver.detect_project_profile(root)
        self.assertEqual(profile.ecosystem, "go")
        self.assertEqual(profile.test_command, "go test ./...")

    def test_unknown_repository_does_not_invent_a_test_command(self):
        with temporary_repo({"README.md": "demo"}) as root:
            profile = driver.detect_project_profile(root)
        self.assertEqual(profile.ecosystem, "generic")
        self.assertIsNone(profile.test_command)
```

- [ ] **Step 2: Run the detector tests and observe the missing API**

Run: `python3 -m unittest eval/deepswe/test_wgenty_code_agent.py -v`

Expected: FAIL because `ProjectProfile` and `detect_project_profile` are absent.

- [ ] **Step 3: Implement the minimal profile model and detector**

Use `@dataclass(frozen=True)` and `pathlib.Path`. Detect, in order: `Cargo.toml`, `go.mod`, Python markers (`pyproject.toml`, `pytest.ini`, `setup.cfg`, `tox.ini`), then `package.json`; otherwise return `generic`. Give Rust `cargo test`, Go `go test ./...`, Python `pytest`, and JavaScript `npm test --` only when `package.json` parses to an object containing a non-empty `scripts.test`; malformed JSON must return a generic JavaScript profile with no broad command.

- [ ] **Step 4: Extend coverage for Python, JavaScript, and malformed metadata**

```python
def test_package_json_test_script_selects_npm_test(self):
    with temporary_repo({"package.json": '{"scripts":{"test":"vitest run"}}'}) as root:
        self.assertEqual(driver.detect_project_profile(root).test_command, "npm test --")

def test_malformed_package_json_has_no_test_command(self):
    with temporary_repo({"package.json": "{"}) as root:
        self.assertIsNone(driver.detect_project_profile(root).test_command)
```

- [ ] **Step 5: Verify Task 1**

Run: `python3 -m unittest eval/deepswe/test_wgenty_code_agent.py -v`

Expected: all profile tests pass.

### Task 2: Render a DeepSWE-specific developer instruction block

**Files:**

- Modify: `eval/deepswe/wgenty_code_agent.py`
- Modify: `eval/deepswe/test_wgenty_code_agent.py`

**Interfaces:**

- `render_deepswe_instructions(profile: ProjectProfile) -> str` returns the complete developer instruction string used in generated settings.
- The renderer embeds a known `profile.test_command` exactly once; generic profiles state that the agent must inspect repository/CI guidance rather than invent a command.

- [ ] **Step 1: Write failing prompt-rendering tests**

```python
def test_rust_prompt_uses_detected_broad_test_command(self):
    text = driver.render_deepswe_instructions(
        driver.ProjectProfile.rust()
    )
    self.assertIn("cargo test", text)
    self.assertNotIn("npx vitest run", text)

def test_prompt_requires_failure_convergence_and_final_evidence(self):
    text = driver.render_deepswe_instructions(driver.ProjectProfile.generic())
    self.assertIn("rerun that same focused test", text)
    self.assertIn("must not claim tests pass without executing them", text)
```

- [ ] **Step 2: Run the focused renderer tests and observe the missing function**

Run: `python3 -m unittest eval/deepswe/test_wgenty_code_agent.py -v`

Expected: FAIL because `render_deepswe_instructions` is absent.

- [ ] **Step 3: Implement the renderer**

Render a concise workflow: spend at most 5–15 rounds locating code/tests; implement promptly; use work-graph tools only for independent phases or multi-subsystem changes; test after meaningful changes; on failure extract the test/assertion, fix behavior rather than expected-behavior tests, then rerun that focused test; finally run the known broad command or inspect existing project/CI guidance when unknown. Include the no-cloning/no-user-questions rules and final commit instruction from the existing driver.

- [ ] **Step 4: Verify Task 2**

Run: `python3 -m unittest eval/deepswe/test_wgenty_code_agent.py -v`

Expected: all detector and renderer tests pass.

### Task 3: Wire the profile into the Pier driver without changing artifact semantics

**Files:**

- Modify: `eval/deepswe/wgenty_code_agent.py`
- Modify: `eval/deepswe/test_wgenty_code_agent.py`
- Modify: `eval/deepswe/README.md`

**Interfaces:**

- `WgentyCodeAgent.run` probes `/app` (the DeepSWE repository root) after its environment is created and calls `render_deepswe_instructions(detect_project_profile(Path('/app')))`, assigning the result to `settings["prompt"]["developer_instructions"]`.
- Base commit capture, CodeGraph initialization, agent invocation, commit, and patch diff commands remain textually and behaviorally unchanged.

- [ ] **Step 1: Write a failing settings-construction test**

Extract the settings construction into a pure helper if needed so the test can assert the profile instruction is placed under `settings["prompt"]["developer_instructions"]` without constructing a Pier environment.

```python
def test_settings_use_the_profile_specific_developer_instructions(self):
    settings = driver.build_eval_settings("deepseek-v4-pro", "https://api.example", driver.ProjectProfile.rust())
    self.assertEqual(settings["models"]["main"]["name"], "deepseek-v4-pro")
    self.assertIn("cargo test", settings["prompt"]["developer_instructions"])
```

- [ ] **Step 2: Run the focused test and observe the missing helper**

Run: `python3 -m unittest eval/deepswe/test_wgenty_code_agent.py -v`

Expected: FAIL because `build_eval_settings` is absent.

- [ ] **Step 3: Extract settings construction and integrate it**

Implement `build_eval_settings(model_name, base_url, profile)` with the existing models/transport values unchanged. In `run`, call `detect_project_profile(Path("/app"))` immediately before serializing settings and pass the profile to the helper. Do not add a driver-side test invocation or change post-run Git commands.

- [ ] **Step 4: Document the behavior**

Update the evaluation README Configuration section to describe automatic repository-profile detection, its supported ecosystems, and the safe unknown-repository fallback. Remove any implication that Vitest is universally configured.

- [ ] **Step 5: Verify Task 3**

Run: `python3 -m unittest eval/deepswe/test_wgenty_code_agent.py -v && python3 -m py_compile eval/deepswe/wgenty_code_agent.py`

Expected: the test suite passes and the driver compiles.

### Task 4: Run repository-level validation and review the diff

**Files:**

- Modify: `eval/deepswe/wgenty_code_agent.py`
- Modify: `eval/deepswe/test_wgenty_code_agent.py`
- Modify: `eval/deepswe/README.md`
- Create: `docs/superpowers/specs/2026-08-13-deepswe-evaluation-profile-design.md`
- Create: `docs/superpowers/plans/2026-08-13-deepswe-evaluation-profile.md`

**Interfaces:**

- No new dependencies, CLI flags, artifact paths, or changes to `model.patch` collection.

- [ ] **Step 1: Run the complete new Python test suite**

Run: `python3 -m unittest eval/deepswe/test_wgenty_code_agent.py -v`

Expected: all tests pass.

- [ ] **Step 2: Syntax-check changed scripts**

Run: `python3 -m py_compile eval/deepswe/wgenty_code_agent.py eval/deepswe/analyze_results.py`

Expected: exit status 0.

- [ ] **Step 3: Inspect only the intended diff**

Run: `git diff --check && git diff -- eval/deepswe/wgenty_code_agent.py eval/deepswe/test_wgenty_code_agent.py eval/deepswe/README.md docs/superpowers/specs/2026-08-13-deepswe-evaluation-profile-design.md docs/superpowers/plans/2026-08-13-deepswe-evaluation-profile.md`

Expected: no whitespace errors; no changes to runtime or Pier artifact protocol.

- [ ] **Step 4: Commit only on explicit user request**

Do not commit as part of this task. If requested, stage only the files listed above and use `feat(eval): add repository-aware DeepSWE profile`.
