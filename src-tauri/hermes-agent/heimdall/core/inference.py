"""Transitive inference engine — graph-based relationship discovery.

Pure graph algorithm (no LLM). Discovers implied relationships by analyzing
shared neighbors in the entity graph. Runs after entity extraction completes.
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

# Transitive relation inference rules:
# If A has relation R1 to C and B has relation R2 to C,
# then A and B might have an inferred relationship.
_INFERENCE_RULES: dict[tuple[str, ...], str] = {
    ("knows", "knows"): "knows",
    ("belongs_to", "belongs_to"): "relates_to",
    ("contains", "contains"): "relates_to",
    ("produces", "produces"): "relates_to",
    ("causes", "causes"): "causes",
    ("inspired_by", "inspired_by"): "inspired_by",
    ("relates_to", "relates_to"): "relates_to",
}

# Default fallback when no specific rule matches
_DEFAULT_INFERRED_TYPE = "relates_to"


@dataclass
class Inference:
    """A candidate inferred relationship between two entities."""
    inference_id: str
    entity_a: str
    entity_b: str
    inferred_type: str
    evidence: list[dict[str, str]]  # [{shared_neighbor, relation_type_a, relation_type_b}]
    confidence: float = 0.3
    status: str = "pending"
    namespace: str = "general"


class TransitiveInferenceEngine:
    """Discover implied relationships through shared-neighbor analysis."""

    def __init__(self, store):
        self._store = store

    # ------------------------------------------------------------------
    # Public API
    # ------------------------------------------------------------------

    def run(
        self,
        namespace: str = "general",
        min_shared_neighbors: int = 2,
    ) -> list[Inference]:
        """Run transitive inference over a namespace.

        Returns a list of candidate inferences for entity pairs that share
        at least *min_shared_neighbors* common neighbors but lack a direct
        edge.
        """
        relations = self._load_relations(namespace)
        if not relations:
            return []

        adjacency = self._build_adjacency(relations)
        existing_edges = self._build_edge_set(relations)
        candidates = self._find_shared_neighbor_pairs(
            adjacency, existing_edges, min_shared_neighbors
        )

        inferences: list[Inference] = []
        for entity_a, entity_b, shared in candidates:
            inferred_type = self._infer_type(adjacency, entity_a, entity_b, shared)
            evidence = [
                {
                    "shared_neighbor": neighbor_id,
                    "relation_type_a": adjacency[entity_a][neighbor_id],
                    "relation_type_b": adjacency[entity_b][neighbor_id],
                }
                for neighbor_id in shared
            ]
            inf = Inference(
                inference_id=_gen_inference_id(entity_a, entity_b),
                entity_a=entity_a,
                entity_b=entity_b,
                inferred_type=inferred_type,
                evidence=evidence,
                namespace=namespace,
            )
            inferences.append(inf)

        logger.info(
            "Inference run complete: %d candidates found (namespace=%s, min_shared=%d)",
            len(inferences), namespace, min_shared_neighbors,
        )
        return inferences

    def confirm(self, inference_id: str) -> None:
        """Promote a pending inference to a confirmed relation."""
        self._update_status(inference_id, "confirmed")

    def reject(self, inference_id: str) -> None:
        """Reject a candidate inference."""
        self._update_status(inference_id, "rejected")

    # ------------------------------------------------------------------
    # Internal
    # ------------------------------------------------------------------

    def _load_relations(self, namespace: str) -> list[dict]:
        """Load all relations for a namespace from the store."""
        conn = self._store._conn
        rows = conn.execute(
            "SELECT source_id, target_id, type FROM kr_relations WHERE namespace = ?",
            (namespace,),
        ).fetchall()
        return [{"source": r[0], "target": r[1], "type": r[2]} for r in rows]

    def _build_adjacency(
        self, relations: list[dict]
    ) -> dict[str, dict[str, str]]:
        """Build undirected adjacency: entity_id -> {neighbor_id: relation_type}."""
        adj: dict[str, dict[str, str]] = defaultdict(dict)
        for rel in relations:
            src, tgt, rtype = rel["source"], rel["target"], rel["type"]
            adj[src][tgt] = rtype
            adj[tgt][src] = rtype
        return dict(adj)

    def _build_edge_set(self, relations: list[dict]) -> set[frozenset[str]]:
        """Return the set of undirected edges that already exist."""
        return {frozenset((r["source"], r["target"])) for r in relations}

    def _find_shared_neighbor_pairs(
        self,
        adjacency: dict[str, dict[str, str]],
        existing_edges: set[frozenset[str]],
        min_shared: int,
    ) -> list[tuple[str, str, list[str]]]:
        """Find entity pairs with shared neighbors but no direct edge."""
        entities = list(adjacency.keys())
        candidates: list[tuple[str, str, list[str]]] = []

        for i in range(len(entities)):
            for j in range(i + 1, len(entities)):
                a, b = entities[i], entities[j]
                if frozenset((a, b)) in existing_edges:
                    continue
                shared = [
                    n for n in adjacency[a]
                    if n in adjacency[b]
                ]
                if len(shared) >= min_shared:
                    candidates.append((a, b, shared))

        return candidates

    def _infer_type(
        self,
        adjacency: dict[str, dict[str, str]],
        entity_a: str,
        entity_b: str,
        shared_neighbors: list[str],
    ) -> str:
        """Determine the inferred relation type from shared-neighbor patterns."""
        type_votes: dict[str, int] = defaultdict(int)
        for neighbor_id in shared_neighbors:
            ra = adjacency[entity_a][neighbor_id]
            rb = adjacency[entity_b][neighbor_id]
            key = tuple(sorted((ra, rb)))
            inferred = _INFERENCE_RULES.get(key, _DEFAULT_INFERRED_TYPE)
            type_votes[inferred] += 1

        if not type_votes:
            return _DEFAULT_INFERRED_TYPE
        return max(type_votes, key=lambda k: type_votes[k])

    def _update_status(self, inference_id: str, status: str) -> None:
        """Update inference status in the database."""
        conn = self._store._conn
        conn.execute(
            "UPDATE kr_inferences SET status = ?, resolved_at = ? WHERE inference_id = ?",
            (status, time.strftime("%Y-%m-%dT%H:%M:%S"), inference_id),
        )
        conn.commit()


def _gen_inference_id(entity_a: str, entity_b: str) -> str:
    ns = uuid.UUID("6ba7b810-9dad-11d1-80b4-00c04fd430c8")
    raw = "|".join(sorted([entity_a, entity_b]))
    return str(uuid.uuid5(ns, raw))
