"""Hypothesis generation — identify knowledge gaps in the entity graph.

Analyzes the graph to find entity pairs that "should" have a relation
based on structural similarity but don't. Generates hypotheses for the
user to confirm or dismiss.
"""

from __future__ import annotations

import json
import logging
import time
import uuid
from collections import defaultdict
from dataclasses import dataclass, field
from typing import Any, Optional

logger = logging.getLogger(__name__)

# Common relation types that entities of the same domain often share
_COMMON_RELATION_SIGNATURES: dict[str, list[str]] = {
    "concept": ["relates_to", "contrasts_with", "inspired_by"],
    "tool": ["relates_to", "depends_on", "produces"],
    "person": ["knows", "relates_to"],
    "event": ["causes", "preceded_by", "relates_to"],
    "project": ["relates_to", "depends_on", "produces"],
}


@dataclass
class RelationGap:
    """A missing relation hypothesis for an entity pair."""
    entity_id: str
    entity_name: str
    missing_relation_type: str
    suggestion: str
    confidence: float = 0.3


@dataclass
class Hypothesis:
    """A generated hypothesis about a missing relation."""
    hypothesis_id: str
    entity_a: str
    entity_b: str
    suggested_relation: str
    reasoning: str
    confidence: float
    namespace: str
    status: str = "pending"
    created_at: str = ""


class HypothesisEngine:
    """Generate hypotheses about missing relations in the knowledge graph."""

    def __init__(self, store):
        self._store = store

    # ------------------------------------------------------------------
    # Public API
    # ------------------------------------------------------------------

    def find_gaps(
        self,
        namespace: str = "general",
        entity_type: Optional[str] = None,
    ) -> list[RelationGap]:
        """Find entities missing common relation types for their entity type.

        For each entity of the given type (or all types), check which of
        the common relation types are absent and flag them as gaps.
        """
        entities = self._load_entities(namespace, entity_type)
        existing_relations = self._load_entity_relation_counts(namespace)

        gaps: list[RelationGap] = []
        for e in entities:
            eid = e["entity_id"]
            ename = e["name"]
            etype = e.get("types", "concept")
            if isinstance(etype, str):
                try:
                    etype = json.loads(etype)
                except Exception:
                    etype = ["concept"]
            primary_type = etype[0] if etype else "concept"

            expected = _COMMON_RELATION_SIGNATURES.get(primary_type, ["relates_to"])
            existing = existing_relations.get(eid, set())

            for rel_type in expected:
                if rel_type not in existing:
                    gaps.append(RelationGap(
                        entity_id=eid,
                        entity_name=ename,
                        missing_relation_type=rel_type,
                        suggestion=f"Add a '{rel_type}' relation connecting '{ename}' to related entities",
                    ))

        logger.info(
            "Gap analysis: %d gaps found (namespace=%s, type=%s)",
            len(gaps), namespace, entity_type or "all",
        )
        return gaps

    def generate(
        self,
        namespace: str = "general",
        limit: int = 20,
    ) -> list[Hypothesis]:
        """Generate hypotheses for entity pairs that share domain but lack relations."""
        entities = self._load_entities(namespace)
        entity_domains = self._build_domain_index(entities)
        existing_edges = self._load_edge_set(namespace)

        now = time.strftime("%Y-%m-%dT%H:%M:%S")
        hypotheses: list[Hypothesis] = []

        for domain, eids in entity_domains.items():
            if len(eids) < 2:
                continue
            for i in range(len(eids)):
                for j in range(i + 1, len(eids)):
                    a, b = eids[i], eids[j]
                    if frozenset((a, b)) in existing_edges:
                        continue

                    # Suggest most common relation for that domain context
                    h = Hypothesis(
                        hypothesis_id=_gen_hypothesis_id(a, b, domain),
                        entity_a=a,
                        entity_b=b,
                        suggested_relation="relates_to",
                        reasoning=f"Both entities belong to domain '{domain}' but have no direct relation",
                        confidence=0.3,
                        namespace=namespace,
                        created_at=now,
                    )
                    hypotheses.append(h)
                    if len(hypotheses) >= limit:
                        break
                if len(hypotheses) >= limit:
                    break
            if len(hypotheses) >= limit:
                break

        logger.info("Generated %d hypotheses for namespace=%s", len(hypotheses), namespace)
        return hypotheses

    def dismiss(self, hypothesis_id: str) -> None:
        """Mark a hypothesis as dismissed."""
        conn = self._store._conn
        conn.execute(
            "UPDATE kr_inferences SET status = 'rejected', resolved_at = ? WHERE inference_id = ?",
            (time.strftime("%Y-%m-%dT%H:%M:%S"), hypothesis_id),
        )
        conn.commit()

    # ------------------------------------------------------------------
    # Internal
    # ------------------------------------------------------------------

    def _load_entities(
        self, namespace: str, entity_type: Optional[str] = None
    ) -> list[dict]:
        conn = self._store._conn
        if entity_type:
            rows = conn.execute(
                """SELECT entity_id, name, types, domains FROM kr_entities
                   WHERE namespace = ? AND types LIKE ?""",
                (namespace, f'%"{entity_type}"%'),
            ).fetchall()
        else:
            rows = conn.execute(
                "SELECT entity_id, name, types, domains FROM kr_entities WHERE namespace = ?",
                (namespace,),
            ).fetchall()
        return [dict(r) for r in rows]

    def _load_entity_relation_counts(
        self, namespace: str
    ) -> dict[str, set[str]]:
        conn = self._store._conn
        rows = conn.execute(
            """SELECT source_id, type FROM kr_relations WHERE namespace = ?
               UNION SELECT target_id, type FROM kr_relations WHERE namespace = ?""",
            (namespace, namespace),
        ).fetchall()
        counts: dict[str, set[str]] = defaultdict(set)
        for r in rows:
            counts[r[0]].add(r[1])
        return dict(counts)

    def _load_edge_set(self, namespace: str) -> set[frozenset[str]]:
        conn = self._store._conn
        rows = conn.execute(
            "SELECT source_id, target_id FROM kr_relations WHERE namespace = ?",
            (namespace,),
        ).fetchall()
        return {frozenset((r[0], r[1])) for r in rows}

    def _build_domain_index(self, entities: list[dict]) -> dict[str, list[str]]:
        idx: dict[str, list[str]] = defaultdict(list)
        for e in entities:
            domains = e.get("domains", "")
            if domains:
                try:
                    domain_list = json.loads(domains) if isinstance(domains, str) else domains
                except Exception:
                    domain_list = []
                for d in domain_list:
                    idx[d].append(e["entity_id"])
        return dict(idx)


def _gen_hypothesis_id(entity_a: str, entity_b: str, context: str) -> str:
    ns = uuid.UUID("6ba7b810-9dad-11d1-80b4-00c04fd430c8")
    raw = "|".join(sorted([entity_a, entity_b]) + [context])
    return str(uuid.uuid5(ns, raw))
