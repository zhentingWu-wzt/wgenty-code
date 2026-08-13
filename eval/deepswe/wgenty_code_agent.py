"""Pier agent driver for wgenty-code (DeepSWE evaluation).

Registers wgenty-code as an installed agent in Pier's eval harness.
The binary is bind-mounted into the task container at runtime (see job.yaml),
so install_spec only ensures git is available for the post-run commit.

Usage:
    pier run -c job.yaml
    # or via CLI flags (without mounts — binary must already be in the image):
    pier run -p tasks --agent-import-path wgenty_code_agent:WgentyCodeAgent \
        --model deepseek-v4-pro --ae DEEPSEEK_API_KEY=sk-xxx
"""

import base64
import binascii
import json
import shlex
import uuid
from dataclasses import dataclass
from typing import Any, ClassVar, Mapping

from pier.agents.installed.base import (
    BaseInstalledAgent,
    CliFlag,
    with_prompt_template,
)
from pier.agents.network import allowlist_from_urls
from pier.environments.base import BaseEnvironment
from pier.models.agent.context import AgentContext
from pier.models.agent.install import AgentInstallSpec, InstallStep
from pier.models.agent.network import NetworkAllowlist
from pier.utils.logger import logger

# Common LLM API domains — used as the default network allowlist.
_DEFAULT_DOMAINS: list[str] = [
    "api.deepseek.com",
    "api.anthropic.com",
    "dashscope.aliyuncs.com",
    "api.openai.com",
    "openrouter.ai",
    "api.mistral.ai",
    "api.groq.com",
    "api.x.ai",
]

# Map model-name prefixes to provider domains for auto-detection.
_PROVIDER_DOMAINS: dict[str, list[str]] = {
    "deepseek": ["api.deepseek.com"],
    "anthropic": ["api.anthropic.com"],
    "claude": ["api.anthropic.com"],
    "dashscope": ["dashscope.aliyuncs.com"],
    "qwen": ["dashscope.aliyuncs.com"],
    "openai": ["api.openai.com"],
    "openrouter": ["openrouter.ai"],
    "mistral": ["api.mistral.ai"],
    "groq": ["api.groq.com"],
    "xai": ["api.x.ai"],
}

_REPOSITORY_MARKERS: tuple[str, ...] = (
    "Cargo.toml",
    "go.mod",
    "pyproject.toml",
    "pytest.ini",
    "setup.cfg",
    "tox.ini",
    "package.json",
)
_PACKAGE_JSON_LIMIT = 65_536


@dataclass(frozen=True)
class ProjectProfile:
    """Detected repository ecosystem and its safest test command."""

    ecosystem: str
    test_command: str | None
    focused_test_hint: str
    package_hint: str

    @classmethod
    def generic(cls) -> "ProjectProfile":
        """Return the conservative profile for an unknown repository."""
        return cls(
            ecosystem="generic",
            test_command=None,
            focused_test_hint="",
            package_hint="",
        )


def detect_project_profile(repository_metadata: Mapping[str, str]) -> ProjectProfile:
    """Classify bounded repository metadata without filesystem access or errors."""
    if "Cargo.toml" in repository_metadata:
        return ProjectProfile(
            "rust", "cargo test", "cargo test TestName", "Cargo.toml"
        )
    if "go.mod" in repository_metadata:
        return ProjectProfile(
            "go", "go test ./...", "go test ./path -run TestName", "go.mod"
        )
    for marker in ("pyproject.toml", "pytest.ini", "setup.cfg", "tox.ini"):
        if marker in repository_metadata:
            return ProjectProfile(
                "python", "pytest", "pytest path::test_name", marker
            )
    if "package.json" in repository_metadata:
        try:
            package = json.loads(repository_metadata["package.json"])
        except (TypeError, json.JSONDecodeError):
            return ProjectProfile(
                "javascript",
                None,
                "npm test -- <runner-specific focused arguments>",
                "package.json",
            )
        test_script = (
            package.get("scripts", {}).get("test")
            if isinstance(package, dict) and isinstance(package.get("scripts"), dict)
            else None
        )
        test_command = (
            "npm test --"
            if isinstance(test_script, str) and test_script.strip()
            else None
        )
        return ProjectProfile(
            "javascript",
            test_command,
            "npm test -- <runner-specific focused arguments>",
            "package.json",
        )
    return ProjectProfile.generic()


def parse_repository_metadata(output: str | None) -> dict[str, str]:
    """Parse the bounded marker protocol emitted inside the task container."""
    metadata: dict[str, str] = {}
    for line in (output or "").splitlines():
        if line.startswith("marker:"):
            marker = line.removeprefix("marker:")
            if marker in _REPOSITORY_MARKERS:
                metadata.setdefault(marker, "")
            continue
        if not line.startswith("content:package.json:"):
            continue
        encoded = line.removeprefix("content:package.json:")
        try:
            package_json = base64.b64decode(encoded, validate=True).decode("utf-8")
        except (binascii.Error, UnicodeDecodeError):
            continue
        metadata["package.json"] = package_json
    return metadata


def repository_metadata_command() -> str:
    """Build the read-only command that emits the bounded marker protocol."""
    markers = " ".join(shlex.quote(marker) for marker in _REPOSITORY_MARKERS)
    return (
        f"for marker in {markers}; do\n"
        "  if test -f \"$marker\"; then printf 'marker:%s\\n' \"$marker\"; fi\n"
        "done\n"
        "if test -f package.json; then\n"
        "  printf 'content:package.json:'\n"
        f"  head -c {_PACKAGE_JSON_LIMIT} package.json | base64 | tr -d '\\n'\n"
        "  printf '\\n'\n"
        "fi"
    )


def _focused_test_guidance(profile: ProjectProfile) -> str:
    """Render safe ecosystem-specific focused-test guidance."""
    if profile.ecosystem == "javascript" and not profile.test_command:
        return (
            "The detected package.json does not declare a usable test script. Inspect "
            "package.json and project or CI guidance before choosing focused or broad "
            "test commands; do not invent one."
        )
    if not profile.focused_test_hint:
        return (
            "Inspect repository and CI guidance before choosing a focused test command; "
            "do not invent one."
        )
    if profile.ecosystem == "go":
        condition = "only after identifying a concrete failing test and package"
    elif profile.ecosystem == "javascript":
        condition = "only after identifying the test runner's syntax and concrete test"
    else:
        condition = "only after identifying a concrete failing test"
    return (
        f"For focused testing, {condition}, specialize this template: "
        f"`{profile.focused_test_hint}`."
    )


def render_deepswe_instructions(profile: ProjectProfile) -> str:
    """Render repository-aware developer instructions for a DeepSWE trial."""
    if profile.test_command:
        final_verification = (
            "Before finishing, run this repository's broad verification command exactly "
            f"once and record its real outcome: `{profile.test_command}`."
        )
    else:
        final_verification = (
            "Before finishing, inspect the repository and CI guidance to select and run "
            "the appropriate broad verification command; do not invent one."
        )

    marker_context = (
        f"The repository profile was detected from `{profile.package_hint}`."
        if profile.package_hint
        else "No supported repository marker was detected."
    )
    focused_testing = _focused_test_guidance(profile)

    return "\n".join(
        (
            "You are solving a software engineering task in an autonomous evaluation sandbox.",
            marker_context,
            "1. Read the task and repository guidance, then spend at most 5-15 rounds "
            "locating the responsible code and existing tests before implementing promptly.",
            "2. Do NOT clone external repositories or fetch external specs. Work with the "
            "code and tests already in the repo.",
            "3. Do NOT ask questions; make reasonable assumptions and proceed.",
            "4. Use work-graph tools only for independent phases or changes spanning "
            "multiple subsystems; do not add work-graph ceremony to a focused change.",
            "5. After each meaningful change, run the most focused relevant test. "
            f"{focused_testing}",
            "6. When a test fails, extract the failing test and assertion, fix the "
            "responsible behavior rather than expected-behavior tests, then rerun that "
            "same focused test.",
            "7. You must not claim tests pass without executing them.",
            f"8. {final_verification}",
            "9. Follow the project's existing coding style and conventions.",
            "10. Commit all changes with git before finishing.",
        )
    )


def render_fallback_agents_md(profile: ProjectProfile) -> str:
    """Render profile-aware guidance for repositories without AGENTS.md."""
    if profile.test_command:
        testing_guidance = (
            f"- Use the repository's broad verification command when appropriate: "
            f"`{profile.test_command}`\n"
        )
    else:
        testing_guidance = (
            "- Inspect the repository and CI guidance to choose an appropriate test command; "
            "do not invent one\n"
        )

    return (
        "# AGENTS.md\n\n"
        "## Testing\n"
        "- Run the most focused relevant test after meaningful changes\n"
        f"- {_focused_test_guidance(profile)}\n"
        f"{testing_guidance}\n"
        "## Code Style\n"
        "- Follow the existing coding style in the repository\n"
        "- Use the same formatting and indentation as surrounding code\n"
        "- Do not add unnecessary dependencies\n\n"
        "## Workflow\n"
        "- Read the task instruction carefully before starting\n"
        "- Make incremental changes and test after each one\n"
        "- Do not modify test files that define expected behavior (test specs)\n"
        "- Commit all changes when done\n"
    )


def build_eval_settings(
    model_name: str, base_url: str, profile: ProjectProfile
) -> dict[str, Any]:
    """Build the wgenty-code settings used for a DeepSWE evaluation trial."""
    return {
        "models": {
            "main": {
                "name": model_name,
                "base_url": base_url,
            },
            "transport": {
                "max_tokens": 16384,
                "timeout": 300,
                "streaming": True,
                "beta_headers": [],
            },
        },
        "prompt": {
            "developer_instructions": render_deepswe_instructions(profile),
        },
    }


def _infer_domains(model_name: str) -> list[str]:
    """Infer LLM API domains from the model name."""
    lower = (model_name or "").lower()
    for prefix, domains in _PROVIDER_DOMAINS.items():
        if prefix in lower:
            return domains
    return []


def _infer_base_url(model_name: str) -> str:
    """Infer the default API base URL from the model name."""
    domains = _infer_domains(model_name)
    if domains:
        return f"https://{domains[0]}"
    return "https://api.deepseek.com"


class WgentyCodeAgent(BaseInstalledAgent):
    """Pier agent driver that runs wgenty-code in autonomous (YOLO) mode.

    The wgenty-code binary is bind-mounted into the task container at runtime
    (configured via ``environment.mounts`` in the job config).  The agent
    receives the task instruction, writes it to a temp file, runs
    ``wgenty-code query --prompt-file ... --yolo --max-rounds N``, then
    commits all changes so the verifier can collect ``model.patch``.
    """

    CLI_FLAGS: ClassVar[list[CliFlag]] = []

    def __init__(
        self,
        max_rounds: int | str = 200,
        binary_path: str = "/usr/local/bin/wgenty-code",
        base_url: str | None = None,
        *args: Any,
        **kwargs: Any,
    ) -> None:
        super().__init__(*args, **kwargs)
        self._max_rounds = int(max_rounds)
        self._binary_path = binary_path
        self._base_url = base_url

    # ------------------------------------------------------------------
    # Identity
    # ------------------------------------------------------------------

    @staticmethod
    def name() -> str:
        return "wgenty-code"

    def get_version_command(self) -> str | None:
        return f"{self._binary_path} --version"

    def parse_version(self, stdout: str) -> str:
        return stdout.strip().splitlines()[-1] if stdout.strip() else "unknown"

    # ------------------------------------------------------------------
    # Install (runs at docker-build time; binary is mounted at runtime)
    # ------------------------------------------------------------------

    def install_spec(self) -> AgentInstallSpec:
        root_run = (
            "if command -v apt-get &>/dev/null; then"
            "  apt-get update && apt-get install -y git;"
            " elif command -v apk &>/dev/null; then"
            "  apk add --no-cache git bash;"
            " elif command -v yum &>/dev/null; then"
            "  yum install -y git;"
            " elif command -v dnf &>/dev/null; then"
            "  dnf install -y git;"
            " else"
            '  echo "Warning: No known package manager found" >&2;'
            " fi"
        )
        return AgentInstallSpec(
            agent_name=self.name(),
            steps=[
                InstallStep(user="root", run=root_run),
                InstallStep(
                    user="root",
                    run="npm install -g @colbymchenry/codegraph 2>&1 || "
                    'echo "codegraph install skipped"',
                ),
            ],
        )

    # ------------------------------------------------------------------
    # Network allowlist (Squid egress proxy)
    # ------------------------------------------------------------------

    def network_allowlist(self) -> NetworkAllowlist:
        urls: list[str] = []

        # Explicit base URL override (highest priority)
        base_url = self._base_url or self._get_env("API_BASE_URL")
        if base_url:
            urls.append(base_url)

        # Provider-specific base URL env vars
        for key in (
            "OPENAI_BASE_URL",
            "OPENAI_API_BASE",
            "ANTHROPIC_BASE_URL",
        ):
            if value := self._get_env(key):
                urls.append(value)

        # Infer domains from model name
        inferred = _infer_domains(self.model_name or "")

        return allowlist_from_urls(
            urls,
            default_domains=inferred if inferred else _DEFAULT_DOMAINS,
        )

    # ------------------------------------------------------------------
    # Run
    # ------------------------------------------------------------------

    @with_prompt_template
    async def run(
        self,
        instruction: str,
        environment: BaseEnvironment,
        context: AgentContext,
    ) -> None:
        model_name = self.model_name or "deepseek-v4-pro"
        base_url = (
            self._base_url
            or self._get_env("API_BASE_URL")
            or _infer_base_url(model_name)
        )

        env = self.build_process_env({})

        # Collect a bounded marker set inside the task container. Classification
        # remains pure and never probes the Pier host filesystem.
        try:
            metadata_result = await self.exec_as_agent(
                environment,
                command=repository_metadata_command(),
                env=env,
                cwd="/app",
                timeout_sec=10,
            )
        except RuntimeError as exc:
            logger.debug(
                "wgenty-code agent: repository metadata probe failed; using generic profile: %s",
                exc,
            )
            repository_metadata: dict[str, str] = {}
        else:
            repository_metadata = parse_repository_metadata(metadata_result.stdout)
        profile = detect_project_profile(repository_metadata)

        # Build a minimal settings.json so wgenty-code uses the right model.
        settings = build_eval_settings(model_name, base_url, profile)
        settings_json = json.dumps(settings, indent=2)

        # --- Step 1: write instruction to a temp file (avoids ARG_MAX) ---
        task_marker = f"WGENTY_TASK_{uuid.uuid4().hex[:8]}"
        write_instruction = (
            f"cat > /tmp/task.md << '{task_marker}'\n"
            f"{instruction}\n"
            f"{task_marker}\n"
        )
        await self.exec_as_agent(
            environment, command=write_instruction, env=env
        )

        # --- Step 2: write minimal settings.json ---
        settings_marker = f"WGENTY_SETTINGS_{uuid.uuid4().hex[:8]}"
        write_settings = (
            f'mkdir -p "$HOME/.wgenty-code"\n'
            f'cat > "$HOME/.wgenty-code/settings.json" << \'{settings_marker}\'\n'
            f"{settings_json}\n"
            f"{settings_marker}\n"
        )
        await self.exec_as_agent(
            environment, command=write_settings, env=env
        )

        # --- Step 3: save base commit + exclude .wgenty-code/ from git ---
        # The verifier needs model.patch = git diff <base> HEAD.  We save the
        # base commit now (before the agent makes changes) and generate the
        # patch after the agent commits.  Also exclude .wgenty-code/ (sessions,
        # memory) from the patch via .git/info/exclude (no tracked file changes).
        exclude_cmd = (
            'git rev-parse HEAD > /tmp/base_commit && '
            'mkdir -p .git/info && '
            'grep -qxF ".wgenty-code/" .git/info/exclude 2>/dev/null || '
            'echo ".wgenty-code/" >> .git/info/exclude'
        )
        await self.exec_as_agent(
            environment, command=exclude_cmd, env=env
        )

        # --- Step 3.5: initialize CodeGraph for code navigation ---
        # If codegraph is installed (from install_spec), build the index so
        # wgenty-code's headless runtime can connect to the CodeGraph MCP
        # server for symbol lookup, call graphs, etc.
        await self.exec_as_agent(
            environment,
            command="codegraph init 2>&1 || echo 'codegraph init skipped'",
            env=env,
        )

        # --- Step 3.6: write AGENTS.md if not already present ---
        # Provides project-level guidance (Layer 9 in prompt assembly) so the
        # agent knows the test framework, coding conventions, etc.  Only
        # writes if the repo doesn't already have one.
        agents_md = render_fallback_agents_md(profile)
        agents_md_cmd = (
            "test -f AGENTS.md || "
            f"cat > AGENTS.md << 'WGENTY_AGENTS_EOF'\n{agents_md}\nWGENTY_AGENTS_EOF"
        )
        await self.exec_as_agent(
            environment, command=agents_md_cmd, env=env
        )

        # --- Step 4: run wgenty-code ---
        # `|| true` ensures a non-zero agent exit (API error, max rounds
        # reached, etc.) does not abort the trial — whatever changes were
        # made are still committed and verified.
        run_cmd = (
            f"mkdir -p /logs/agent /logs/artifacts && "
            f"WGENTY_VERBOSE=1 {shlex.quote(self._binary_path)} query "
            f"--prompt-file /tmp/task.md "
            f"--yolo "
            f"--max-rounds {self._max_rounds} "
            f"2>&1 | tee /logs/agent/wgenty-code.txt /logs/artifacts/agent-output.txt || true"
        )
        await self.exec_as_agent(
            environment, command=run_cmd, env=env
        )

        # --- Step 5: commit so the verifier can collect model.patch ---
        # The verifier diffs <base_commit>..HEAD, so a commit is required.
        commit_cmd = (
            "git add -A && "
            "git -c user.name='wgenty-code' -c user.email='agent@wgenty-code' "
            "commit -q -m 'wgenty-code solution' || true"
        )
        await self.exec_as_agent(
            environment, command=commit_cmd, env=env
        )

        # --- Step 6: create model.patch for the verifier ---
        # Pier's VerifierConfig doesn't parse [[verifier.collect]] from
        # task.toml (pydantic drops unknown fields), so the collect step that
        # would normally generate model.patch never runs.  We generate it here
        # by diffing the saved base commit against HEAD.
        patch_cmd = (
            "mkdir -p /logs/artifacts && "
            "git config --global --add safe.directory /app && "
            'git diff --binary "$(cat /tmp/base_commit)" HEAD '
            "> /logs/artifacts/model.patch || true"
        )
        await self.exec_as_agent(
            environment, command=patch_cmd, env=env
        )

    # ------------------------------------------------------------------
    # Post-run (trajectory metrics — no-op for now)
    # ------------------------------------------------------------------

    def populate_context_post_run(self, context: AgentContext) -> None:
        logger.debug("wgenty-code agent: populate_context_post_run (no-op)")
