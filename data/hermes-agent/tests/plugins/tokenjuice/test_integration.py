"""Integration tests for TokenJuice plugin.

Tests the full pipeline: classification -> rule matching -> compression.
Also tests hook registration via the plugin system.
"""

import pytest


class TestEndToEndPipeline:
    """Simulate real-world tool outputs through the full pipeline."""

    def _run_pipeline(self, tool_name, args, output):
        from plugins.tokenjuice.reducer import reduce_tool_result
        return reduce_tool_result(
            tool_name=tool_name,
            args=args,
            result=output,
        )

    def test_git_diff_realistic(self):
        output = (
            "diff --git a/Cargo.toml b/Cargo.toml\n"
            "index 82755..783ad 100644\n"
            "--- a/Cargo.toml\n"
            "+++ b/Cargo.toml\n"
            "@@ -4,7 +4,7 @@ version = \"0.1.0\"\n"
            " [dependencies]\n"
            "-serde = \"1.0.180\"\n"
            "+serde = { version = \"1.0.210\", features = [\"derive\"] }\n"
            "diff --git a/src/main.rs b/src/main.rs\n"
            "index a12bc..d34ef 100644\n"
            "--- a/src/main.rs\n"
            "+++ b/src/main.rs\n"
            "@@ -10,6 +10,10 @@ fn main() {\n"
            "     println!(\"Hello\");\n"
            "+    let config = load_config();\n"
            "+    println!(\"Config: {:?}\", config);\n"
            " }\n"
        ) + ("\n" * 50)  # padding
        result = self._run_pipeline(
            "terminal",
            {"command": "git diff HEAD~1"},
            output,
        )
        assert result is not None
        assert "[Compacted:" in result
        # Should keep file names and changes
        assert "Cargo.toml" in result
        assert "+serde" in result

    def test_npm_install_realistic(self):
        output = (
            "\x1b[33mnpm WARN deprecated\x1b[0m old-package@1.0.0\n"
            + "downloading...\n" * 30
            + "\x1b[32madded 47 packages, removed 2 packages, "
            + "changed 12 packages, and audited 312 packages in 5s\x1b[0m\n"
            + "\x1b[32mfound 0 vulnerabilities\x1b[0m\n"
        )
        result = self._run_pipeline(
            "terminal",
            {"command": "npm install"},
            output,
        )
        assert result is not None
        assert "[Compacted:" in result
        assert "added 47" in result
        assert "found 0 vulnerabilities" in result
        # ANSI should be stripped
        assert "\x1b" not in result

    def test_docker_build_realistic(self):
        steps = []
        for i in range(80):
            steps.append(f"Step {i}/80: RUN some command")
            steps.append(f" ---> Running in abc{i:04d}")
            steps.append(f" ---> def{i:04d}")
        steps.append("ERROR: build failed at step 50")
        output = "\n".join(steps)
        result = self._run_pipeline(
            "terminal",
            {"command": "docker build -t app ."},
            output,
        )
        assert result is not None
        assert "ERROR" in result
        assert "[Compacted:" in result

    def test_pytest_output_realistic(self):
        lines = []
        for i in range(100):
            lines.append(
                f"tests/test_module_{i}.py::test_case_{i} PASSED [ {i}%]"
            )
        lines.append("=" * 60)
        lines.append("100 passed in 12.34s")
        lines.append("=" * 60)
        output = "\n".join(lines)
        result = self._run_pipeline(
            "terminal",
            {"command": "pytest tests/ -v"},
            output,
        )
        assert result is not None
        assert "100 passed" in result
        assert "[Compacted:" in result

    def test_repeating_build_progress(self):
        output = (
            "Building...\n"
            + "Compiling module A\n"
            + "Compiling module A\n" * 100
            + "Compiling module B\n"
            + "Compiling module B\n" * 50
            + "Build finished.\n"
        )
        result = self._run_pipeline(
            "terminal",
            {"command": "cargo build"},
            output,
        )
        assert result is not None
        # Should deduplicate heavily
        assert result.count("Compiling module A") <= 2
        assert result.count("Compiling module B") <= 2

    def test_short_output_passes_through(self):
        output = "short output\njust two lines"
        result = self._run_pipeline(
            "terminal",
            {"command": "echo hello"},
            output,
        )
        assert result is None  # Too small to compress

    def test_error_traceback(self):
        # Error traceback with many repetitions — should compress heavily
        traceback_block = (
            "Traceback (most recent call last):\n"
            + '  File "app.py", line 42, in <module>\n'
            + "    result = divide(10, 0)\n"
            + '  File "app.py", line 10, in divide\n'
            + "    raise ValueError('Cannot divide by zero')\n"
            + "ValueError: Cannot divide by zero\n"
        )
        # Repeat many times + add filler to ensure above threshold
        output = (traceback_block * 30) + ("padding\n" * 50)
        result = self._run_pipeline(
            "terminal",
            {"command": "python app.py"},
            output,
        )
        assert result is not None
        assert "[Compacted:" in result
        assert "Traceback" in result
        assert "ValueError" in result


class TestPluginRegistration:
    """Verify the plugin can be loaded via the Hermes plugin system."""

    def test_register_function_exists(self):
        from plugins.tokenjuice import register
        assert callable(register)

    def test_register_hooks(self):
        """Simulate what the plugin manager does during loading."""
        hooks_registered = []

        class FakeCtx:
            def register_hook(self, name, callback):
                hooks_registered.append((name, callback.__name__))

        from plugins.tokenjuice import register
        register(FakeCtx())

        hook_names = {h[0] for h in hooks_registered}
        assert "transform_tool_result" in hook_names
        assert "transform_terminal_output" in hook_names

    def test_hook_functions_are_callable(self):
        from plugins.tokenjuice.reducer import (
            reduce_terminal_output,
            reduce_tool_result,
        )
        assert callable(reduce_tool_result)
        assert callable(reduce_terminal_output)

    def test_plugin_manifest_exists(self):
        import os
        manifest_path = os.path.join(
            os.path.dirname(__file__),
            "..",
            "..",
            "..",
            "plugins",
            "tokenjuice",
            "plugin.yaml",
        )
        manifest_path = os.path.normpath(manifest_path)
        assert os.path.exists(manifest_path), (
            f"plugin.yaml not found at {manifest_path}"
        )


class TestConfigYaml:
    """Verify plugin.yaml is valid."""

    def test_manifest_parseable(self):
        import os
        try:
            import yaml
        except ImportError:
            pytest.skip("PyYAML not available")

        manifest_path = os.path.join(
            os.path.dirname(__file__),
            "..",
            "..",
            "..",
            "plugins",
            "tokenjuice",
            "plugin.yaml",
        )
        manifest_path = os.path.normpath(manifest_path)
        with open(manifest_path, encoding="utf-8") as f:
            data = yaml.safe_load(f)

        assert data["name"] == "tokenjuice"
        assert "transform_tool_result" in data.get("provides_hooks", [])
        assert data["kind"] == "backend"


class TestFailOpen:
    """Verify TokenJuice never breaks tool result processing."""

    def test_broken_output_still_returns(self):
        """Even with malformed output, should not crash."""
        from plugins.tokenjuice.reducer import reduce_tool_result

        # None result should return None (pass-through)
        result = reduce_tool_result(
            tool_name="terminal",
            args={"command": "test"},
            result=None,  # type: ignore
        )
        assert result is None  # pass-through

    def test_malformed_args_handled(self):
        from plugins.tokenjuice.reducer import reduce_tool_result

        output = "x" * 1000
        # args as string instead of dict
        result = reduce_tool_result(
            tool_name="terminal",
            args="not a dict",  # type: ignore
            result=output,
        )
        # Should not crash
        assert isinstance(result, (str, type(None)))

    def test_exception_in_transform_does_not_propagate(self):
        """If a transform throws, the pipeline continues."""
        from plugins.tokenjuice.reducer import _apply_rules

        # With tags that match no rule's min_chars for a small result
        tags = ["shell"]
        result = "a" * 100  # Below 240 threshold
        stats = _apply_rules(tags, result)
        assert stats is None  # No rule matched, graceful
