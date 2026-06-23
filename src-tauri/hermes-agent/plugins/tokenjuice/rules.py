"""
Compression rule definitions. Each rule maps a classification tag + optional
output size threshold to a list of transform steps.

Rules are ordered: first match wins per tag. Later rules are fallbacks.
"""

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


class CompressionRule:
    """A single compression rule that matches a classification tag."""

    def __init__(
        self,
        tag: str,
        min_chars: int,
        transforms: list,
        description: str = "",
    ):
        self.tag = tag
        self.min_chars = min_chars
        self.transforms = transforms
        self.description = description

    def applies_to(self, tag: str, output_size: int) -> bool:
        return self.tag == tag and output_size > self.min_chars


RULES = [
    CompressionRule(
        tag="git",
        min_chars=240,
        transforms=[strip_ansi, extract_git_summary, dedupe_adjacent_lines],
        description="git: filenames + first 3-5 change lines per file",
    ),
    CompressionRule(
        tag="package_install",
        min_chars=240,
        transforms=[
            strip_ansi,
            extract_package_summary,
            dedupe_adjacent_lines,
        ],
        description="npm/cargo/pip install: summary lines only",
    ),
    CompressionRule(
        tag="docker",
        min_chars=240,
        transforms=[
            strip_ansi,
            extract_docker_summary,
            dedupe_adjacent_lines,
        ],
        description="docker build: last 30 lines + error lines",
    ),
    CompressionRule(
        tag="grep",
        min_chars=240,
        transforms=[strip_ansi, dedupe_adjacent_lines, truncate_long_lines],
        description="grep/search: dedupe adjacent, truncate lines > 2000 chars",
    ),
    CompressionRule(
        tag="file_list",
        min_chars=500,
        transforms=[strip_ansi, dedupe_adjacent_lines],
        description="ls/find/tree: dedupe only",
    ),
    CompressionRule(
        tag="test_run",
        min_chars=1000,
        transforms=[
            strip_ansi,
            keep_first_last_lines,
            dedupe_adjacent_lines,
        ],
        description="pytest/jest/cargo test: first 50 + last 30 lines",
    ),
    CompressionRule(
        tag="error_output",
        min_chars=500,
        transforms=[strip_ansi, extract_error_lines, dedupe_adjacent_lines],
        description="Error output: extract error lines with ±2 context lines",
    ),
    CompressionRule(
        tag="code",
        min_chars=500,
        transforms=[strip_ansi, dedupe_adjacent_lines, truncate_long_lines],
        description="Code execution output: strip ANSI, dedupe, truncate long lines",
    ),
    CompressionRule(
        tag="generic",
        min_chars=500,
        transforms=[strip_ansi, dedupe_adjacent_lines, truncate_long_lines],
        description="Generic: strip ANSI, dedupe adjacent, truncate > 2000 chars",
    ),
]
