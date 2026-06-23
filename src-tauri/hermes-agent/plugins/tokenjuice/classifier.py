"""
Classify tool output to match the right compression rules.

Classification is based on tool_name + output content patterns.
Returns a list of applicable tags (a result can match multiple).
"""

import re
from typing import List

TOOL_CLASSIFICATION: dict = {
    "terminal": "shell",
    "bash": "shell",
    "execute_code": "code",
    "read_file": "file_read",
    "search_files": "search",
    "grep": "search",
    "web_fetch": "web",
    "web_search": "web",
    "list_directory": "file_list",
    "search_code_symbols": "search",
    "run_shell_command": "shell",
}

SHELL_SUB_PATTERNS: list = [
    (
        re.compile(r"\b(git\s+(status|diff|log|show|branch|stash))\b"),
        "git",
    ),
    (
        re.compile(
            r"\b(npm|cargo|pip|yarn|pnpm|poetry)\s+(install|add|update|upgrade)"
        ),
        "package_install",
    ),
    (
        re.compile(r"\b(docker|podman)\s+(build|run|compose|ps|images)\b"),
        "docker",
    ),
    (re.compile(r"\b(grep|rg|ag|ack)\b"), "grep"),
    (re.compile(r"\b(find|ls|dir|tree)\b"), "file_list"),
    (re.compile(r"\b(cat|head|tail|less)\b"), "file_read"),
    (re.compile(r"\b(curl|wget)\b"), "http"),
    (
        re.compile(
            r"\b(pytest|npm test|cargo test|go test|jest|unittest)\b"
        ),
        "test_run",
    ),
]


def classify(
    tool_name: str = "", args: dict = None, output: str = ""
) -> List[str]:
    """Return classification tags for a tool result.

    Tags are applied in order of specificity — later tags override earlier
    in the rule matching phase.
    """
    tags: list = []

    primary = TOOL_CLASSIFICATION.get(tool_name)
    if primary:
        tags.append(primary)

    if primary == "shell" and isinstance(args, dict):
        command = args.get("command", "")
        for pattern, tag in SHELL_SUB_PATTERNS:
            if pattern.search(command):
                tags.append(tag)
                break

    # Output-based classification
    output_head = output[:500].lower() if output else ""
    if (
        "error" in output_head
        or "traceback" in output_head
        or "exception" in output_head
    ):
        tags.append("error_output")

    tags.append("generic")
    return tags
