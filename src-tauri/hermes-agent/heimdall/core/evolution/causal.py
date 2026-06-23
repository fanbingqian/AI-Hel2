"""Causal chain builder — BFS-based causal path discovery.

Builds multi-hop causal chains from individual causal relations.
Triggered after new relations are added to the knowledge graph.

Causal relation types that participate in chain building:
  causes, produces, triggers, results_in, leads_to,
  depends_on, requires, blocks,
  preceded_by, followed_by
"""

from __future__ import annotations

import json
import logging
import math
import time
import uuid
from collections import deque
from dataclasses import dataclass, field
from typing import Optional

logger = logging.getLogger(__name__)

# Relation types that can form causal chains
CAUSAL_FORWARD_TYPES = frozenset({
    "causes", "produces", "triggers", "results_in", "leads_to",
})

CAUSAL_BACKWARD_TYPES = frozenset({
    "depends_on", "requires", "preceded_by",
})

CAUSAL_BIDIRECTIONAL_TYPES = frozenset({
    "blocks", "followed_by",
})

ALL_CAUSAL_TYPES = CAUSAL_FORWARD_TYPES | CAUSAL_BACKWARD_TYPES | CAUSAL_BIDIRECTIONAL_TYPES

# Default edge weights used in chain scoring
DEFAULT_EDGE_WEIGHTS: dict[str, float] = {
    "causes": 0.9,
    "leads_to": 0.8,
    "produces": 0.75,
    "triggers": 0.75,
    "results_in": 0.7,
    "depends_on": 0.7,
    "requires": 0.65,
    "preceded_by": 0.5,
    "followed_by": 0.5,
    "blocks": 0.6,
}

MAX_HOPS = 5
MIN_CHAIN_LENGTH = 3


@dataclass
class CausalChain:
    """A multi-hop causal path through the knowledge graph."""
    chain_id: str
    chain_path: list[dict[str, str]]  # [{entity_id, relation_id}]
    length: int
    chain_score: float
    namespace: str
    first_seen: str
    last_updated: str
    is_active: bool = True


class CausalChainBuilder:
    """Build and score multi-hop causal chains from pairwise relations."""

    def __init__(self, store):
        self._store = store

    # ------------------------------------------------------------------
    # Public API
    # ------------------------------------------------------------------

    def build_chain_from(
        self,
        source_id: str,
        target_id: str,
        relation_type: str,
        namespace: str = "general",
    ) -> list[CausalChain]:
        """Build causal chains that pass through a newly added relation.

        Strategy:
          - Backward BFS from source_id (following reverse causal edges)
          - Forward BFS from target_id (following forward causal edges)
          - Merge: reverse_backward_path + [source→target] + forward_path
          - Keep chains of length >= MIN_CHAIN_LENGTH.
        """
        if relation_type not in ALL_CAUSAL_TYPES:
            return []

        adj_forward, adj_backward = self._build_causal_adjacency(namespace)

        # Backward: walk from source against edge direction
        backward_paths = self._bfs_backward(source_id, adj_backward, MAX_HOPS)

        # Forward: walk from target along edge direction
        forward_paths = self._bfs_forward(target_id, adj_forward, MAX_HOPS)

        chains: list[CausalChain] = []
        now = time.strftime("%Y-%m-%dT%H:%M:%S")

        for bpath in backward_paths:
            for fpath in forward_paths:
                merged = bpath + [
                    {"entity_id": target_id, "relation_id": f"{source_id}→{target_id}:{relation_type}"},
                ] + fpath if bpath else [
                    {"entity_id": source_id, "relation_id": f"{source_id}→{target_id}:{relation_type}"},
                ] + fpath

                if not bpath:
                    merged = [
                        {"entity_id": source_id, "relation_id": f"{source_id}→{target_id}:{relation_type}"},
                    ] + fpath

                if len(merged) < MIN_CHAIN_LENGTH:
                    continue

                score = self.score_chain(merged, adj_forward)

                chain = CausalChain(
                    chain_id=self._gen_chain_id(merged),
                    chain_path=merged,
                    length=len(merged),
                    chain_score=score,
                    namespace=namespace,
                    first_seen=now,
                    last_updated=now,
                )
                chains.append(chain)

        # Deduplicate by chain_id
        seen: set[str] = set()
        unique: list[CausalChain] = []
        for c in chains:
            if c.chain_id not in seen:
                seen.add(c.chain_id)
                unique.append(c)

        logger.info(
            "Causal chain build: relation=%s→%s (%s), chains=%d",
            source_id, target_id, relation_type, len(unique),
        )
        return unique

    def score_chain(
        self,
        chain_path: list[dict[str, str]],
        adjacency: dict[str, dict[str, str]],
    ) -> float:
        """Score a causal chain: product of edge weights * log(1 + length)."""
        weight_product = 1.0
        for step in chain_path:
            rel_id = step.get("relation_id", "")
            rel_type = rel_id.split(":")[-1] if ":" in rel_id else ""
            weight = DEFAULT_EDGE_WEIGHTS.get(rel_type, 0.5)
            weight_product *= weight
        return weight_product * math.log(1 + len(chain_path))

    def find_significant_chains(
        self,
        namespace: str = "general",
        min_score: float = 0.3,
    ) -> list[dict]:
        """Query active chains above a score threshold."""
        conn = self._store._conn
        rows = conn.execute(
            """SELECT * FROM kr_causal_chains
               WHERE namespace = ? AND is_active = 1 AND chain_score >= ?
               ORDER BY chain_score DESC""",
            (namespace, min_score),
        ).fetchall()
        return [dict(r) for r in rows]

    def rebuild_all(self, namespace: str = "general") -> list[CausalChain]:
        """Re-scan all causal relations and rebuild chains from scratch."""
        conn = self._store._conn
        rows = conn.execute(
            """SELECT source_id, target_id, type FROM kr_relations
               WHERE namespace = ? AND type IN (
                'causes','produces','triggers','results_in','leads_to',
                'depends_on','requires','preceded_by','followed_by','blocks'
               )""",
            (namespace,),
        ).fetchall()

        all_chains: list[CausalChain] = []
        for row in rows:
            chains = self.build_chain_from(row[0], row[1], row[2], namespace)
            all_chains.extend(chains)

        # Deactivate old chains for this namespace and insert new ones
        conn.execute(
            "UPDATE kr_causal_chains SET is_active = 0 WHERE namespace = ?",
            (namespace,),
        )
        for chain in all_chains:
            self._upsert_chain(chain)
        conn.commit()

        logger.info("Rebuilt %d causal chains for namespace=%s", len(all_chains), namespace)
        return all_chains

    # ------------------------------------------------------------------
    # Internal — BFS traversal
    # ------------------------------------------------------------------

    def _build_causal_adjacency(
        self, namespace: str
    ) -> tuple[dict[str, dict[str, str]], dict[str, dict[str, str]]]:
        """Build forward and backward causal adjacency maps."""
        conn = self._store._conn
        rows = conn.execute(
            """SELECT source_id, target_id, type FROM kr_relations
               WHERE namespace = ?""",
            (namespace,),
        ).fetchall()

        forward: dict[str, dict[str, str]] = {}
        backward: dict[str, dict[str, str]] = {}

        for src, tgt, rtype in rows:
            if rtype in CAUSAL_FORWARD_TYPES:
                forward.setdefault(src, {})[tgt] = rtype
                backward.setdefault(tgt, {})[src] = rtype
            elif rtype in CAUSAL_BACKWARD_TYPES:
                backward.setdefault(src, {})[tgt] = rtype
                forward.setdefault(tgt, {})[src] = rtype
            elif rtype in CAUSAL_BIDIRECTIONAL_TYPES:
                forward.setdefault(src, {})[tgt] = rtype
                forward.setdefault(tgt, {})[src] = rtype
                backward.setdefault(src, {})[tgt] = rtype
                backward.setdefault(tgt, {})[src] = rtype

        return forward, backward

    def _bfs_forward(
        self, start: str, adj: dict[str, dict[str, str]], max_hops: int
    ) -> list[list[dict[str, str]]]:
        """BFS forward from start, returning all paths up to max_hops."""
        paths: list[list[dict[str, str]]] = []
        queue: deque[tuple[str, list[dict[str, str]]]] = deque()
        queue.append((start, []))
        visited_depth: dict[str, int] = {start: 0}

        while queue:
            current, path = queue.popleft()
            depth = len(path)
            if depth >= max_hops:
                continue

            for neighbor, rtype in adj.get(current, {}).items():
                new_step = {
                    "entity_id": neighbor,
                    "relation_id": f"{current}→{neighbor}:{rtype}",
                }
                new_path = path + [new_step]
                if len(new_path) >= 2:
                    paths.append(new_path)
                prev_depth = visited_depth.get(neighbor, 999)
                if depth + 1 < prev_depth:
                    visited_depth[neighbor] = depth + 1
                    queue.append((neighbor, new_path))

        return paths

    def _bfs_backward(
        self, start: str, adj: dict[str, dict[str, str]], max_hops: int
    ) -> list[list[dict[str, str]]]:
        """BFS backward from start, returning reversed paths."""
        raw_paths = self._bfs_forward(start, adj, max_hops)
        reversed_paths: list[list[dict[str, str]]] = []
        for path in raw_paths:
            rev = list(reversed(path))
            reversed_paths.append(rev)
        return reversed_paths

    def _gen_chain_id(self, chain_path: list[dict[str, str]]) -> str:
        """Generate a stable chain ID from the ordered entity path."""
        ns = uuid.UUID("6ba7b810-9dad-11d1-80b4-00c04fd430c8")
        key = "→".join(step["entity_id"] for step in chain_path)
        return str(uuid.uuid5(ns, key))

    def _upsert_chain(self, chain: CausalChain) -> None:
        """Insert or replace a causal chain in the database."""
        conn = self._store._conn
        conn.execute(
            """INSERT OR REPLACE INTO kr_causal_chains
               (chain_id, chain_path, length, chain_score, namespace,
                first_seen, last_updated, is_active)
               VALUES (?, ?, ?, ?, ?, ?, ?, ?)""",
            (
                chain.chain_id,
                json.dumps(chain.chain_path, ensure_ascii=False),
                chain.length,
                chain.chain_score,
                chain.namespace,
                chain.first_seen,
                chain.last_updated,
                1 if chain.is_active else 0,
            ),
        )
