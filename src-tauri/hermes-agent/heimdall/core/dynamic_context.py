"""Dynamic system prompt context builder (V2.3).

Injects knowledge overview + scale-adaptive guidance into the LLM system prompt
each turn. Designed to be independent of run_agent.py — only requires ~5 lines
of integration in the agent's _build_system_prompt().

Usage:
    from heimdall.core.dynamic_context import DynamicContextBuilder
    builder = DynamicContextBuilder(overview_cache, store)
    dynamic_segment = builder.build(namespace)  # namespace=None for global
"""

from __future__ import annotations

from .overview_cache import OverviewCache, behavior_guidance


class DynamicContextBuilder:
    """Builds the dynamic segment of the system prompt each turn."""

    def __init__(self, overview_cache: OverviewCache, store):
        self._overview_cache = overview_cache
        self._store = store

    def build(self, namespace: str | None = None) -> str:
        """Build the dynamic context segment for the system prompt.

        Args:
            namespace: Per-agent domain name, or None for global overview.
        """
        overview = self._overview_cache.read(namespace)

        try:
            if namespace:
                count = self._store.count_by_namespace(namespace)
            else:
                count = self._store.count_all()
        except Exception:
            count = 0

        guidance = behavior_guidance(count)

        return (
            "## 知识库\n"
            f"{overview}\n\n"
            "## 检索指引\n"
            f"{guidance}"
        )
