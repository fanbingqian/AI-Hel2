"""HEIMDALL Manager — MemoryProvider implementation backed by HEIMDALL engine.

Implements the upstream ``agent.memory_provider.MemoryProvider`` interface
so HEIMDALL plugs into the agent's ``MemoryManager`` as a standard provider.
"""

from __future__ import annotations

import logging
from pathlib import Path
from typing import Any, Optional

from agent.memory_provider import MemoryProvider
from heimdall.config import HeimdallConfig
from heimdall.provider import HeimdallProvider

logger = logging.getLogger(__name__)


class HeimdallManager(MemoryProvider):
    """MemoryProvider implementation backed by the HEIMDALL knowledge engine."""

    def __init__(
        self,
        session_id: str = "",
        heimdall_dir: Optional[Path] = None,
        config: Optional[HeimdallConfig] = None,
        extract_fn: Any = None,
        raw_config: dict = None,
    ):
        if heimdall_dir is None:
            from hermes_constants import get_heimdall_dir
            heimdall_dir = get_heimdall_dir()

        self.session_id = session_id
        self.heimdall_dir = heimdall_dir
        self.config = config or HeimdallConfig()
        self._extract_fn = extract_fn
        self._raw_config = raw_config or {}

        self.provider = HeimdallProvider(
            heimdall_dir=heimdall_dir,
            config=self.config,
            extract_fn=extract_fn,
        )
        self._initialized = False

    # ------------------------------------------------------------------
    # MemoryProvider — identification
    # ------------------------------------------------------------------

    @property
    def name(self) -> str:
        return "heimdall"

    def is_available(self) -> bool:
        """HEIMDALL is always available when enabled (no external API keys needed)."""
        return True

    # ------------------------------------------------------------------
    # MemoryProvider — lifecycle
    # ------------------------------------------------------------------

    def initialize(self, session_id: str, **kwargs) -> None:
        """Initialize the HEIMDALL engine. Called once at session start.

        *kwargs* from MemoryManager.initialize_all() include:
          - hermes_home, platform, agent_context, agent_identity, etc.
        """
        if self._initialized:
            return
        self.session_id = session_id or self.session_id

        # Forward raw_config (embedding/API config) through kwargs
        merged_kwargs = dict(kwargs)
        if self._raw_config:
            merged_kwargs.setdefault("raw_config", self._raw_config)

        self.provider.initialize(session_id=self.session_id, **merged_kwargs)
        self._initialized = True

    def shutdown(self) -> None:
        """Shut down the HEIMDALL engine and persist state."""
        if self.provider:
            self.provider.shutdown()
        self._initialized = False

    # ------------------------------------------------------------------
    # MemoryProvider — optional hooks
    # ------------------------------------------------------------------

    def on_session_end(self, messages: list) -> None:
        """Handle session end — final extraction and cleanup."""
        if self.provider:
            self.provider.on_session_end(messages)

    def on_pre_compress(self, messages: list) -> str:
        """Extract context before compression."""
        if self.provider:
            return self.provider.on_pre_compress(messages)
        return ""

    def on_memory_write(self, action: str, target: str, content: str,
                        metadata: dict = None) -> None:
        """Bridge memory writes to the provider."""
        if self.provider:
            self.provider.on_memory_write(action, target, content)

    def on_delegation(self, task: str, result: str, *,
                      child_session_id: str = "", **kwargs) -> None:
        """Record delegation results."""
        if self.provider:
            self.provider.on_delegation(task, result, child_session_id)

    # ------------------------------------------------------------------
    # MemoryProvider — system prompt
    # ------------------------------------------------------------------

    def system_prompt_block(self) -> str:
        """Build the HEIMDALL system prompt block (persona + knowledge context).

        Returns empty string when not initialized.
        """
        if not self.provider or not self.provider._is_initialized:
            return ""
        _label, content = self.provider.system_prompt_block()
        if not content:
            return ""
        return content

    # ------------------------------------------------------------------
    # HEIMDALL-specific extensions (not part of upstream MemoryProvider)
    # ------------------------------------------------------------------

    def get_guidance_text(self) -> str:
        """Return the HEIMDALL tool guidance block for the system prompt.

        Called directly by the agent — not routed through MemoryManager.
        """
        return self.provider.NEXUS_GUIDANCE

    def set_namespace(self, namespace: str) -> None:
        """Override namespace for per-session domain scoping."""
        if self.provider:
            self.provider.set_namespace(namespace)

    def refresh_context(self) -> None:
        """Reload entity/knowledge state for a rebuilt system prompt.

        The HEIMDALL provider reloads from disk on each system_prompt_block()
        call, so no explicit reload is needed here.
        """
        pass

    # ------------------------------------------------------------------
    # MemoryProvider — per-turn operations
    # ------------------------------------------------------------------

    def prefetch(self, query: str, *, session_id: str = "") -> str:
        """Return cached prefetch from the previous turn's queue_prefetch."""
        _ = query
        if self.provider:
            return self.provider.get_cached_prefetch() or ""
        return ""

    def queue_prefetch(self, query: str, *, session_id: str = "") -> None:
        """Queue prefetch for the next turn."""
        if self.provider:
            self.provider.queue_prefetch(query, self.session_id)

    def sync_turn(self, user_content: str, assistant_content: str,
                  *, session_id: str = "") -> None:
        """Sync a completed turn — triggers entity extraction and context update."""
        if self.provider:
            self.provider.sync_turn(user_content, assistant_content, self.session_id)

    # ------------------------------------------------------------------
    # MemoryProvider — tool support
    # ------------------------------------------------------------------

    def get_tool_schemas(self) -> list[dict]:
        """Return all HEIMDALL tool schemas including the memory tool."""
        if not self.provider:
            return []
        return self.provider.get_tool_schemas()

    def handle_tool_call(self, tool_name: str, args: dict, **kwargs) -> str:
        """Dispatch a HEIMDALL tool call. Returns JSON string result."""
        if not self.provider:
            return '{"error": "HEIMDALL provider not initialized"}'
        return self.provider.handle_tool_call(tool_name, args, **kwargs)

    # ------------------------------------------------------------------
    # HEIMDALL-specific tool helpers (not part of upstream interface)
    # ------------------------------------------------------------------

    def has_tool(self, tool_name: str) -> bool:
        """Check whether *tool_name* belongs to HEIMDALL."""
        if not self.provider:
            return False
        if tool_name == "heimdall_memory":
            return True
        return self.provider.has_tool(tool_name)

    def is_memory_tool(self, tool_name: str) -> bool:
        """Check whether *tool_name* is the heimdall_memory tool."""
        return tool_name == "heimdall_memory"

    # ------------------------------------------------------------------
    # State
    # ------------------------------------------------------------------

    def is_initialized(self) -> bool:
        """Return True if the provider is initialized and ready."""
        return self._initialized

    def get_stats(self) -> dict:
        """Return provider statistics."""
        if self.provider:
            return self.provider.get_stats()
        return {"entities": 0, "is_first_run": True}
