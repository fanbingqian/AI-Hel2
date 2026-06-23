"""HEIMDALL Provider — unified memory engine replacing MemoryProvider ABC.

HeimdallProvider is the single integration point that replaces the old
MemoryProvider + BuiltinMemoryProvider + 8 external plugin architecture.
It connects the HEIMDALL core engine (entity store, persona, knowledge,
social graph, retrieval, elevator) to the agent loop.

Key design decisions:
  - Frozen snapshot at session start (prompt cache stability)
  - <memory-context> fence for prefetched content
  - tool schemas served from heimdall/tools/* via the registry
  - Sync triggers entity extraction after each turn
"""

from __future__ import annotations

import logging
import time
from pathlib import Path
from typing import Any, Optional

from heimdall.config import HeimdallConfig
from heimdall.core.entity_store import EntityStore
from heimdall.core.persona import PersonaManager
from heimdall.core.knowledge import KnowledgeManager
from heimdall.core.social_graph import SocialGraph
from heimdall.core.media_refs import MediaRefIndex
from heimdall.core.extraction import EntityExtractor
from heimdall.core.retrieval import MultiPathRetriever
from heimdall.core.cold_start import ColdStartExtractor
from heimdall.core.privacy import load_or_create_salt
from heimdall.core.embedding import EmbeddingClient, EmbeddingQueue
from heimdall.core.overview_cache import CommunityCache, OverviewCache
from heimdall.core.dynamic_context import DynamicContextBuilder
from heimdall.elevator.levels import CognitiveElevator, ElevationLevel
from heimdall.elevator.actions import ActionSuggester
from heimdall.views.reconnect import ReconnectEngine
from heimdall.views.empty_states import EmptyStates

logger = logging.getLogger(__name__)


class HeimdallProvider:
    """Integrated HEIMDALL memory engine for the AI agent.

    This replaces the old MemoryProvider ABC and its 8 plugin implementations.
    It manages the full HEIMDALL lifecycle: initialize → prefetch → sync → shutdown.

    Usage with AIAgent:
        provider = HeimdallProvider(heimdall_dir, config)
        provider.initialize(session_id="session-1", platform="cli")

        # Each turn:
        context = provider.prefetch(user_message, session_id)
        provider.sync_turn(user_message, assistant_response, session_id)

        # End:
        provider.shutdown()
    """

    # Provider identity
    name = "heimdall"
    NEXUS_GUIDANCE = (
        "You have access to Nexus, a local knowledge graph engine with 5 tools:\n"
        "- `nexus_map` — View the complete knowledge map: domain distribution, key entities, "
        "subdomains, and cross-domain bridges. Use first to survey what the knowledge base covers.\n"
        "- `nexus_search` — Full-text search for entities. Use after nexus_map (or directly) "
        "to find relevant concepts.\n"
        "- `nexus_detail` — Get full entity details including all inbound/outbound relations. "
        "Use to drill into interesting search results.\n"
        "- `nexus_paths` — Find shortest relationship paths between two entities (names or UUIDs). "
        "Use to understand how concepts connect.\n"
        "- `nexus_neighbors` — Expand the neighborhood around an entity (BFS, 1-4 hops). "
        "Use to browse related concepts.\n\n"
        "Principles:\n"
        "- You decide when to call each tool — no automatic injection of knowledge context.\n"
        "- nexus_map gives you a bird's-eye view; call it when the user references domains "
        "you haven't explored yet.\n"
        "- If nexus_map shows relevant coverage, use nexus_search to find specific entities.\n"
        "- If the knowledge base has no relevant content, fall back to web_search.\n"
        "- All tools are read-only and query the local knowledge graph."
        "When you learn something durable about the user, record it. "
        "Skip temporary task state and one-time queries."
    )

    def __init__(
        self,
        heimdall_dir: Path,
        config: Optional[HeimdallConfig] = None,
        extract_fn: Any = None,
    ):
        self.heimdall_dir = heimdall_dir
        self.config = config or HeimdallConfig()
        self._extract_fn = extract_fn
        self._session_id: str = ""
        self._platform: str = "cli"
        self._is_initialized: bool = False
        self._prefetch_cache: Optional[str] = None

        # Core engine components (created at init, initialized later)
        self.store: Optional[EntityStore] = None
        self.persona: Optional[PersonaManager] = None
        self.knowledge: Optional[KnowledgeManager] = None
        self.social_graph: Optional[SocialGraph] = None
        self.media_refs: Optional[MediaRefIndex] = None
        self.extractor: Optional[EntityExtractor] = None
        self.retriever: Optional[MultiPathRetriever] = None
        self.cold_start: Optional[ColdStartExtractor] = None
        self.elevator: Optional[CognitiveElevator] = None
        self.action_suggester: Optional[ActionSuggester] = None
        self.reconnect: Optional[ReconnectEngine] = None

        # V2.3: Embedding + Knowledge overview + Dynamic context
        self._embedding_client: Optional[EmbeddingClient] = None
        self._embedding_queue: Optional[EmbeddingQueue] = None
        self._community_cache: Optional[CommunityCache] = None
        self._overview_cache: Optional[OverviewCache] = None
        self._dynamic_builder: Optional[DynamicContextBuilder] = None
        self._namespace: Optional[str] = None

    # ------------------------------------------------------------------
    # Availability
    # ------------------------------------------------------------------

    def is_available(self) -> bool:
        """HEIMDALL is always available — no external credentials needed."""
        return True

    def set_namespace(self, namespace: str) -> None:
        """Override the namespace for per-turn domain scoping.

        Called by the agent loop before each conversation turn when the
        desktop sends knowledge_namespace in the API request body.
        This allows a single HEIMDALL instance to serve multiple agent
        profiles without restarting.
        """
        self._namespace = namespace

    # ------------------------------------------------------------------
    # Lifecycle
    # ------------------------------------------------------------------

    def initialize(self, session_id: str = "", **kwargs) -> None:
        """Initialize the full HEIMDALL engine.

        Called once at session start. Creates the entity store, loads
        persona/knowledge snapshots, and wires up all components.
        """
        if self._is_initialized:
            return

        self._session_id = session_id
        self._platform = kwargs.get("platform", "cli")
        hermes_home = kwargs.get("hermes_home")
        if hermes_home:
            self.heimdall_dir = Path(hermes_home) / "heimdall"

        self.heimdall_dir.mkdir(parents=True, exist_ok=True)

        # Step 1: Load or create privacy salt
        salt_path = self.heimdall_dir / ".salt"
        salt = load_or_create_salt(salt_path)

        # Step 2: Initialize entity store
        db_path = self.heimdall_dir / "heimdall.db"
        self.store = EntityStore(db_path, salt=salt)
        self.store.initialize()

        # Step 3: Load persona and knowledge snapshots
        self.persona = PersonaManager(self.heimdall_dir, store=self.store)
        self.persona.load()

        self.knowledge = KnowledgeManager(self.heimdall_dir, store=self.store)
        self.knowledge.load()

        # Step 4: Tool state wiring removed — old heimdall tools replaced by Nexus

        # Step 5: Initialize sub-components
        self.social_graph = SocialGraph(self.store)
        self.media_refs = MediaRefIndex(self.store)

        self.extractor = EntityExtractor(
            self.store,
            extract_fn=self._extract_fn,
            confidence_threshold=self.config.extraction_confidence_threshold,
            max_entities_per_turn=self.config.extraction_max_entities_per_turn,
        )

        self.retriever = MultiPathRetriever(
            self.store,
            vector_weight=self.config.retrieval_vector_weight,
            keyword_weight=self.config.retrieval_keyword_weight,
            graph_weight=self.config.retrieval_graph_weight,
            temporal_weight=self.config.retrieval_temporal_weight,
            top_k=self.config.retrieval_top_k,
        )

        self.cold_start = ColdStartExtractor(
            self.store, self.persona, self.knowledge,
            self.extractor, self.heimdall_dir,
        )

        self.elevator = CognitiveElevator()
        self.action_suggester = ActionSuggester(self.store)
        self.reconnect = ReconnectEngine(self.store)

        # Step 6: Run cold start or migration if needed
        if self.cold_start.is_first_run() and not self.cold_start.is_migrated_from_hermes():
            hermes_home_path = Path(hermes_home) if hermes_home else Path.home() / ".hermes"
            if hermes_home_path.exists():
                logger.info("First HEIMDALL run — attempting migration from %s", hermes_home_path)
                try:
                    result = self.cold_start.migrate_from_hermes(hermes_home_path)
                    logger.info("Migration result: %s", result)
                except Exception as e:
                    logger.error("Migration failed (non-fatal): %s", e)

        # Step 7: V2.3 — embedding, overview cache, dynamic context
        self._namespace = self.config.knowledge_namespace
        raw_config = kwargs.get("raw_config", {})
        self._embedding_client = EmbeddingClient(raw_config)
        self._embedding_queue = EmbeddingQueue(self._embedding_client, self.store)
        self._embedding_queue.start()

        self._community_cache = CommunityCache()
        self._overview_cache = OverviewCache(self.store, self._community_cache)
        self._dynamic_builder = DynamicContextBuilder(self._overview_cache, self.store)

        self._is_initialized = True
        logger.info("HEIMDALL initialized (session=%s, platform=%s, entities=%d, namespace=%s)",
                     session_id, self._platform, self.store.get_entity_count(), self._namespace)

    def shutdown(self) -> None:
        """Clean shutdown — close database, persist pending writes."""
        if self._embedding_queue:
            self._embedding_queue.stop()
        if self.store:
            self.store.close()
            self.store = None
        self._is_initialized = False
        self._prefetch_cache = None

    # ------------------------------------------------------------------
    # System prompt
    # ------------------------------------------------------------------

    def system_prompt_block(self) -> tuple[str, str]:
        """Return (label, content) for injection into the system prompt.

        Returns lightweight Nexus guidance — the Agent now queries knowledge
        on-demand via the 5 nexus_* tools rather than receiving injected context.
        """
        return ("Nexus", self.NEXUS_GUIDANCE)

    # ------------------------------------------------------------------
    # Per-turn operations
    # ------------------------------------------------------------------

    def prefetch(self, query: str, session_id: str = "") -> str:
        """Retrieve relevant memory context for the current user query (V2.3: + namespace).

        Returns a context string wrapped in <memory-context> fence tags.
        This is injected into the user message, NOT the system prompt.
        """
        if not self._is_initialized or not self.retriever:
            return ""

        context = self.retriever.prefetch_context(query, namespace=self._namespace)
        if not context:
            return ""

        return self._build_memory_context_block(context)

    def queue_prefetch(self, query: str, session_id: str = "") -> None:
        """Queue prefetch for the next turn (currently synchronous).

        In Phase 2, this will use a background thread for async retrieval.
        """
        self._prefetch_cache = self.prefetch(query, session_id)

    def get_cached_prefetch(self) -> Optional[str]:
        """Return cached prefetch result from the previous queue_prefetch call."""
        cached = self._prefetch_cache
        self._prefetch_cache = None
        return cached

    def sync_turn(
        self,
        user_message: str,
        assistant_message: str,
        session_id: str = "",
    ) -> None:
        """Sync a completed conversation turn to HEIMDALL.

        Triggers entity extraction, social graph updates, and Knowledge Ring
        original text archiving.

        V2.2: Handles escalated extractions (timeout/low-confidence → A2A path).
              500ms hardcoded timeout enforced inside extract_from_turn.
        """
        if not self._is_initialized or not self.extractor:
            return

        sid = session_id or self._session_id

        result = self.extractor.extract_from_turn(
            user_message=user_message,
            assistant_message=assistant_message,
            session_id=sid,
            namespace=self._namespace or "general",
        )

        # V2.2: Handle escalated extractions
        if result.get("escalated"):
            logger.info("Extraction escalated: %s (elapsed: %.0fms)",
                       result.get("reason", "unknown"),
                       result.get("elapsed_ms", 0))
            # If A2A escalation is enabled in config, send payload
            # (Phase 2: async cloud call; Phase 1: log + skip)

        # Archive original text to Knowledge Ring originals table
        try:
            combined = f"User: {user_message}\nAssistant: {assistant_message}"
            self.store.archive_original(
                source_type="dialog",
                content=combined,
                metadata={"session_id": sid, "user_msg_len": len(user_message)},
                namespace=self._namespace or "general",
            )
        except Exception:
            logger.debug("Failed to archive original text", exc_info=True)

        # V2.3: Schedule overview cache rebuild after extraction
        if self._overview_cache and self._namespace:
            try:
                self._overview_cache.schedule_rebuild(self._namespace)
            except Exception:
                pass

        # Phase 1: Run transitive inference and causal chain building
        self._run_evolution_pipeline(result, sid)

    def _run_evolution_pipeline(self, extraction_result: dict, session_id: str) -> None:
        """Run inference + causal chain building after entity extraction.

        These are pure graph algorithms (no LLM), so they complete in < 1s.
        Failures are logged but never surfaced to the user.
        """
        ns = self._namespace or "general"
        try:
            from heimdall.core.inference import TransitiveInferenceEngine
            engine = TransitiveInferenceEngine(self.store)
            inferences = engine.run(namespace=ns, min_shared_neighbors=2)

            now = __import__("time").strftime("%Y-%m-%dT%H:%M:%S")
            for inf in inferences:
                try:
                    self.store._conn.execute(
                        """INSERT OR IGNORE INTO kr_inferences
                           (inference_id, entity_a, entity_b, inferred_type, evidence,
                            confidence, status, namespace, created_at)
                           VALUES (?, ?, ?, ?, ?, ?, 'pending', ?, ?)""",
                        (inf.inference_id, inf.entity_a, inf.entity_b, inf.inferred_type,
                         __import__("json").dumps(inf.evidence, ensure_ascii=False),
                         inf.confidence, ns, now),
                    )
                except Exception:
                    pass
            self.store._conn.commit()

            if inferences:
                logger.info(
                    "Evolution pipeline: %d inferences for namespace=%s",
                    len(inferences), ns,
                )
        except Exception:
            logger.debug("Inference engine skipped", exc_info=True)

        try:
            from heimdall.core.evolution.causal import CausalChainBuilder
            builder = CausalChainBuilder(self.store)
            builder.rebuild_all(namespace=ns)
        except Exception:
            logger.debug("Causal chain builder skipped", exc_info=True)

    # ------------------------------------------------------------------
    # Tool schemas
    # ------------------------------------------------------------------

    def get_tool_schemas(self) -> list[dict]:
        """Return tool schemas for HEIMDALL tools.

        Tools are registered in the central tool registry at import time
        by heimdall/tools/*.py. This method returns the subset of schemas
        for HEIMDALL tools.
        """
        from tools.registry import registry
        heimdall_tools = [
            t for t in registry._tools.values()
            if t.toolset == "heimdall"
        ]
        return [t.schema for t in heimdall_tools]

    def has_tool(self, tool_name: str) -> bool:
        """Check if a tool name belongs to HEIMDALL."""
        from tools.registry import registry
        return registry.get_toolset_for_tool(tool_name) == "heimdall"

    def handle_tool_call(self, tool_name: str, args: dict, **kwargs) -> str:
        """Dispatch a tool call to the appropriate tool handler.

        Old heimdall tool handlers replaced by Nexus tools.
        """
        from tools.registry import registry
        return registry.dispatch(tool_name, args)

    # ------------------------------------------------------------------
    # Lifecycle hooks
    # ------------------------------------------------------------------

    def on_turn_start(self, turn_number: int, message: str, **kwargs) -> None:
        """Called at the start of each turn."""
        pass

    def on_session_end(self, messages: list) -> None:
        """Called when the session ends — perform final extraction."""
        if not self.extractor or not messages:
            return
        last_user = ""
        last_assistant = ""
        for m in reversed(messages):
            if m.get("role") == "user" and not last_user:
                last_user = m.get("content", "")
            if m.get("role") == "assistant" and not last_assistant:
                last_assistant = m.get("content", "")
        if last_user or last_assistant:
            self.extractor.extract_from_turn(last_user, last_assistant, self._session_id)

    def on_pre_compress(self, messages: list) -> str:
        """Extract key context before compression."""
        if not self._is_initialized:
            return ""
        # Return entity summary to preserve across compression boundary
        count = self.store.get_entity_count() if self.store else 0
        top_entities = self.store.list_entities(limit=10) if self.store else []
        names = [e.get("display_name", "") for e in top_entities if e.get("occurrence_count", 0) > 1]
        return f"HEIMDALL: {count} entities tracked. Key entities: {', '.join(names[:5])}" if names else ""

    def on_memory_write(self, action: str, target: str, content: str) -> None:
        """Handle explicit memory writes from the agent.

        When the agent calls heimdall_memory tool, this ensures the
        entity store and persona/knowledge are updated.
        """
        if not self.store:
            return
        if target == "memory":
            self.store.upsert_entity(
                display_name=content[:200],
                entity_type="concept",
                confidence=0.8,
                source_session_id=self._session_id,
            )

    def on_delegation(self, task: str, result: str, child_session_id: str) -> None:
        """Record subagent work in the entity store."""
        if self.store:
            self.store.upsert_entity(
                display_name=f"子任务: {task[:100]}",
                entity_type="event",
                source_session_id=child_session_id,
                confidence=0.5,
            )

    # ------------------------------------------------------------------
    # Elevator integration
    # ------------------------------------------------------------------

    def classify_query(self, user_message: str) -> ElevationLevel:
        """Classify a user query's capability level."""
        if not self.elevator:
            return ElevationLevel.L1
        return self.elevator.classify(user_message)

    def get_elevator_warning(self, level: ElevationLevel, response: str) -> Optional[str]:
        """Get an elevator warning for L2/L3 responses, if applicable."""
        if not self.elevator:
            return None
        if level == ElevationLevel.L2:
            return self.elevator.get_l2_warning(response)
        elif level == ElevationLevel.L3:
            return self.elevator.get_l3_block_message(response[:100])
        return None

    # ------------------------------------------------------------------
    # Suggestions
    # ------------------------------------------------------------------

    def get_suggestions(self) -> dict:
        """Get all action suggestions (reconnect, knowledge review, etc.)."""
        if not self.action_suggester:
            return {"L1": [], "L2": [], "L3": []}
        return self.action_suggester.get_all_suggestions()

    def get_reconnect_suggestions(self) -> list[dict]:
        """Get reconnect suggestions."""
        if not self.reconnect:
            return []
        return self.reconnect.get_suggestions()

    # ------------------------------------------------------------------
    # Stats
    # ------------------------------------------------------------------

    def get_stats(self) -> dict:
        return {
            "entities": self.store.get_entity_count() if self.store else 0,
            "is_first_run": self.cold_start.is_first_run() if self.cold_start else True,
            "elevator": self.elevator.stats if self.elevator else {},
            "social_graph": self.social_graph.get_stats() if self.social_graph else {},
        }

    # ------------------------------------------------------------------
    # Helpers
    # ------------------------------------------------------------------

    @staticmethod
    def _build_memory_context_block(raw_context: str) -> str:
        """Wrap context in fence tags to separate from user input."""
        if not raw_context.strip():
            return ""
        return (
            "<memory-context>\n"
            "以下是从记忆系统中检索到的上下文信息，用于辅助理解用户意图，"
            "并非用户当前输入的一部分。\n\n"
            f"{raw_context}\n"
            "</memory-context>"
        )
