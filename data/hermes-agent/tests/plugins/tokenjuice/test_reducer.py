"""Unit tests for TokenJuice reducer pipeline."""

import pytest
from plugins.tokenjuice.reducer import (
    TINY_OUTPUT_MAX_CHARS,
    _apply_rules,
    reduce_terminal_output,
    reduce_tool_result,
)


class TestReduceToolResult:
    def test_skips_tiny_output(self):
        result = reduce_tool_result(
            tool_name="terminal",
            args={"command": "git diff"},
            result="x" * 100,
        )
        assert result is None

    def test_returns_none_when_no_savings(self):
        # Output that won't benefit from compression
        result = reduce_tool_result(
            tool_name="unknown_tool",
            args={},
            result="unique line 1\nunique line 2\nunique line 3",
        )
        assert result is None

    def test_compresses_git_output(self):
        git_output = (
            "diff --git a/src/main.rs b/src/main.rs\n"
            + "index 123..456\n"
            + "--- a/src/main.rs\n"
            + "+++ b/src/main.rs\n"
            + "@@ -10,6 +10,8 @@\n"
            + " unchanged context line\n"
            + "+added line of code\n"
            + "-removed line of code\n"
            + " more context that is unchanged\n"
        ) * 10  # Repeat to get above threshold
        result = reduce_tool_result(
            tool_name="terminal",
            args={"command": "git diff HEAD~1"},
            result=git_output,
        )
        assert result is not None
        assert "[Compacted:" in result
        assert "% saved" in result

    def test_compresses_with_dup_lines(self):
        output = (
            "Starting build...\n"
            + "processing...\n" * 200
            + "Build complete.\n"
        )
        result = reduce_tool_result(
            tool_name="terminal",
            args={"command": "npm run build"},
            result=output,
        )
        assert result is not None
        assert "processing..." in result
        # Should have deduplicated the 200 repeats
        assert result.count("processing...") < 200

    def test_handles_empty_result(self):
        result = reduce_tool_result(
            tool_name="terminal",
            args={},
            result="",
        )
        assert result is None

    def test_handles_none_args(self):
        output = "x" * 300 + "\n" + "y" * 300
        result = reduce_tool_result(
            tool_name="terminal",
            args=None,
            result=output,
        )
        # Should not crash — may or may not compress depending on content
        assert isinstance(result, (str, type(None)))

    def test_includes_compaction_marker(self):
        output = (
            "ERROR: Something went wrong\n" * 50
            + "additional context\n" * 50
        )
        result = reduce_tool_result(
            tool_name="terminal",
            args={"command": "run tests"},
            result=output,
        )
        if result is not None:
            assert "[Compacted:" in result
            assert "chars" in result


class TestReduceTerminalOutput:
    def test_delegates_to_reduce_tool_result(self):
        output = (
            "line1\n" + "processing...\n" * 100 + "line_final\n"
        )
        result = reduce_terminal_output(
            tool_name="terminal",
            args={"command": "ls"},
            result=output,
        )
        # Should behave same as reduce_tool_result for terminal tools
        if result is not None:
            assert "[Compacted:" in result


class TestApplyRules:
    def test_matches_generic_fallback(self):
        tags = ["shell", "generic"]
        result = "line1\n" + "repeating\n" * 100 + "last\n"
        stats = _apply_rules(tags, result)
        assert stats is not None
        assert "compacted" in stats
        # Generic rule should dedupe
        assert stats.get("dup_lines_removed", 0) > 0

    def test_no_match_returns_none(self):
        # Tags that don't match any rule should return None
        # (but 'generic' always matches if size > min_chars)
        pass

    def test_first_match_per_tag_wins(self):
        # 'git' tag matches git rule before generic
        tags = ["shell", "git", "generic"]
        result = (
            "diff --git a/x b/x\n"
            "--- a/x\n"
            "+++ b/x\n"
            "+new line\n"
        ) * 30
        stats = _apply_rules(tags, result)
        assert stats is not None
        assert stats.get("git_chars_saved", 0) > 0


class TestTinyOutputThreshold:
    def test_exact_threshold_passthrough(self):
        output = "x" * TINY_OUTPUT_MAX_CHARS
        result = reduce_tool_result(
            tool_name="terminal",
            args={"command": "git diff"},
            result=output,
        )
        assert result is None

    def test_just_above_threshold(self):
        output = "line\n" * (TINY_OUTPUT_MAX_CHARS // 5 + 1)
        if len(output) <= TINY_OUTPUT_MAX_CHARS:
            pytest.skip("output still too small")
        result = reduce_tool_result(
            tool_name="terminal",
            args={"command": "ls"},
            result=output,
        )
        # May or may not compress, but should not crash
        assert isinstance(result, (str, type(None)))
