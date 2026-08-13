"""Tests for repository-profile detection in the DeepSWE agent driver."""

import json
import sys
import types
import unittest
from pathlib import Path


def _install_pier_import_stubs():
    """Provide the unavailable Pier dependency needed to import the driver."""
    module_names = (
        "pier",
        "pier.agents",
        "pier.agents.installed",
        "pier.agents.network",
        "pier.environments",
        "pier.models",
        "pier.models.agent",
        "pier.utils",
        "pier.agents.installed.base",
        "pier.environments.base",
        "pier.models.agent.context",
        "pier.models.agent.install",
        "pier.models.agent.network",
        "pier.utils.logger",
    )
    for name in module_names:
        sys.modules.setdefault(name, types.ModuleType(name))

    installed_base = sys.modules["pier.agents.installed.base"]
    installed_base.BaseInstalledAgent = type("BaseInstalledAgent", (), {})
    installed_base.CliFlag = type("CliFlag", (), {})
    installed_base.with_prompt_template = lambda function: function
    sys.modules["pier.agents.network"].allowlist_from_urls = lambda *args, **kwargs: None
    sys.modules["pier.environments.base"].BaseEnvironment = type(
        "BaseEnvironment", (), {}
    )
    sys.modules["pier.models.agent.context"].AgentContext = type(
        "AgentContext", (), {}
    )
    install = sys.modules["pier.models.agent.install"]
    install.AgentInstallSpec = type("AgentInstallSpec", (), {})
    install.InstallStep = type("InstallStep", (), {})
    sys.modules["pier.models.agent.network"].NetworkAllowlist = type(
        "NetworkAllowlist", (), {}
    )
    sys.modules["pier.utils.logger"].logger = types.SimpleNamespace(debug=lambda *_: None)


_install_pier_import_stubs()
sys.path.insert(0, str(Path(__file__).parent))
import wgenty_code_agent as driver


class ProjectProfileTests(unittest.TestCase):
    def test_cargo_manifest_selects_rust_test_command(self):
        profile = driver.detect_project_profile(
            {"Cargo.toml": "[package]\\nname = 'demo'"}
        )
        self.assertEqual(profile.ecosystem, "rust")
        self.assertEqual(profile.test_command, "cargo test")

    def test_go_manifest_beats_package_json(self):
        profile = driver.detect_project_profile(
            {"go.mod": "module demo", "package.json": "{}"}
        )
        self.assertEqual(profile.ecosystem, "go")
        self.assertEqual(profile.test_command, "go test ./...")

    def test_unknown_repository_does_not_invent_a_test_command(self):
        profile = driver.detect_project_profile({"README.md": "demo"})
        self.assertEqual(profile.ecosystem, "generic")
        self.assertIsNone(profile.test_command)

    def test_python_marker_selects_pytest(self):
        profile = driver.detect_project_profile(
            {"pyproject.toml": "[project]\\nname = 'demo'"}
        )
        self.assertEqual(profile.ecosystem, "python")
        self.assertEqual(profile.test_command, "pytest")

    def test_package_json_test_script_selects_npm_test(self):
        profile = driver.detect_project_profile(
            {"package.json": '{"scripts":{"test":"vitest run"}}'}
        )
        self.assertEqual(profile.test_command, "npm test --")

    def test_malformed_package_json_has_no_test_command(self):
        profile = driver.detect_project_profile({"package.json": "{"})
        self.assertEqual(profile.ecosystem, "javascript")
        self.assertIsNone(profile.test_command)


class DeepSWEInstructionTests(unittest.TestCase):
    def test_rust_prompt_uses_detected_broad_test_command(self):
        text = driver.render_deepswe_instructions(
            driver.ProjectProfile(
                "rust", "cargo test", "cargo test TestName", "Cargo.toml"
            )
        )
        self.assertIn("cargo test", text)
        self.assertIn("cargo test TestName", text)
        self.assertNotIn("npx vitest run", text)

    def test_go_prompt_renders_focused_package_and_test_template(self):
        text = driver.render_deepswe_instructions(
            driver.ProjectProfile(
                "go", "go test ./...", "go test ./path -run TestName", "go.mod"
            )
        )

        self.assertIn("go test ./path -run TestName", text)
        self.assertIn("concrete failing test and package", text)

    def test_python_prompt_renders_focused_node_template(self):
        text = driver.render_deepswe_instructions(
            driver.ProjectProfile(
                "python", "pytest", "pytest path::test_name", "pyproject.toml"
            )
        )

        self.assertIn("pytest path::test_name", text)
        self.assertIn("concrete failing test", text)

    def test_javascript_prompt_routes_focused_arguments_through_test_script(self):
        text = driver.render_deepswe_instructions(
            driver.ProjectProfile(
                "javascript",
                "npm test --",
                "npm test -- <runner-specific focused arguments>",
                "package.json",
            )
        )

        self.assertIn("npm test -- <runner-specific focused arguments>", text)
        self.assertIn("package.json", text)
        self.assertIn("only after identifying the test runner's syntax", text)

    def test_javascript_without_test_script_renders_safe_focused_fallback(self):
        text = driver.render_deepswe_instructions(
            driver.ProjectProfile(
                "javascript",
                None,
                "npm test -- <runner-specific focused arguments>",
                "package.json",
            )
        )

        self.assertIn("package.json does not declare a usable test script", text)
        self.assertIn("Inspect package.json and project or CI guidance", text)
        self.assertNotIn("`npm test --`", text)

    def test_prompt_requires_failure_convergence_and_final_evidence(self):
        text = driver.render_deepswe_instructions(driver.ProjectProfile.generic())
        self.assertIn("rerun that same focused test", text)
        self.assertIn("must not claim tests pass without executing them", text)

    def test_fallback_agents_guidance_uses_no_invented_test_command(self):
        text = driver.render_fallback_agents_md(driver.ProjectProfile.generic())

        self.assertIn("Inspect the repository and CI guidance", text)
        self.assertNotIn("npx vitest run", text)
        self.assertNotIn("full test suite", text)


class EvalSettingsTests(unittest.TestCase):
    def test_settings_use_the_profile_specific_developer_instructions(self):
        settings = driver.build_eval_settings(
            "deepseek-v4-pro",
            "https://api.example",
            driver.ProjectProfile("rust", "cargo test", "cargo test", "Cargo.toml"),
        )

        self.assertEqual(settings["models"]["main"]["name"], "deepseek-v4-pro")
        self.assertIn("cargo test", settings["prompt"]["developer_instructions"])


class PierRunBoundaryTests(unittest.IsolatedAsyncioTestCase):
    async def test_run_collects_container_markers_before_building_settings(self):
        agent = driver.WgentyCodeAgent(max_rounds=1)
        agent.model_name = "deepseek-v4-pro"
        agent._get_env = lambda _key: None
        agent.build_process_env = lambda _base: {}
        calls = []

        async def fake_exec_as_agent(
            environment,
            command,
            env=None,
            cwd=None,
            timeout_sec=None,
        ):
            calls.append(
                {
                    "environment": environment,
                    "command": command,
                    "env": env,
                    "cwd": cwd,
                    "timeout_sec": timeout_sec,
                }
            )
            if command.startswith("for marker in "):
                return types.SimpleNamespace(
                    stdout="marker:go.mod\n",
                    stderr="",
                    return_code=0,
                )
            return types.SimpleNamespace(stdout="", stderr="", return_code=0)

        agent.exec_as_agent = fake_exec_as_agent
        environment = object()

        await agent.run("implement the task", environment, object())

        self.assertIs(calls[0]["environment"], environment)
        self.assertEqual(calls[0]["cwd"], "/app")
        self.assertEqual(calls[0]["timeout_sec"], 10)
        self.assertIn("head -c 65536 package.json", calls[0]["command"])
        self.assertNotIn("README", calls[0]["command"])
        settings_write = next(
            call["command"]
            for call in calls
            if '"$HOME/.wgenty-code/settings.json"' in call["command"]
        )
        settings_payload = settings_write[
            settings_write.index("{") : settings_write.rindex("\nWGENTY_SETTINGS_")
        ]
        settings = json.loads(settings_payload)
        instructions = settings["prompt"]["developer_instructions"]
        self.assertIn("go test ./path -run TestName", instructions)

    async def test_run_degrades_failed_container_probe_to_generic_profile(self):
        agent = driver.WgentyCodeAgent(max_rounds=1)
        agent.model_name = "deepseek-v4-pro"
        agent._get_env = lambda _key: None
        agent.build_process_env = lambda _base: {}
        calls = []

        async def fake_exec_as_agent(
            environment,
            command,
            env=None,
            cwd=None,
            timeout_sec=None,
        ):
            calls.append(command)
            if len(calls) == 1:
                raise RuntimeError("container probe unavailable")
            return types.SimpleNamespace(stdout="", stderr="", return_code=0)

        agent.exec_as_agent = fake_exec_as_agent

        await agent.run("implement the task", object(), object())

        settings_write = next(
            command
            for command in calls
            if '"$HOME/.wgenty-code/settings.json"' in command
        )
        settings_payload = settings_write[
            settings_write.index("{") : settings_write.rindex("\nWGENTY_SETTINGS_")
        ]
        instructions = json.loads(settings_payload)["prompt"][
            "developer_instructions"
        ]
        self.assertIn("No supported repository marker was detected", instructions)
        self.assertIn("do not invent one", instructions)
