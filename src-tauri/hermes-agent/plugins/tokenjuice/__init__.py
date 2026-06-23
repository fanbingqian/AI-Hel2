"""
TokenJuice: Tool output compression plugin for Hermes Agent.

Compresses tool stdout/stderr before it enters the LLM context window.
Target savings: 30-60% token reduction on tool-heavy conversations.

Registered hooks:
    - transform_tool_result: compress all tool results
    - transform_terminal_output: compress terminal output (specialized)
"""


def register(ctx):
    """Register TokenJuice hooks with the Hermes plugin system."""
    from plugins.tokenjuice.reducer import (
        reduce_terminal_output,
        reduce_tool_result,
    )

    ctx.register_hook("transform_tool_result", reduce_tool_result)
    ctx.register_hook("transform_terminal_output", reduce_terminal_output)
