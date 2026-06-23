"""A2A Escalation Protocol (V2.2) — structured summary + pointer, no truncation.

When local extraction is insufficient (low confidence, unknown domain, timeouts),
the A2A client builds a payload that:
  1. Sends structured summary (entities/relations extracted locally)
  2. Sends original pointer (hash + length, not full text)
  3. Sends specific fragments needing refinement
"""

import hashlib
import logging
from typing import Any, Optional

logger = logging.getLogger(__name__)


class A2AEscalationClient:
    """A2A escalation protocol client (V2.2: structured summary + pointer).

    Builds payloads that preserve full context without truncating original text.
    Cloud returns fine-grained relations and refined entity types.
    """

    def __init__(
        self,
        session_id: str = "",
        platform: str = "cli",
        agent_id: str = "",
        max_entities_local: int = 20,
    ):
        self._session_id = session_id
        self._platform = platform
        self._agent_id = agent_id or f"heimdall-{session_id[:8] if session_id else 'unknown'}"
        self.max_entities_local = max_entities_local

    # ------------------------------------------------------------------
    # Payload building
    # ------------------------------------------------------------------

    def build_payload(
        self,
        input_text: str,
        local_extraction: dict,
        reason: str,
    ) -> dict:
        """Build A2A escalation payload (V2.2: no truncation).

        Args:
            input_text: original user+assistant text (full, untruncated)
            local_extraction: whatever was extracted locally before timeout/escalation
            reason: why we're escalating (timeout, low_confidence, unknown_domain, etc.)

        Returns:
            A2A protocol payload dict
        """
        entities = local_extraction.get("entities", [])
        relations = local_extraction.get("relations", [])

        # 1. Structured summary — what the local model already extracted
        structured_summary = {
            "entity_count": len(entities),
            "entities": [
                {
                    "name": e.get("name", ""),
                    "types": e.get("types", ["concept"]),
                    "type_detail": e.get("type_detail", ""),
                    "confidence": e.get("confidence", 0.5),
                }
                for e in entities[:self.max_entities_local]
            ],
            "relations": [
                {
                    "source": r.get("source", ""),
                    "target": r.get("target", ""),
                    "type": r.get("type", "relates_to"),
                    "confidence": r.get("confidence", 0.5),
                }
                for r in relations[:self.max_entities_local]
            ],
        }

        # 2. Original pointer (hash + length, NOT the full text)
        original_ref = local_extraction.get("original_ref")
        original_pointer = None
        if original_ref:
            original_pointer = {
                "local_db_ref": original_ref,
                "content_hash": self._hash_text(input_text),
                "length": len(input_text),
            }

        # 3. Fragments needing refinement (low-confidence items)
        needs_refinement = []
        # Low-confidence entities
        for e in entities:
            if e.get("confidence", 0.5) < 0.7:
                needs_refinement.append({
                    "type": "entity",
                    "name": e.get("name", ""),
                    "source_text": e.get("source_text", ""),
                    "coarse_types": e.get("types", ["concept"]),
                    "reason": "low_confidence",
                })
        # Low-confidence relations
        for r in relations:
            if r.get("confidence", 0.5) < 0.7:
                needs_refinement.append({
                    "type": "relation",
                    "source": r.get("source", ""),
                    "target": r.get("target", ""),
                    "coarse_type": r.get("type", "relates_to"),
                    "reason": "low_confidence",
                })

        # 4. Required capabilities for the cloud
        required_capabilities = ["relation_extraction_fine"]
        if reason == "timeout":
            required_capabilities.append("entity_extraction_full")
        if reason == "unknown_domain":
            required_capabilities.append("domain_classification")
        if reason == "long_chain":
            required_capabilities.append("long_chain_reasoning")

        return {
            "protocol": "a2a/v2",
            "sender": self._agent_id,
            "capability_boundary": {
                "local_done": structured_summary,
                "escalation_reason": reason,
                "required_capability": required_capabilities,
            },
            "original_pointer": original_pointer,
            "needs_refinement": needs_refinement,
            "session_context": {
                "session_id": self._session_id,
                "platform": self._platform,
            },
        }

    def build_timeout_payload(
        self, input_text: str, partial: Optional[dict] = None, elapsed_ms: float = 0
    ) -> dict:
        """Build escalation payload specifically for timeout scenarios."""
        partial = partial or {"entities": [], "relations": []}
        return self.build_payload(
            input_text=input_text,
            local_extraction=partial,
            reason=f"timeout:{elapsed_ms:.0f}ms",
        )

    def build_low_confidence_payload(
        self, input_text: str, extraction: dict
    ) -> dict:
        """Build escalation payload for low-confidence extraction scenarios."""
        return self.build_payload(
            input_text=input_text,
            local_extraction=extraction,
            reason="low_confidence",
        )

    def parse_cloud_response(self, response: dict) -> dict:
        """Parse cloud A2A response and merge with local extraction.

        Cloud returns:
          - relations_fine: fine-grained relations with 8-type classification
          - entity_refinements: refined entity types, domains
          - summary: human-readable summary of what was improved
        """
        result = {
            "relations_fine": response.get("relations_fine", []),
            "entity_refinements": response.get("entity_refinements", []),
            "summary": response.get("summary", ""),
            "merged_count": 0,
        }

        fine_relations = response.get("relations_fine", [])
        result["merged_count"] = len(fine_relations)

        for rel in fine_relations:
            logger.debug(
                "Cloud refined relation: %s -[%s]-> %s (confidence: %.2f)",
                rel.get("source", ""), rel.get("type", ""),
                rel.get("target", ""), rel.get("confidence", 0.0),
            )

        return result

    # ------------------------------------------------------------------
    # Helpers
    # ------------------------------------------------------------------

    @staticmethod
    def _hash_text(text: str) -> str:
        return hashlib.sha256(text.encode()).hexdigest()[:16]

    @property
    def session_id(self) -> str:
        return self._session_id

    @session_id.setter
    def session_id(self, value: str) -> None:
        self._session_id = value
