"""
Main reduction pipeline. Given a tool result, classify it, match rules,
apply transforms in order, and return the compacted result.

Two entry points:
- reduce_tool_result: for the transform_tool_result hook (all tools)
- reduce_terminal_output: for the transform_terminal_output hook (terminal only)
"""

import logging
from typing import Any, Dict, Optional

from plugins.tokenjuice.classifier import classify
from plugins.tokenjuice.rules import RULES

logger = logging.getLogger("tokenjuice")

TINY_OUTPUT_MAX_CHARS = 240


def _apply_rules(tags: list, result: str) -> Optional[Dict[str, Any]]:
    """Match tags against rules and apply transforms in order.

    Returns stats dict or None if no rule matched.
    """
    original_size = len(result)
    stats: Dict[str, Any] = {"original_chars": original_size}
    compacted = result
    any_rule_matched = False

    # Try specific tags first (skip 'generic' until end)
    specific_tags = [t for t in tags if t != "generic"]
    if "generic" in tags:
        specific_tags.append("generic")

    for tag in specific_tags:
        for rule in RULES:
            if rule.applies_to(tag, original_size):
                any_rule_matched = True
                for transform in rule.transforms:
                    try:
                        compacted, t_stats = transform(compacted)
                        for k, v in t_stats.items():
                            if v:
                                if k in stats:
                                    stats[k] = stats[k] + v if isinstance(v, int) else v
                                else:
                                    stats[k] = v
                    except Exception as exc:
                        logger.debug(
                            "TokenJuice: transform %s failed on [%s]: %s",
                            getattr(transform, "__name__", str(transform)),
                            tag,
                            exc,
                        )
                break  # First matching rule per tag wins

    if not any_rule_matched:
        return None

    stats["compacted"] = compacted
    return stats


def reduce_tool_result(
    tool_name: str = "",
    args: dict = None,
    result: str = "",
    **kwargs,
) -> Optional[str]:
    """Entry point for the transform_tool_result hook.

    Called for every tool result before it enters LLM context.

    Returns:
        Compressed result string, or None (pass-through) if compression
        was skipped or would produce no savings.
    """
    if not result or len(result) <= TINY_OUTPUT_MAX_CHARS:
        return None

    if not isinstance(args, dict):
        args = {}
    tags = classify(tool_name, args, result)
    stats = _apply_rules(tags, result)

    if stats is None or "compacted" not in stats:
        return None

    compacted = stats["compacted"]
    original_size = stats["original_chars"]
    saved = original_size - len(compacted)
    pct = (saved / original_size * 100) if original_size > 0 else 0

    if saved <= 0:
        return None  # No actual savings

    logger.info(
        "TokenJuice: [%s] %s -> %s chars (%.0f%% saved, %s dup lines removed)",
        tool_name,
        original_size,
        len(compacted),
        pct,
        stats.get("dup_lines_removed", 0),
    )

    dup_removed = stats.get("dup_lines_removed", 0)
    marker = (
        f"[Compacted: {original_size} -> {len(compacted)} chars "
        f"({pct:.0f}% saved, {dup_removed} dup lines removed)]\n\n"
    )
    return marker + compacted


def reduce_terminal_output(
    tool_name: str = "",
    args: dict = None,
    result: str = "",
    **kwargs,
) -> Optional[str]:
    """Entry point for the transform_terminal_output hook.

    Terminal-specific compression — runs before the generic
    transform_tool_result hook for terminal tool output.
    Delegates to the same pipeline but adds terminal-specific pre-processing.
    """
    # For terminal, result may be a JSON string containing the output dict
    # We handle both raw strings and JSON-wrapped outputs
    return reduce_tool_result(
        tool_name=tool_name,
        args=args,
        result=result,
        **kwargs,
    )
