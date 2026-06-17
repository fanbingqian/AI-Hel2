"""
TokenJuice transform functions. Each transform takes a string and returns a
(string, stats_dict) tuple. Transforms are composable via pipeline chaining.

All transforms are idempotent and order-independent.
"""

import re
from typing import Any, Dict, Tuple

ANSI_RE = re.compile(r'\x1b\[[0-9;]*[a-zA-Z]')

HTML_TAG_RE = re.compile(
    r'<(script|style|noscript|iframe|svg)\b[^>]*>.*?</\1>',
    re.DOTALL | re.IGNORECASE,
)


def strip_ansi(text: str) -> Tuple[str, Dict[str, Any]]:
    """Remove ANSI escape sequences."""
    cleaned = ANSI_RE.sub('', text)
    removed = len(text) - len(cleaned)
    return cleaned, {"ansi_chars_removed": removed}


def dedupe_adjacent_lines(text: str) -> Tuple[str, Dict[str, Any]]:
    """Remove consecutive identical lines, replacing with a single instance.

    Handles the common case of build/test output repeating the same
    progress line hundreds of times.
    """
    lines = text.split('\n')
    if len(lines) <= 1:
        return text, {"dup_lines_removed": 0}

    deduped = []
    dup_count = 0
    prev = None
    for line in lines:
        if line == prev:
            dup_count += 1
        else:
            deduped.append(line)
            prev = line

    return '\n'.join(deduped), {"dup_lines_removed": dup_count}


def truncate_long_lines(
    text: str, max_line_chars: int = 2000
) -> Tuple[str, Dict[str, Any]]:
    """Truncate individual lines that exceed max_line_chars."""
    lines = text.split('\n')
    truncated = 0
    result = []
    for line in lines:
        if len(line) > max_line_chars:
            result.append(
                line[:max_line_chars]
                + f'... [line truncated, original {len(line)} chars]'
            )
            truncated += 1
        else:
            result.append(line)
    return '\n'.join(result), {"lines_truncated": truncated}


def keep_first_last_lines(
    text: str, head_lines: int = 50, tail_lines: int = 30
) -> Tuple[str, Dict[str, Any]]:
    """Keep first N and last N lines, drop the middle.

    Useful for build logs and test output where the beginning
    (setup/errors) and end (summary/results) matter most.
    """
    lines = text.split('\n')
    total = len(lines)
    keep = head_lines + tail_lines
    if total <= keep:
        return text, {"middle_lines_omitted": 0}

    omitted = total - keep
    head = lines[:head_lines]
    tail = lines[-tail_lines:]
    summary = (
        f"\n... [{omitted} lines omitted — "
        f"use read_file with offset to view full output] ...\n"
    )
    return (
        '\n'.join(head) + summary + '\n'.join(tail),
        {"middle_lines_omitted": omitted},
    )


def extract_git_summary(text: str) -> Tuple[str, Dict[str, Any]]:
    """For git status/diff: keep filenames + first 3-5 change lines per file.

    Drops unchanged context lines.
    """
    lines = text.split('\n')
    file_sections: list = []
    current_section: list = []

    for line in lines:
        stripped = line.strip()
        if (
            stripped.startswith('diff --git')
            or stripped.startswith('--- a/')
            or stripped.startswith('+++ b/')
        ):
            if current_section and len(current_section) > 1:
                header = current_section[0]
                changes = [
                    l for l in current_section[1:]
                    if l.startswith('+') or l.startswith('-')
                ]
                file_sections.append(header)
                file_sections.extend(changes[:5])
            current_section = [line]
        elif stripped.startswith('@@'):
            if current_section and len(current_section) > 1:
                header = current_section[0]
                changes = [
                    l for l in current_section[1:]
                    if l.startswith('+') or l.startswith('-')
                ]
                file_sections.append(header)
                file_sections.extend(changes[:3])
            current_section = [line]
        else:
            current_section.append(line)

    if current_section:
        header = current_section[0]
        changes = [
            l for l in current_section[1:]
            if l.startswith('+') or l.startswith('-')
        ]
        file_sections.append(header)
        file_sections.extend(changes[:3])

    result = '\n'.join(file_sections)
    saved = len(text) - len(result)
    return result, {"git_chars_saved": max(0, saved)}


def extract_package_summary(text: str) -> Tuple[str, Dict[str, Any]]:
    """For npm/cargo/pip install: keep only summary lines.

    (added/removed/updated/changed packages).
    """
    summary_lines = []
    keywords = [
        'added', 'removed', 'updated', 'changed', 'installed',
        'uninstalled', 'upgraded', 'downgraded', 'audited',
        'found 0 vulnerabilities', 'vulnerabilities found',
        'packages are looking for funding',
    ]

    for line in text.split('\n'):
        lower = line.lower().strip()
        if any(kw in lower for kw in keywords):
            summary_lines.append(line)

    if summary_lines:
        result = '\n'.join(summary_lines)
        saved = len(text) - len(result)
        return result, {"package_chars_saved": max(0, saved)}
    return text, {"package_chars_saved": 0}


def extract_docker_summary(text: str) -> Tuple[str, Dict[str, Any]]:
    """For docker build: keep last 30 lines + error lines."""
    lines = text.split('\n')
    error_lines = []
    last_n = lines[-30:] if len(lines) >= 30 else lines

    for line in lines[:-30]:
        lower = line.lower()
        if any(
            kw in lower
            for kw in ['error', 'fail', 'fatal', 'cannot', 'denied']
        ):
            error_lines.append(line)

    result_lines = error_lines + last_n
    result = '\n'.join(result_lines)
    omitted = max(0, len(lines) - len(result_lines))
    if omitted > 0:
        result = (
            f"... [{omitted} intermediate lines omitted] ...\n" + result
        )
    return result, {"docker_chars_saved": max(0, len(text) - len(result))}


def extract_error_lines(
    text: str, context_lines: int = 2
) -> Tuple[str, Dict[str, Any]]:
    """Extract error lines with surrounding context.

    Keeps lines matching error patterns + N context lines around each.
    """
    error_pattern = re.compile(
        r'(error|Error|ERROR|fail|Fail|FAIL|traceback|Traceback|'
        r'exception|Exception|fatal|Fatal|FATAL|assert|Assert|'
        r'panic|Panic|segfault|SIGSEGV|killed|Killed)',
    )
    lines = text.split('\n')
    error_indices: set = set()
    for i, line in enumerate(lines):
        if error_pattern.search(line):
            for j in range(
                max(0, i - context_lines),
                min(len(lines), i + context_lines + 1),
            ):
                error_indices.add(j)

    if not error_indices:
        return text, {"error_lines_extracted": 0}

    result_lines = [lines[i] for i in sorted(error_indices)]
    result = '\n'.join(result_lines)
    saved = len(text) - len(result)
    return result, {
        "error_lines_extracted": len(error_indices),
        "error_chars_saved": max(0, saved),
    }
