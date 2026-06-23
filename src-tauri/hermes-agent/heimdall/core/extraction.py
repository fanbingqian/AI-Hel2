"""HEIMDALL Entity Extractor — LLM-assisted entity and relation extraction.

After each conversation turn, extracts entities (person, concept, skill,
event, etc.) and their relationships from the dialogue. Uses the Hermes
auxiliary client for LLM-based extraction; ONNX-based extraction is a
future option (Phase 2).

Knowledge Ring V1.0: Extended to extract 5 main entity types, 8 relation types,
aliases, domains, and confidence scores. Dual-writes to both old and new tables.

V2.2: 500ms hardcoded timeout (time.monotonic), FTRL auto-decision integration.
"""

from __future__ import annotations

import json
import logging
import time
from typing import Any, Optional

from heimdall.core.entity_store import EntityStore

logger = logging.getLogger(__name__)

# V2.2: Hardcoded physical constraint — cannot be disabled by config
EXTRACTION_TIMEOUT_MS = 500

# Patterns for entity names that should be rejected (code artifacts, errors, filenames)
_JUNK_PATTERNS = [
    r'\.(py|js|ts|jsx|tsx|java|go|rs|cpp|c|h|css|html|md|yaml|yml|json|xml|sql|db|log|tmp|bak)$',
    r'^(error|exception|traceback|warning|debug|info)s*[:/]',
    r'error\s*code:\s*\d+',
    r'^\s*\{\s*[\'"].*error.*[\'"]',
    r'^(File|文件)\s*["""]',
    r'^(line|行)\s*\d+',
    r'^/\w+(/\w+)+',  # Unix paths
    r'^[A-Z]:\\',     # Windows paths
    r'\.(min|bundle|chunk|vendor)\b',
    r'^(node_modules|__pycache__|\.git|dist|build|src|lib|bin|etc|tmp|var)$',
]


def _is_junk_entity_name(name: str) -> bool:
    """Return True if the name looks like a filename, error message, or code artifact."""
    import re as _re
    # Reject very long names (likely error messages or reasoning text)
    if len(name) > 80:
        return True
    for pat in _JUNK_PATTERNS:
        if _re.search(pat, name, _re.IGNORECASE):
            return True
    return False

EXTRACTION_SYSTEM_PROMPT = """You are an entity extraction system for a personal AI memory agent.
Analyze the conversation below and extract structured information.

Output valid JSON with two arrays:
1. "entities" — people, organizations, projects, tools, concepts, skills, events, locations mentioned.
   Each entity: {"name": "...", "type": "person|organization|project|tool|concept|skill|event|location", "confidence": 0.0-1.0, "attributes": {...}}
2. "edges" — relationships between entities.
   Each edge: {"source": "entity_name", "target": "entity_name", "role": "subject|object|context", "emotion": -1.0 to 1.0}

Rules:
- Extract only durable information, not transient task state
- confidence < 0.6 means uncertain — mark for review
- emotion: positive values = positive sentiment, negative = negative
- For third-party persons, use only first names or relationship labels
- Do NOT include the user themselves as an entity
- Limit to 8 entities maximum"""

# Knowledge Ring V1.0 extraction prompt — extended with relations, domains, aliases
EXTRACTION_SYSTEM_PROMPT_V2 = """You are a knowledge graph extraction system for a personal AI memory engine.
Analyze the conversation below and extract structured knowledge.

Output valid JSON with these arrays:
1. "entities" — durable concepts, content, people, events, artifacts mentioned.
   Each entity: {"name": "...", "type": "concept|content|person|event|artifact", "type_detail": "...", "confidence": 0.0-1.0, "properties": {...}}
   - concept: abstract knowledge (disciplines, methodologies, domains, insights, skills)
   - content: information with载体 (articles, books, videos, podcasts, webpages, dialogs)
   - person: people, organizations, groups
   - event: meetings, projects, tasks, decisions, activities, milestones
   - artifact: tangible works, tools, items
   - type_detail: optional subclass (e.g. "skill" for concept, "project" for event, "tool" for artifact, "organization" for person)

2. "relations" — explicit relationships between entities.
   Each relation: {"source": "entity_name", "target": "entity_name", "type": "belongs_to|contains|relates_to|contrasts_with|causes|produces|inspired_by|knows", "confidence": 0.0-1.0, "direction": "unidirectional|bidirectional"}
   - belongs_to: child → parent (e.g. Transformer → Deep Learning)
   - contains: parent → child (e.g. Deep Learning → Transformer)
   - relates_to: general association
   - contrasts_with: opposition / comparison
   - causes: cause → effect
   - produces: creator → output
   - inspired_by: influenced by
   - knows: social connection between people

3. "aliases" — alternative names for entities in this conversation.
   Each alias: {"entity_name": "canonical name", "alias": "alternative name", "context": "usage context"}

4. "domains" — knowledge domains this conversation touches.
   Just a list of domain name strings: ["AI/ML", "农业", "设计", ...]

Rules:
- Extract only durable information, not transient task state
- confidence < 0.6 means uncertain
- For third-party persons, use only first names or relationship labels
- Do NOT include the user themselves as an entity
- Limit to 8 entities maximum
- Domains should be broad categories (Chinese or English)"""

# Old → new entity type mapping for dual-write
_OLD_TYPE_MAP = {
    "concept": "concept", "content": "concept", "person": "person",
    "event": "event", "artifact": "concept",
}


class EntityExtractor:
    """LLM-assisted entity extraction from conversation turns.

    Uses the Hermes auxiliary client for extraction. The caller passes
    an `extract_fn` callable that takes (system_prompt, user_prompt) and
    returns a string response. This avoids a hard dependency on the
    auxiliary client module.

    V2.2: 500ms hardcoded timeout, FTRL auto-decision integration.
    """

    def __init__(
        self,
        store: EntityStore,
        extract_fn: Any = None,
        confidence_threshold: float = 0.6,
        max_entities_per_turn: int = 8,
    ):
        self.store = store
        self._extract_fn = extract_fn  # callable(system_prompt, user_prompt) -> str
        self.confidence_threshold = confidence_threshold
        self.max_entities_per_turn = max_entities_per_turn

        # V2.2: FTRL auto-decision (lazy init)
        self._ftrl_decision = None
        self._escalation_client = None

    @property
    def ftrl_decision(self):
        if self._ftrl_decision is None:
            from heimdall.core.ftrl import FTRLAutoDecision
            self._ftrl_decision = FTRLAutoDecision()
        return self._ftrl_decision

    @property
    def escalation_client(self):
        if self._escalation_client is None:
            from heimdall.core.escalation import A2AEscalationClient
            self._escalation_client = A2AEscalationClient()
        return self._escalation_client

    # ------------------------------------------------------------------
    # Public API
    # ------------------------------------------------------------------

    def extract_from_turn(
        self,
        user_message: str,
        assistant_message: str,
        session_id: str = "",
        namespace: str = "general",
    ) -> dict:
        """Extract entities from a completed conversation turn (V2.3: + namespace).

        V2.2: 500ms hardcoded timeout enforced via time.monotonic().
        Returns:
            {"entities": [...], "edges": [...], "count": N}
        """
        start = time.monotonic()

        if not self._extract_fn:
            result = self._rule_based_extract(user_message, assistant_message, session_id, namespace)
        else:
            try:
                # Check timeout before LLM call
                if (time.monotonic() - start) * 1000 > EXTRACTION_TIMEOUT_MS:
                    return self._timeout_result("pre_extract_timeout", start)
                result = self._llm_extract(user_message, assistant_message, session_id, namespace)
            except Exception as exc:
                logger.warning("LLM extraction failed, falling back to rule-based: %s", exc)
                result = self._rule_based_extract(user_message, assistant_message, session_id, namespace)

        # V2.2: Apply FTRL auto-decision
        result = self._apply_auto_decision(result)

        elapsed_ms = (time.monotonic() - start) * 1000
        if elapsed_ms > EXTRACTION_TIMEOUT_MS:
            logger.warning("Extraction exceeded %dms (took %.0fms), escalating",
                          EXTRACTION_TIMEOUT_MS, elapsed_ms)
            result["_escalated"] = True
            result["_elapsed_ms"] = elapsed_ms

        return result

    def extract_from_text(self, text: str, session_id: str = "", namespace: str = "general") -> dict:
        """Extract entities from a single text block (used for migration)."""
        return self.extract_from_turn(text, "", session_id, namespace=namespace)

    # ------------------------------------------------------------------
    # V2.2: FTRL auto-decision + timeout handling
    # ------------------------------------------------------------------

    def _apply_auto_decision(self, extraction_result: dict) -> dict:
        """FTRL-driven auto decision (V2.2: dynamic thresholds).

        Classifies each extracted entity as:
          - confirmed: confidence >= FTRL-predicted threshold
          - low_confidence: confidence between threshold*0.5 and threshold
          - pending_review: below threshold*0.5, needs user review
        """
        entities = extraction_result.get("entities", [])
        if not entities or not self._extract_fn:
            return extraction_result

        try:
            ftrl = self.ftrl_decision
        except Exception:
            return extraction_result

        for entity in entities:
            conf = float(entity.get("confidence", 0.5))
            entity_type = entity.get("type", "concept")
            if isinstance(entity_type, list):
                entity_type = entity_type[0] if entity_type else "concept"
            domain = (entity.get("domains") or ["general"])[0] if isinstance(
                entity.get("domains"), list) else "general"
            attrs = entity.get("attributes", {})
            complexity = "complex" if isinstance(attrs, dict) and len(attrs) > 3 else "simple"

            threshold = ftrl.predict_threshold(
                entity_type=entity_type,
                relation_type="relates_to",
                domain=domain,
                complexity=complexity,
            )

            if conf >= threshold:
                entity["_auto_status"] = "confirmed"
            elif conf >= threshold * 0.5:
                entity["_auto_status"] = "low_confidence"
                entity["tags"] = entity.get("tags", []) + ["[低置信度]"]
            else:
                entity["_auto_status"] = "pending_review"

        return extraction_result

    def _timeout_result(self, reason: str, start_time: float) -> dict:
        """Build escalation result for timeout scenarios (V2.2)."""
        elapsed_ms = (time.monotonic() - start_time) * 1000
        return {
            "escalated": True,
            "reason": f"timeout:{reason}",
            "elapsed_ms": elapsed_ms,
            "entities": [],
            "edges": [],
            "count": 0,
            "message": "端侧处理超时，建议升舱至云端处理",
        }

    # ------------------------------------------------------------------
    # LLM extraction
    # ------------------------------------------------------------------

    def _llm_extract(
        self, user_message: str, assistant_message: str, session_id: str,
        namespace: str = "general",
    ) -> dict:
        prompt = (
            f"User message:\n{user_message[:2000]}\n\n"
            f"Assistant response:\n{assistant_message[:2000]}"
        )
        response = self._extract_fn(EXTRACTION_SYSTEM_PROMPT, prompt)
        data = self._parse_response(response)
        return self._persist_extracted(data, session_id, namespace)

    def _parse_response(self, response: str) -> dict:
        """Parse LLM extraction response, handling JSON in markdown fences."""
        text = response.strip()
        if text.startswith("```"):
            lines = text.split("\n")
            text = "\n".join(lines[1:]) if len(lines) > 1 else text
            if text.endswith("```"):
                text = text[:-3]
        try:
            return json.loads(text)
        except json.JSONDecodeError:
            logger.debug("Failed to parse extraction response as JSON")
            return {"entities": [], "edges": []}

    def _persist_extracted(self, data: dict, session_id: str, namespace: str = "general") -> dict:
        """Write extracted entities and edges to the store (V2.3: + namespace)."""
        entities = data.get("entities", [])[: self.max_entities_per_turn]
        edges = data.get("edges", [])[: self.max_entities_per_turn * 2]

        entity_map: dict[str, str] = {}  # name -> entity_id
        entity_type_map: dict[str, str] = {}  # name -> entity type
        persisted_entities = []

        for ent in entities:
            name = ent.get("name", "").strip()
            if not name:
                continue
            if _is_junk_entity_name(name):
                continue
            conf = float(ent.get("confidence", 0.5))
            if conf < self.confidence_threshold:
                continue
            etype = ent.get("type", "concept")
            if etype not in {"person", "organization", "project", "tool", "concept", "skill", "event", "location"}:
                etype = "concept"
            attrs = ent.get("attributes", {}) if isinstance(ent.get("attributes"), dict) else {}

            # Write to old heimdall_entities table
            eid = self.store.upsert_entity(
                display_name=name,
                entity_type=etype,
                source_session_id=session_id,
                source_track="memory",
                confidence=conf,
                attributes=attrs,
            )
            entity_map[name] = eid
            entity_type_map[name] = etype
            persisted_entities.append({"name": name, "entity_id": eid, "type": etype})

            # Dual-write to Knowledge Ring kr_entities (new 5-type schema)
            self._persist_entity_v2(name, etype, attrs, conf, session_id, namespace)

        for edge in edges:
            src = edge.get("source", "").strip()
            tgt = edge.get("target", "").strip()
            if not src or not tgt:
                continue
            src_id = entity_map.get(src)
            tgt_id = entity_map.get(tgt)
            if not src_id or not tgt_id:
                continue
            role = edge.get("role", "context")
            if role not in {"subject", "object", "context"}:
                role = "context"
            emotion = float(edge.get("emotion", 0.0))
            self.store.add_memory_edge(
                entity_id=tgt_id if role == "object" else src_id,
                role=role,
                emotion=max(-1.0, min(1.0, emotion)),
                session_id=session_id,
            )
            if src_id != tgt_id:
                self.store.upsert_social_edge(
                    source_entity_id=src_id,
                    target_entity_id=tgt_id,
                    relationship_type="mentioned_with",
                    emotion=emotion,
                )

        # Dual-write relations and aliases from Knowledge Ring extraction
        kr_relations = data.get("relations", [])
        for rel in kr_relations:
            src_name = rel.get("source", "").strip()
            tgt_name = rel.get("target", "").strip()
            if not src_name or not tgt_name:
                continue
            src_v2 = self.store.get_entity_v2_by_name(src_name)
            tgt_v2 = self.store.get_entity_v2_by_name(tgt_name)
            if not src_v2 or not tgt_v2:
                # Try finding by creating placeholder
                src_v2 = self._ensure_entity_v2(src_name, entity_type_map.get(src_name, "concept"), session_id)
                tgt_v2 = self._ensure_entity_v2(tgt_name, entity_type_map.get(tgt_name, "concept"), session_id)
            if src_v2 and tgt_v2:
                rel_type = rel.get("type", "relates_to")
                if rel_type in {"belongs_to", "contains", "relates_to", "contrasts_with",
                                "causes", "produces", "inspired_by", "knows"}:
                    self.store.add_relation(
                        source_id=src_v2["entity_id"],
                        target_id=tgt_v2["entity_id"],
                        rel_type=rel_type,
                        confidence=float(rel.get("confidence", 0.5)),
                        direction=rel.get("direction", "bidirectional"),
                        source_text=rel.get("source_text", ""),
                        session_id=session_id,
                        namespace=namespace,
                    )

        kr_aliases = data.get("aliases", [])
        for alias in kr_aliases:
            entity_name = alias.get("entity_name", "").strip()
            alias_name = alias.get("alias", "").strip()
            if not entity_name or not alias_name:
                continue
            entity_v2 = self.store.get_entity_v2_by_name(entity_name)
            if entity_v2:
                self.store.add_alias(
                    entity_id=entity_v2["entity_id"],
                    name=alias_name,
                    context=alias.get("context", ""),
                )

        domains = data.get("domains", [])
        for domain in domains:
            if domain and isinstance(domain, str):
                self.store.register_domain(domain_name=domain)

        return {"entities": persisted_entities, "edges": edges, "count": len(persisted_entities)}

    def _persist_entity_v2(
        self, name: str, old_type: str, attrs: dict, confidence: float, session_id: str,
        namespace: str = "general",
    ) -> Optional[str]:
        """Write a single entity to the Knowledge Ring kr_entities table using 5-type schema."""
        from heimdall.core.entity_store import ENTITY_TYPE_MIGRATION_MAP
        new_type, type_detail = ENTITY_TYPE_MIGRATION_MAP.get(old_type, ("concept", old_type))
        try:
            return self.store.upsert_entity_v2(
                name=name,
                entity_type=new_type,
                type_detail=type_detail,
                properties=attrs if attrs else {},
                confidence=confidence,
                session_id=session_id,
                namespace=namespace,
            )
        except Exception:
            logger.debug("Failed to persist entity v2: %s", name, exc_info=True)
            return None

    def _ensure_entity_v2(self, name: str, old_type: str = "concept", session_id: str = "",
                         namespace: str = "general") -> Optional[dict]:
        """Get or create a Knowledge Ring entity by name."""
        entity = self.store.get_entity_v2_by_name(name)
        if entity:
            return entity
        eid = self._persist_entity_v2(name, old_type, {}, 0.5, session_id, namespace)
        if eid:
            return self.store.get_entity_v2(eid)
        return None

    # ------------------------------------------------------------------
    # Rule-based fallback
    # ------------------------------------------------------------------

    def _rule_based_extract(
        self, user_message: str, assistant_message: str, session_id: str,
        namespace: str = "general",
    ) -> dict:
        """Simple rule-based extraction as fallback when LLM is unavailable.

        Detects capitalized proper nouns and quoted strings as candidate entities.
        Also creates co-occurrence edges between entities found in the same turn.
        Dual-writes to both old and Knowledge Ring tables.
        """
        import re
        combined = f"{user_message}\n{assistant_message}"
        entities = []
        entity_map: dict[str, str] = {}

        # Simple proper noun heuristic (English + Chinese)
        proper_patterns = [
            r'\b[A-Z][a-z]+(?:\s[A-Z][a-z]+)*\b',  # English proper nouns
            r'["""]([^"""]+)["”]',  # Quoted terms
            r'[一-鿿]{2,4}(?:老师|先生|女士|同事|朋友|家人|老板)',  # Chinese person refs
        ]
        seen = set()
        for pattern in proper_patterns:
            for match in re.finditer(pattern, combined):
                name = match.group(1) if match.lastindex else match.group(0)
                name = name.strip()
                if name and name not in seen and len(name) > 1:
                    seen.add(name)
                    if len(entities) >= self.max_entities_per_turn:
                        break
                    eid = self.store.upsert_entity(
                        display_name=name,
                        entity_type="concept",
                        source_session_id=session_id,
                        confidence=0.4,
                    )
                    entity_map[name] = eid
                    entities.append({"name": name, "entity_id": eid, "type": "concept"})
                    # Dual-write to Knowledge Ring
                    self._persist_entity_v2(name, "concept", {}, 0.4, session_id, namespace)

        # Create co-occurrence edges between entities in the same turn
        edges = []
        entity_names = list(entity_map.keys())
        for i in range(len(entity_names)):
            for j in range(i + 1, len(entity_names)):
                src_name = entity_names[i]
                tgt_name = entity_names[j]
                src_id = entity_map[src_name]
                tgt_id = entity_map[tgt_name]
                try:
                    self.store.add_memory_edge(
                        entity_id=src_id, role="context", emotion=0.0, session_id=session_id,
                    )
                    self.store.add_memory_edge(
                        entity_id=tgt_id, role="context", emotion=0.0, session_id=session_id,
                    )
                except Exception:
                    pass
                if src_id != tgt_id:
                    try:
                        self.store.upsert_social_edge(
                            source_entity_id=src_id,
                            target_entity_id=tgt_id,
                            relationship_type="mentioned_with",
                            emotion=0.0,
                        )
                    except Exception:
                        pass
                edges.append({
                    "source": src_name, "target": tgt_name,
                    "role": "context", "emotion": 0.0,
                })
                # Dual-write relation to Knowledge Ring
                src_v2 = self.store.get_entity_v2_by_name(src_name)
                tgt_v2 = self.store.get_entity_v2_by_name(tgt_name)
                if src_v2 and tgt_v2:
                    try:
                        self.store.add_relation(
                            source_id=src_v2["entity_id"],
                            target_id=tgt_v2["entity_id"],
                            rel_type="relates_to",
                            confidence=0.3,
                            session_id=session_id,
                            namespace=namespace,
                        )
                    except Exception:
                        pass

        return {"entities": entities, "edges": edges, "count": len(entities)}
