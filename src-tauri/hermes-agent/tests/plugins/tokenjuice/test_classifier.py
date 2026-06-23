"""Unit tests for TokenJuice tool classifier."""

from plugins.tokenjuice.classifier import classify


class TestClassify:
    def test_terminal_shell_primary(self):
        tags = classify(tool_name="terminal", output="some output")
        assert "shell" in tags
        assert "generic" in tags

    def test_grep_sub_classification(self):
        tags = classify(
            tool_name="terminal",
            args={"command": "grep -r 'pattern' src/"},
            output="result",
        )
        assert "shell" in tags
        assert "grep" in tags

    def test_git_sub_classification(self):
        tags = classify(
            tool_name="terminal",
            args={"command": "git diff HEAD~1"},
            output="diff result",
        )
        assert "shell" in tags
        assert "git" in tags

    def test_npm_install_sub_classification(self):
        tags = classify(
            tool_name="terminal",
            args={"command": "npm install express"},
            output="installing...",
        )
        assert "shell" in tags
        assert "package_install" in tags

    def test_cargo_sub_classification(self):
        tags = classify(
            tool_name="terminal",
            args={"command": "cargo update"},
            output="updating...",
        )
        assert "shell" in tags
        assert "package_install" in tags

    def test_docker_sub_classification(self):
        tags = classify(
            tool_name="terminal",
            args={"command": "docker build -t app ."},
            output="building...",
        )
        assert "shell" in tags
        assert "docker" in tags

    def test_test_run_sub_classification(self):
        tags = classify(
            tool_name="terminal",
            args={"command": "pytest tests/"},
            output="test output",
        )
        assert "shell" in tags
        assert "test_run" in tags

    def test_error_output_detection(self):
        tags = classify(
            tool_name="terminal",
            args={"command": "some command"},
            output="Traceback (most recent call last):\nError occurred",
        )
        assert "error_output" in tags

    def test_read_file_tool(self):
        tags = classify(tool_name="read_file", output="file content")
        assert "file_read" in tags

    def test_web_search_tool(self):
        tags = classify(tool_name="web_search", output="search results")
        assert "web" in tags

    def test_unknown_tool_generic_only(self):
        tags = classify(tool_name="unknown_tool", output="some output")
        assert "generic" in tags
        assert len(tags) == 1

    def test_empty_args(self):
        tags = classify(tool_name="terminal", args={}, output="output")
        assert "shell" in tags

    def test_pip_install(self):
        tags = classify(
            tool_name="terminal",
            args={"command": "pip install requests"},
            output="installing...",
        )
        assert "package_install" in tags
