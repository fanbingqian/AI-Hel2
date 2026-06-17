"""HEIMDALL PageRank — 2-hop localized PageRank with forgetting curve.

V3.0 Section 3.2: localized PageRank propagation with base_type_weight
priors, exponential forgetting, and error self-monitoring. Computation
is bounded to 2-hop neighborhoods to stay within ≤2s latency budget.

Key formulas:
  base_score = type_weight × [α×Freq + β×Σ(intensity_i × weight_i)]
  effective = base_score × e^(-λ×Δt) × (1 + access_count^0.5)
  delta_N1 = effective × decay / degree(E)
  delta_N2 = delta_N1 × decay / degree(N1) × 0.5
"""

from __future__ import annotations

import logging
import math
import time
from collections import defaultdict
from typing import Optional

logger = logging.getLogger(__name__)

# ── Base type weights (V3.0 spec, empirical prior from 100-user study) ──
# Prior source: internal team analysis of relationship networks across
# 100 test users. Subject to A/B tuning and periodic counterfactual reports.
BASE_TYPE_WEIGHT = {
    # Entity roles in social context
    "spouse": 2.0,
    "parent": 2.0,
    "child": 2.0,
    # Core identity labels
    "core_identity": 1.5,
    # Default
    "default": 1.0,
    # Transient events
    "transient_event": 0.5,
}

# Entity-type to base weight mapping
ENTITY_TYPE_WEIGHT = {
    "person": 1.2,       # People are central
    "project": 1.1,      # Projects matter
    "skill": 1.1,        # Skills are important
    "tool": 1.0,         # Tools are references
    "organization": 1.0,
    "concept": 0.9,
    "event": 0.7,
    "location": 0.8,
    "media": 0.7,
}

# ── Algorithm parameters ──
ALPHA = 0.6          # weight of frequency term
BETA = 0.4           # weight of connection intensity term
LAMBDA = 0.01        # forgetting rate (per day)
DECAY_FACTOR = 0.85  # 1-hop propagation decay
HOP2_ATTENUATION = 0.5  # 2-hop attenuation multiplier
ERROR_THRESHOLD = 0.2   # trigger cloud re-sync when error > 0.2

# Fixed retrieval weights for phase 1 (before FTRL online learning)
PHASE1_WEIGHTS = {
    "vector_sim": 0.5,
    "pagerank": 0.2,
    "time_decay": 0.2,
    "freshness": 0.1,
    "bridge_boost": 0.0,    # enabled only when bridges exist
    "serendipity": 0.15,    # cold-start value (was 0.2 in V2.0)
}


class PageRankComputer:
    """Lazy, local PageRank computation with 3-hop propagation (V2.3).

    Usage:
        pr = PageRankComputer(store)
        pr.recalc_all()            # batch update all entities
        pr.update_entity(eid)      # update a single entity + 3-hop neighbors
        pr.get_retrieval_score(eid, query_vector_sim)  # retrieval ranking
    """

    INCREMENTAL_LIMIT = 200  # full recompute after this many incremental updates

    def __init__(self, store):
        self._store = store
        self._last_global_sync: dict[str, float] = {}  # eid → synced_pagerank
        self._incremental_count = 0
        self._global_pagerank: dict[str, float] = {}
        self._total_nodes = 0

    # ------------------------------------------------------------------
    # Public API
    # ------------------------------------------------------------------

    def recalc_all(self) -> dict:
        """Batch-recalculate PageRank for all active entities.

        Returns {entity_id: new_pagerank} for all updated entities.
        """
        entities = self._fetch_active_entities()
        if not entities:
            return {}

        self._total_nodes = len(entities)
        now = time.time()
        results: dict[str, float] = {}

        for entity in entities:
            eid = entity["entity_id"]
            score = self._calc_base_score(entity, now)
            results[eid] = score

        # 3-hop propagation pass (in-memory)
        propagated = self._propagate_3hop(results, entities, now)

        # Persist
        self._persist_scores(propagated, now)
        self._global_pagerank.update(propagated)
        self._incremental_count = 0

        logger.info(
            "PageRank recalc: %d entities, range [%.4f, %.4f]",
            len(propagated),
            min(propagated.values()) if propagated else 0,
            max(propagated.values()) if propagated else 0,
        )
        return propagated

    def top_n(self, entities: list[dict], n: int = 3) -> list[dict]:
        """Return top-N entities by current PageRank score."""
        # Sort by pagerank descending; entities without pagerank default to 1.0
        scored = []
        for e in entities:
            eid = e.get("entity_id", "")
            pr = self._global_pagerank.get(eid, e.get("pagerank", 1.0))
            scored.append((pr, e))
        scored.sort(key=lambda x: x[0], reverse=True)
        return [e for _, e in scored[:n]]

    def update_entity(self, entity_id: str) -> dict:
        """Update PageRank for a single entity + its 3-hop neighbors (V2.3).

        Called when a new entity is added or an existing entity is updated.
        Uses incremental counter to trigger periodic full recompute.
        """
        entity = self._store.get_entity(entity_id)
        if not entity:
            return {}

        self._incremental_count += 1
        if self._incremental_count >= self.INCREMENTAL_LIMIT:
            self._incremental_count = 0
            logger.info("Incremental limit reached (%d), triggering full recompute",
                        self.INCREMENTAL_LIMIT)
            return self.recalc_all()

        now = time.time()
        affected: dict[str, float] = {}

        # Base score for the entity
        base = self._calc_base_score(entity, now)
        affected[entity_id] = base

        # Get the entity's neighborhood (BFS 3-hop)
        neighbors_1hop = self._get_neighbors(entity_id)
        degree_e = max(len(neighbors_1hop), 1)

        # Build full subgraph (3-hop)
        subgraph_ids = {entity_id}
        subgraph_ids.update(neighbors_1hop)
        neighbors_2hop: set[str] = set()
        for n1 in neighbors_1hop:
            n2s = self._get_neighbors(n1)
            subgraph_ids.update(n2s)
            neighbors_2hop.update(n2s)
        neighbors_3hop: set[str] = set()
        for n2 in neighbors_2hop:
            n3s = self._get_neighbors(n2)
            subgraph_ids.update(n3s)
            neighbors_3hop.update(n3s)

        # Boundary nodes: 3-hop nodes that have edges outside the subgraph
        boundary_nodes: set[str] = set()
        for n3 in neighbors_3hop:
            n4s = self._get_neighbors(n3)
            if any(nb not in subgraph_ids for nb in n4s):
                boundary_nodes.add(n3)

        # External contribution from boundary nodes
        external_contribution: dict[str, float] = {}
        for bn in boundary_nodes:
            external_contribution[bn] = self._global_pagerank.get(
                bn, 0.15 / max(self._total_nodes, 1)
            )

        # Compute base scores for all subgraph nodes
        for nid in subgraph_ids:
            if nid not in affected:
                n_entity = self._store.get_entity(nid)
                if n_entity:
                    affected[nid] = self._calc_base_score(n_entity, now)
                else:
                    affected[nid] = 0.15 / max(self._total_nodes, 1)

        # 1-hop propagation
        for n1 in neighbors_1hop:
            delta_n1 = base * DECAY_FACTOR / degree_e
            affected[n1] = affected.get(n1, 0) + delta_n1

            # 2-hop propagation
            n1_neighbors = self._get_neighbors(n1)
            degree_n1 = max(len(n1_neighbors), 1)
            for n2 in n1_neighbors:
                if n2 == entity_id or n2 in neighbors_1hop:
                    continue
                delta_n2 = delta_n1 * DECAY_FACTOR / degree_n1 * HOP2_ATTENUATION
                affected[n2] = affected.get(n2, 0.0) + delta_n2

                # 3-hop propagation
                n2_neighbors = self._get_neighbors(n2)
                degree_n2 = max(len(n2_neighbors), 1)
                for n3 in n2_neighbors:
                    if n3 in subgraph_ids and n3 != n2 and n3 != n1 and n3 != entity_id:
                        if n3 not in neighbors_1hop and n3 not in neighbors_2hop:
                            continue
                        delta_n3 = delta_n2 * DECAY_FACTOR / degree_n2 * HOP2_ATTENUATION * 0.5
                        affected[n3] = affected.get(n3, 0.0) + delta_n3

        # Apply boundary external contribution (damping from outside subgraph)
        for bn in boundary_nodes:
            if bn in affected:
                affected[bn] = 0.85 * affected[bn] + 0.15 * external_contribution.get(bn, 0)

        # Persist
        self._persist_scores(affected, now)
        self._global_pagerank.update(affected)

        # Check error against last sync
        for eid, score in affected.items():
            self._check_error(eid, score)

        return affected

    def get_retrieval_score(
        self,
        entity_id: str,
        vector_sim: float = 0.5,
        query_freshness: float = 0.5,
    ) -> float:
        """Compute retrieval ranking score for an entity.

        Phase 1: fixed weights (V3.0 spec).
        Phase 2: FTRL online-learned weights.

        Returns a score in [0, 1].
        """
        entity = self._store.get_entity(entity_id)
        if not entity:
            return 0.0

        pr = entity.get("pagerank", 1.0)
        bridge_score = entity.get("bridge_score", 0.0)
        last_seen = entity.get("last_seen_at", time.time())
        now = time.time()

        # Time decay: exponential, halving every 30 days
        days = (now - last_seen) / 86400.0
        time_decay = math.exp(-0.0231 * days)  # ln(2)/30 ≈ 0.0231

        # Bridge boost: enable only if entity is a bridge
        bridge_boost = bridge_score if entity.get("is_bridge") else 0.0

        # Normalize pagerank to [0,1] range (assume max ~10)
        pr_norm = min(pr / 10.0, 1.0)

        features = {
            "vector_sim": vector_sim,
            "pagerank": pr_norm,
            "time_decay": time_decay,
            "freshness": query_freshness,
            "bridge_boost": bridge_boost,
            "serendipity": self._serendipity_score(entity),
        }

        score = sum(
            PHASE1_WEIGHTS[k] * features.get(k, 0)
            for k in PHASE1_WEIGHTS
        )
        return max(0.0, min(score, 1.0))

    def mark_synced(self, synced_scores: dict[str, float]) -> None:
        """Update the last-known-global-sync values for error tracking."""
        self._last_global_sync.update(synced_scores)
        # Reset error for synced entities
        for eid, score in synced_scores.items():
            try:
                self._store._conn.execute(
                    "UPDATE heimdall_entities SET pagerank_error = 0.0 WHERE entity_id = ?",
                    (eid,),
                )
            except Exception:
                pass

    # ------------------------------------------------------------------
    # Base score calculation
    # ------------------------------------------------------------------

    def _calc_base_score(self, entity: dict, now: float) -> float:
        """Calculate the base PageRank score for an entity.

        base_score = type_weight × [α×Freq + β×Σ(intensity_i × weight_i)]
        effective = base_score × e^(-λ×Δt) × (1 + access_count^0.5)
        """
        etype = entity.get("entity_type", "concept")
        type_weight = ENTITY_TYPE_WEIGHT.get(etype, 1.0)

        # Frequency component
        freq = entity.get("occurrence_count", 1)
        freq_term = ALPHA * math.log1p(freq)  # log1p to dampen

        # Connection intensity component
        connections = self._get_connections(entity.get("entity_id", ""))
        intensity_sum = sum(
            c.get("intensity", 0.5) * c.get("valence_abs", 0.5)
            for c in connections
        )
        conn_term = BETA * min(intensity_sum, 10.0)

        base = type_weight * (freq_term + conn_term)

        # Forgetting curve
        last_seen = entity.get("last_seen_at", now)
        days_since = max(0.0, (now - last_seen) / 86400.0)
        forgetting = math.exp(-LAMBDA * days_since)

        # Access count bonus
        access_count = entity.get("occurrence_count", 0)
        access_bonus = 1.0 + math.sqrt(access_count)

        effective = base * forgetting * access_bonus

        return max(0.01, effective)

    def _serendipity_score(self, entity: dict) -> float:
        """Compute serendipity as cross-community novelty.

        Serendipity is high when an entity bridges communities and has
        connections to diverse entity types.
        """
        eid = entity.get("entity_id", "")
        bridge_score = entity.get("bridge_score", 0.0)
        if bridge_score <= 0:
            return 0.0

        # Diversity bonus: count distinct entity types among neighbors
        neighbors = self._get_neighbors(eid)
        neighbor_types = set()
        for nb in neighbors:
            nb_entity = self._store.get_entity(nb)
            if nb_entity:
                neighbor_types.add(nb_entity.get("entity_type", ""))

        type_diversity = len(neighbor_types) / 9.0  # 9 possible types
        return min(bridge_score * (0.3 + 0.7 * type_diversity), 1.0)

    # ------------------------------------------------------------------
    # 2-hop propagation
    # ------------------------------------------------------------------

    def _propagate_3hop(
        self,
        base_scores: dict[str, float],
        entities: list[dict],
        now: float,
    ) -> dict[str, float]:
        """Propagate base scores through 3-hop neighborhoods (V2.3).

        For each entity E:
          For each 1-hop neighbor N1:
            delta_N1 = score(E) × decay / degree(E)
          For each 2-hop neighbor N2:
            delta_N2 = delta_N1 × decay / degree(N1) × 0.5
          For each 3-hop neighbor N3:
            delta_N3 = delta_N2 × decay / degree(N2) × 0.25
        """
        propagated = dict(base_scores)
        entity_set = {e["entity_id"] for e in entities}

        for entity in entities:
            eid = entity["entity_id"]
            if eid not in base_scores:
                continue
            base = base_scores[eid]

            neighbors_1hop = self._get_neighbors(eid)
            degree_e = max(len(neighbors_1hop), 1)

            for n1 in neighbors_1hop:
                if n1 not in entity_set:
                    continue
                delta_n1 = base * DECAY_FACTOR / degree_e
                propagated[n1] = propagated.get(n1, 0) + delta_n1

                neighbors_2hop = self._get_neighbors(n1)
                degree_n1 = max(len(neighbors_2hop), 1)
                for n2 in neighbors_2hop:
                    if n2 == eid or n2 in neighbors_1hop:
                        continue
                    if n2 not in entity_set:
                        continue
                    delta_n2 = delta_n1 * DECAY_FACTOR / degree_n1 * HOP2_ATTENUATION
                    propagated[n2] = propagated.get(n2, 0) + delta_n2

                    # 3-hop propagation
                    neighbors_3hop = self._get_neighbors(n2)
                    degree_n2 = max(len(neighbors_3hop), 1)
                    for n3 in neighbors_3hop:
                        if n3 == eid or n3 in neighbors_1hop or n3 in neighbors_2hop:
                            continue
                        if n3 not in entity_set:
                            continue
                        delta_n3 = delta_n2 * DECAY_FACTOR / degree_n2 * HOP2_ATTENUATION * 0.5
                        propagated[n3] = propagated.get(n3, 0) + delta_n3

        return propagated

    # ------------------------------------------------------------------
    # Error monitoring
    # ------------------------------------------------------------------

    def _check_error(self, entity_id: str, local_score: float) -> float:
        """Check and record pagerank error against last global sync.

        error = |local_pagerank - last_global_sync|
        If error > 0.2, flag for cloud re-sync.
        """
        synced = self._last_global_sync.get(entity_id)
        if synced is None:
            return 0.0

        error = abs(local_score - synced)
        try:
            self._store._conn.execute(
                "UPDATE heimdall_entities SET pagerank_error = ? WHERE entity_id = ?",
                (round(error, 6), entity_id),
            )
        except Exception:
            pass

        if error > ERROR_THRESHOLD:
            logger.info(
                "PageRank error %.4f > %.2f for %s — cloud re-sync recommended",
                error, ERROR_THRESHOLD, entity_id,
            )
        return error

    # ------------------------------------------------------------------
    # Persistence
    # ------------------------------------------------------------------

    def _persist_scores(self, scores: dict[str, float], now: float) -> None:
        """Write pagerank scores to the database."""
        if not self._store._conn:
            return
        for eid, score in scores.items():
            self._store._conn.execute(
                "UPDATE heimdall_entities SET pagerank = ?, last_seen_at = MAX(last_seen_at, ?) "
                "WHERE entity_id = ?",
                (round(score, 6), now, eid),
            )

    # ------------------------------------------------------------------
    # Data fetching
    # ------------------------------------------------------------------

    def _fetch_active_entities(self) -> list[dict]:
        if not self._store._conn:
            return []
        rows = self._store._conn.execute(
            "SELECT * FROM heimdall_entities WHERE status = 'active'"
        ).fetchall()
        return [dict(r) for r in rows]

    def _get_neighbors(self, entity_id: str) -> set[str]:
        """Get all neighboring entity IDs via social graph edges."""
        neighbors = set()
        if not self._store._conn:
            return neighbors
        try:
            rows = self._store._conn.execute(
                "SELECT source_entity_id, target_entity_id FROM heimdall_social_graph "
                "WHERE source_entity_id = ? OR target_entity_id = ?",
                (entity_id, entity_id),
            ).fetchall()
            for row in rows:
                src = row["source_entity_id"]
                tgt = row["target_entity_id"]
                nb = src if tgt == entity_id else tgt
                if nb != entity_id:
                    neighbors.add(nb)
        except Exception:
            pass
        return neighbors

    def _get_connections(self, entity_id: str) -> list[dict]:
        """Get all social graph connections for an entity."""
        if not self._store._conn:
            return []
        try:
            rows = self._store._conn.execute(
                "SELECT intensity, valence FROM heimdall_social_graph "
                "WHERE source_entity_id = ? OR target_entity_id = ?",
                (entity_id, entity_id),
            ).fetchall()
            result = []
            for row in rows:
                result.append({
                    "intensity": row["intensity"] or 0.5,
                    "valence_abs": abs(row["valence"] or 0.0),
                })
            return result
        except Exception:
            return []

    def get_entities_needing_sync(self) -> list[dict]:
        """Return entities with pagerank_error > ERROR_THRESHOLD."""
        if not self._store._conn:
            return []
        rows = self._store._conn.execute(
            "SELECT entity_id, display_name, pagerank, pagerank_error "
            "FROM heimdall_entities WHERE pagerank_error > ? AND status = 'active' "
            "ORDER BY pagerank_error DESC",
            (ERROR_THRESHOLD,),
        ).fetchall()
        return [dict(r) for r in rows]
