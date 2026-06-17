"""Unit tests for TokenJuice transform functions."""

import pytest
from plugins.tokenjuice.transforms import (
    dedupe_adjacent_lines,
    extract_docker_summary,
    extract_error_lines,
    extract_git_summary,
    extract_package_summary,
    keep_first_last_lines,
    strip_ansi,
    truncate_long_lines,
)


class TestStripAnsi:
    def test_removes_color_codes(self):
        text = "\x1b[31mError\x1b[0m: something failed"
        result, stats = strip_ansi(text)
        assert result == "Error: something failed"
        assert stats["ansi_chars_removed"] > 0

    def test_handles_no_ansi(self):
        text = "plain output"
        result, stats = strip_ansi(text)
        assert result == "plain output"
        assert stats["ansi_chars_removed"] == 0

    def test_handles_empty_string(self):
        result, stats = strip_ansi("")
        assert result == ""
        assert stats["ansi_chars_removed"] == 0

    def test_removes_complex_ansi(self):
        text = "\x1b[1;32mSUCCESS\x1b[0m \x1b[33mWARNING\x1b[0m"
        result, stats = strip_ansi(text)
        assert "SUCCESS" in result
        assert "WARNING" in result
        assert "\x1b" not in result


class TestDedupeAdjacentLines:
    def test_removes_consecutive_duplicates(self):
        text = "line1\nline1\nline1\nline2\nline3\nline3"
        result, stats = dedupe_adjacent_lines(text)
        assert result == "line1\nline2\nline3"
        assert stats["dup_lines_removed"] == 3

    def test_preserves_non_adjacent_duplicates(self):
        text = "line1\nline2\nline1"
        result, stats = dedupe_adjacent_lines(text)
        assert result == "line1\nline2\nline1"
        assert stats["dup_lines_removed"] == 0

    def test_single_line_unchanged(self):
        text = "only one line"
        result, stats = dedupe_adjacent_lines(text)
        assert result == text
        assert stats["dup_lines_removed"] == 0

    def test_empty_string(self):
        result, stats = dedupe_adjacent_lines("")
        assert result == ""
        assert stats["dup_lines_removed"] == 0

    def test_many_duplicates(self):
        lines = ["repeating"] * 100 + ["different"]
        text = "\n".join(lines)
        result, stats = dedupe_adjacent_lines(text)
        assert result == "repeating\ndifferent"
        assert stats["dup_lines_removed"] == 99


class TestTruncateLongLines:
    def test_truncates_over_limit(self):
        text = "short\n" + ("x" * 3000)
        result, stats = truncate_long_lines(text, max_line_chars=2000)
        assert "line truncated" in result
        assert stats["lines_truncated"] == 1

    def test_leaves_short_lines(self):
        text = "line1\nline2\nline3"
        result, stats = truncate_long_lines(text, max_line_chars=2000)
        assert result == text
        assert stats["lines_truncated"] == 0

    def test_respects_custom_limit(self):
        text = "x" * 100 + "\n" + "y" * 50
        result, stats = truncate_long_lines(text, max_line_chars=80)
        assert stats["lines_truncated"] == 1
        assert "y" * 50 in result


class TestKeepFirstLastLines:
    def test_omits_middle(self):
        lines = [f"line{i}" for i in range(200)]
        text = "\n".join(lines)
        result, stats = keep_first_last_lines(text, head_lines=50, tail_lines=30)
        assert stats["middle_lines_omitted"] == 120
        assert "line0" in result
        assert "line199" in result
        assert "lines omitted" in result

    def test_no_omission_when_short(self):
        lines = [f"line{i}" for i in range(30)]
        text = "\n".join(lines)
        result, stats = keep_first_last_lines(text, head_lines=50, tail_lines=30)
        assert stats["middle_lines_omitted"] == 0
        assert result == text

    def test_custom_head_tail(self):
        lines = [f"line{i}" for i in range(50)]
        text = "\n".join(lines)
        result, stats = keep_first_last_lines(text, head_lines=10, tail_lines=10)
        assert stats["middle_lines_omitted"] == 30


class TestExtractGitSummary:
    def test_extracts_file_headers_and_changes(self):
        text = (
            "diff --git a/src/main.rs b/src/main.rs\n"
            "index abc..def\n"
            "--- a/src/main.rs\n"
            "+++ b/src/main.rs\n"
            "@@ -10,6 +10,8 @@\n"
            " unchanged context\n"
            "+added line\n"
            "-removed line\n"
            " more context\n"
        )
        result, stats = extract_git_summary(text)
        assert "diff --git" in result
        assert "+added line" in result
        assert "-removed line" in result
        assert stats["git_chars_saved"] >= 0

    def test_handles_empty_input(self):
        result, stats = extract_git_summary("")
        assert result == ""
        assert stats["git_chars_saved"] == 0


class TestExtractPackageSummary:
    def test_keeps_summary_lines(self):
        text = (
            "downloading packages...\n"
            "progress: 50%\n"
            "added 15 packages\n"
            "removed 3 packages\n"
            "updated 8 packages\n"
            "audited 200 packages in 3s\n"
        )
        result, stats = extract_package_summary(text)
        assert "added 15 packages" in result
        assert "removed 3 packages" in result
        assert "progress" not in result
        assert stats["package_chars_saved"] > 0

    def test_fallback_when_no_summary(self):
        text = "downloading...\ncompiling...\nrunning...\n"
        result, stats = extract_package_summary(text)
        assert result == text
        assert stats["package_chars_saved"] == 0


class TestExtractDockerSummary:
    def test_keeps_last_lines_and_errors(self):
        lines = [f"build step {i}" for i in range(100)]
        lines[50] = "ERROR: something failed"
        text = "\n".join(lines)
        result, stats = extract_docker_summary(text)
        assert "ERROR" in result
        assert "build step 99" in result
        assert stats["docker_chars_saved"] > 0

    def test_short_output_unchanged(self):
        text = "Building...\nSuccess!\n"
        result, stats = extract_docker_summary(text)
        assert "Building" in result
        assert "Success" in result


class TestExtractErrorLines:
    def test_extracts_error_with_context(self):
        text = (
            "line1\nline2\nError: something failed\nline4\nline5\n"
            "line6\nFatal: critical\nline8\nline9"
        )
        result, stats = extract_error_lines(text, context_lines=1)
        assert "Error: something failed" in result
        assert "Fatal: critical" in result
        assert stats["error_lines_extracted"] > 0

    def test_no_errors_unchanged(self):
        text = "all good\nno problems here\nsuccess"
        result, stats = extract_error_lines(text)
        assert result == text
        assert stats["error_lines_extracted"] == 0

    def test_matches_various_error_patterns(self):
        text = "panic: null pointer\nTraceback (most recent call last):\nexception: boom"
        result, stats = extract_error_lines(text)
        assert "panic" in result
        assert "Traceback" in result
        assert "exception" in result
