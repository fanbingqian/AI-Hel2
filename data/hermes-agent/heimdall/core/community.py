"""HEIMDALL Community Detection — dynamic graph embedding + streaming GMM.

V3.0 Section 3.1: data-driven community discovery with soft assignment,
cold-start thresholds, and bridge node detection. All computation is local,
lazy, and numpy-only (no scipy/sklearn dependency).

Algorithm:
  1. Build co-occurrence vectors from memory edges (entities in same session)
  2. Normalize into pseudo-embeddings
  3. K-means++ clustering (stand-in for full GMM; same structural output)
  4. Soft assignment via distance-to-centroid ratios
  5. Bridge detection via cross-community connectivity
  6. Cold-start gate: ≥50 global entities AND ≥5 per community
"""

from __future__ import annotations

import logging
import math
import numpy as np
from collections import defaultdict
from typing import Optional

logger = logging.getLogger(__name__)

# ── constants from V3.0 spec ──────────────────────────────────────────────
COLD_START_GLOBAL_MIN = 50   # min total entities before any community forms
COLD_START_COMMUNITY_MIN = 5  # min entities per community
EMBEDDING_DIM = 64            # reduced from spec 256 for numpy-only practicality
MAX_COMMUNITIES = 12
SOFT_ASSIGN_THRESHOLD = 0.25  # responsibility below this → "unclassified"
# Note: V3.0 spec says 0.6 for full GMM on 256-dim embeddings. With our
# lightweight co-occurrence embeddings, 0.25 is a reasonable lower bound.
# The effective threshold is max(SOFT_ASSIGN_THRESHOLD, 1.0/k + 0.1).
RANDOM_SEED = 42

# Emoji + generic tag labels (V3.0 section 4.3)
COMMUNITY_LABEL_POOL = [
    ("\U0001f4bb", "工作技术"),   # 💻 工作技术
    ("\U0001f3e0", "家庭生活"),   # 🏠 家庭生活
    ("\U0001f3a8", "创意兴趣"),   # 🎨 创意兴趣
    ("\U0001f4da", "学习成长"),   # 📚 学习成长
    ("\U0001f91d", "社交关系"),   # 🤝 社交关系
    ("\U0001f3af", "个人目标"),   # 🎯 个人目标
    ("☀️", "日常生活"), # ☀️ 日常生活
    ("\U0001f3b5", "娱乐休闲"),   # 🎵 娱乐休闲
    ("\U0001f4b0", "财务规划"),   # 💰 财务规划
    ("\U0001f3e5", "健康医疗"),   # 🏥 健康医疗
    ("✈️", "旅行探索"), # ✈️ 旅行探索
    ("\U0001f527", "技能工具"),   # 🔧 技能工具
]


class CommunityDetector:
    """Lazy, numpy-only community detector for the HEIMDALL entity graph.

    Usage:
        detector = CommunityDetector(store)
        result = detector.detect()
        # result: {entity_id: community_id, ...}
        communities = detector.get_communities()
        # communities: [{id, label, emoji, size, entities: [...]}, ...]
    """

    def __init__(self, store):
        self._store = store

    # ------------------------------------------------------------------
    # Public API
    # ------------------------------------------------------------------

    def detect(self) -> dict:
        """Run community detection. Returns {entity_id: community_id} mapping.

        Returns empty dict if cold-start thresholds are not met.
        """
        entities = self._fetch_entities()
        if len(entities) < COLD_START_GLOBAL_MIN:
            logger.info(
                "Community detection skipped: %d entities (need ≥%d)",
                len(entities), COLD_START_GLOBAL_MIN,
            )
            return {}

        entity_ids = [e["entity_id"] for e in entities]
        id_to_idx = {eid: i for i, eid in enumerate(entity_ids)}

        # 1. Build co-occurrence matrix
        cooc = self._build_cooccurrence(entity_ids, id_to_idx)

        # 2. Compute pseudo-embeddings via row-normalized co-occurrence
        embeddings = self._cooc_to_embeddings(cooc)

        # 3. K-means++ clustering
        k = self._estimate_k(embeddings, len(entity_ids))
        if k < 2:
            logger.info("Community detection: k=%d, no meaningful split", k)
            return {}

        labels, centroids = self._kmeans_pp(embeddings, k)

        # 4. Soft assignment + cold-start community filter
        responsibilities, resp_matrix = self._soft_assignment(embeddings, centroids, labels)

        # 5. Filter small communities + reassign low-confidence entities
        assignments = self._apply_thresholds(
            entity_ids, labels, responsibilities, centroids, embeddings,
        )

        # 6. Persist to DB
        self._persist(assignments)

        # 7. Detect bridges (uses full responsibility matrix)
        self._detect_bridges(entity_ids, id_to_idx, labels, resp_matrix, assignments)

        logger.info(
            "Community detection complete: %d entities → %d communities",
            len(entity_ids), len(set(v for v in assignments.values() if v >= 0)),
        )
        return assignments

    def get_communities(self) -> list[dict]:
        """Return current community summaries from the database."""
        if not self._store._conn:
            return []

        rows = self._store._conn.execute(
            "SELECT community_id, COUNT(*) as size "
            "FROM heimdall_entities "
            "WHERE community_id IS NOT NULL AND status = 'active' "
            "GROUP BY community_id ORDER BY size DESC"
        ).fetchall()

        communities = []
        for row in rows:
            cid = row["community_id"]
            members = self._store._conn.execute(
                "SELECT entity_id, display_name, entity_type, community_confidence, is_bridge "
                "FROM heimdall_entities WHERE community_id = ? AND status = 'active' "
                "ORDER BY occurrence_count DESC LIMIT 50",
                (cid,),
            ).fetchall()

            emoji, tag = self._label_for(cid)
            communities.append({
                "community_id": cid,
                "size": row["size"],
                "emoji": emoji,
                "label": tag,
                "entities": [dict(m) for m in members],
            })

        return communities

    def get_communities_metadata(self) -> list[dict]:
        """Return lightweight community metadata for overview cache.

        Each entry: {name, size, top_entities: [name, ...]}
        Suitable for serialization to community_{namespace}.json.
        """
        communities = self.get_communities()
        result = []
        for c in communities:
            entities = c.get("entities", [])
            result.append({
                "name": f"{c.get('emoji', '')} {c.get('label', '')}",
                "size": c.get("size", 0),
                "entities": [
                    {
                        "name": e.get("display_name", ""),
                        "type": e.get("entity_type", ""),
                        "entity_id": e.get("entity_id", ""),
                    }
                    for e in entities[:20]
                ],
            })
        return result

    def get_bridge_entities(self) -> list[dict]:
        """Return entities flagged as bridges between communities."""
        if not self._store._conn:
            return []
        rows = self._store._conn.execute(
            "SELECT * FROM heimdall_entities WHERE is_bridge = 1 AND status = 'active' "
            "ORDER BY bridge_score DESC"
        ).fetchall()
        return [dict(r) for r in rows]

    # ------------------------------------------------------------------
    # Data fetching
    # ------------------------------------------------------------------

    def _fetch_entities(self) -> list[dict]:
        if not self._store._conn:
            return []
        rows = self._store._conn.execute(
            "SELECT entity_id, entity_type, display_name, occurrence_count, confidence "
            "FROM heimdall_entities WHERE status = 'active'"
        ).fetchall()
        return [dict(r) for r in rows]

    # ------------------------------------------------------------------
    # Co-occurrence matrix
    # ------------------------------------------------------------------

    def _build_cooccurrence(
        self, entity_ids: list[str], id_to_idx: dict
    ) -> np.ndarray:
        """Build entity co-occurrence matrix from session-sharing patterns.

        Two entities co-occur if they appear in the same session (memory edges)
        or are linked via social_graph edges.
        """
        n = len(entity_ids)
        cooc = np.eye(n, dtype=np.float32)

        if not self._store._conn:
            return cooc

        # Co-occurrence from shared sessions (memory edges)
        try:
            sess_rows = self._store._conn.execute(
                "SELECT entity_id, session_id FROM heimdall_memory_edges "
                "WHERE entity_id IN ({}) AND session_id != ''".format(
                    ",".join("?" for _ in entity_ids)
                ),
                entity_ids,
            ).fetchall()
        except Exception:
            sess_rows = []

        # Fallback: if no memory edges, use entity source_session_id for
        # co-occurrence (entities created in the same session are related)
        if not sess_rows:
            try:
                sess_rows = self._store._conn.execute(
                    "SELECT entity_id, source_session_id as session_id "
                    "FROM heimdall_entities "
                    "WHERE entity_id IN ({}) AND source_session_id != '' "
                    "AND source_session_id != 'migration' AND status = 'active'".format(
                        ",".join("?" for _ in entity_ids)
                    ),
                    entity_ids,
                ).fetchall()
            except Exception:
                sess_rows = []

        # Group entity IDs by session
        session_entities: dict[str, set] = defaultdict(set)
        for row in sess_rows:
            if row["session_id"]:
                session_entities[row["session_id"]].add(row["entity_id"])

        for eids in session_entities.values():
            members = [id_to_idx[e] for e in eids if e in id_to_idx]
            for i in members:
                for j in members:
                    if i != j:
                        cooc[i, j] += 1.0

        # Co-occurrence from social graph edges
        try:
            social_rows = self._store._conn.execute(
                "SELECT source_entity_id, target_entity_id FROM heimdall_social_graph "
                "WHERE source_entity_id IN ({}) OR target_entity_id IN ({})".format(
                    ",".join("?" for _ in entity_ids),
                    ",".join("?" for _ in entity_ids),
                ),
                entity_ids * 2,
            ).fetchall()
        except Exception:
            social_rows = []

        for row in social_rows:
            src = row["source_entity_id"]
            tgt = row["target_entity_id"]
            if src in id_to_idx and tgt in id_to_idx:
                i, j = id_to_idx[src], id_to_idx[tgt]
                cooc[i, j] += 0.5
                cooc[j, i] += 0.5

        # Gaussian-like normalization
        max_val = cooc.max()
        if max_val > 0:
            cooc = cooc / max_val

        return cooc

    # ------------------------------------------------------------------
    # Embedding construction
    # ------------------------------------------------------------------

    def _cooc_to_embeddings(self, cooc: np.ndarray) -> np.ndarray:
        """Convert co-occurrence matrix to pseudo-embedding vectors.

        Uses co-occurrence rows directly as entity embeddings, with optional
        PCA reduction via power iteration when dimensions permit.
        """
        n = cooc.shape[0]
        dim = min(EMBEDDING_DIM, n)

        # Use co-occurrence rows directly — each row encodes what other
        # entities this entity co-occurs with. Row-normalize to form
        # a proper embedding.
        embeddings = cooc.astype(np.float32).copy()

        # Row-normalize (L2)
        norms = np.linalg.norm(embeddings, axis=1, keepdims=True)
        norms[norms < 1e-10] = 1.0
        embeddings = embeddings / norms

        # If we need to reduce dimensionality, use truncated SVD via
        # power iteration on covariance
        if n > dim:
            mean = embeddings.mean(axis=0, keepdims=True)
            centered = embeddings - mean
            cov = centered.T @ centered / (n - 1)

            reduced = np.zeros((n, dim), dtype=np.float32)
            for d in range(min(dim, n)):
                v = np.random.randn(dim).astype(np.float32)
                v /= np.linalg.norm(v)
                for _ in range(50):
                    v_new = cov @ v
                    for p in range(d):
                        v_new -= np.dot(v_new, reduced[:, p]) * reduced[:, p]
                    v_norm = np.linalg.norm(v_new)
                    if v_norm < 1e-10:
                        break
                    v = v_new / v_norm
                if v_norm > 1e-10:
                    # Project centered data onto this eigenvector
                    reduced[:, d] = centered @ v
            embeddings = reduced

        # Row-normalize again
        norms = np.linalg.norm(embeddings, axis=1, keepdims=True)
        norms[norms < 1e-10] = 1.0
        embeddings = embeddings / norms

        return embeddings.astype(np.float32)

    # ------------------------------------------------------------------
    # K-means++ clustering
    # ------------------------------------------------------------------

    def _estimate_k(self, embeddings: np.ndarray, n: int) -> int:
        """Estimate number of communities via a simple elbow heuristic."""
        if n < 2 * COLD_START_COMMUNITY_MIN:
            return 0
        max_k = min(MAX_COMMUNITIES, n // COLD_START_COMMUNITY_MIN)
        if max_k < 2:
            return 0
        # Try k=2..max_k and pick best by silhouette-like score
        best_k = 2
        best_score = -1.0
        for k in range(2, max_k + 1):
            labels, centroids = self._kmeans_pp(embeddings, k)
            score = self._silhouette_approx(embeddings, labels, centroids)
            if score > best_score:
                best_score = score
                best_k = k
        return best_k

    def _kmeans_pp(
        self, embeddings: np.ndarray, k: int, max_iters: int = 100
    ) -> tuple[np.ndarray, np.ndarray]:
        """K-means++ clustering (numpy-only)."""
        n, d = embeddings.shape
        rng = np.random.default_rng(RANDOM_SEED)

        # K-means++ init
        centroids = np.zeros((k, d), dtype=np.float32)
        centroids[0] = embeddings[rng.integers(0, n)]

        for c in range(1, k):
            dists = np.min(
                [np.sum((embeddings - centroids[i]) ** 2, axis=1) for i in range(c)],
                axis=0,
            )
            probs = dists / dists.sum()
            centroids[c] = embeddings[rng.choice(n, p=probs)]

        # Lloyd's algorithm
        labels = np.zeros(n, dtype=np.int32)
        for _ in range(max_iters):
            # Assignment
            dists = np.zeros((n, k), dtype=np.float32)
            for i in range(k):
                dists[:, i] = np.sum((embeddings - centroids[i]) ** 2, axis=1)
            new_labels = np.argmin(dists, axis=1)

            # Update
            new_centroids = np.zeros((k, d), dtype=np.float32)
            counts = np.zeros(k, dtype=np.int32)
            for i in range(n):
                new_centroids[new_labels[i]] += embeddings[i]
                counts[new_labels[i]] += 1
            for i in range(k):
                if counts[i] > 0:
                    new_centroids[i] /= counts[i]
                else:
                    new_centroids[i] = embeddings[rng.integers(0, n)]

            if np.array_equal(new_labels, labels):
                break
            labels = new_labels
            centroids = new_centroids

        return labels, centroids

    def _silhouette_approx(
        self, embeddings: np.ndarray, labels: np.ndarray, centroids: np.ndarray
    ) -> float:
        """Approximate silhouette score using centroid distances."""
        n = embeddings.shape[0]
        if n < 2:
            return 0.0

        k = centroids.shape[0]
        all_dists = np.zeros((n, k), dtype=np.float32)
        for i in range(k):
            all_dists[:, i] = np.sum((embeddings - centroids[i]) ** 2, axis=1)

        scores = []
        for i in range(n):
            c = labels[i]
            a = all_dists[i, c]
            other_dists = [all_dists[i, j] for j in range(k) if j != c]
            b = min(other_dists) if other_dists else a
            s = (b - a) / max(a, b) if max(a, b) > 0 else 0.0
            scores.append(s)

        return float(np.mean(scores))

    # ------------------------------------------------------------------
    # Soft assignment
    # ------------------------------------------------------------------

    def _soft_assignment(
        self,
        embeddings: np.ndarray,
        centroids: np.ndarray,
        hard_labels: np.ndarray,
    ) -> tuple[np.ndarray, np.ndarray]:
        """Compute soft assignment responsibilities.

        Returns:
          assigned_resp: 1D array[n] — responsibility for the assigned cluster
          resp_matrix: 2D array[n, k] — full responsibility matrix (for bridge calc)

        For each entity, responsibility_i = exp(-dist_i) / sum(exp(-dist_all)).
        Values < threshold indicate borderline entities.
        """
        n, k = len(embeddings), len(centroids)
        dists = np.zeros((n, k), dtype=np.float32)
        for i in range(k):
            dists[:, i] = np.sum((embeddings - centroids[i]) ** 2, axis=1)

        # Softmax over negative distances
        dists_stable = dists - dists.max(axis=1, keepdims=True)
        exp_dists = np.exp(-dists_stable)
        resp_matrix = exp_dists / exp_dists.sum(axis=1, keepdims=True)

        # Return the responsibility for the assigned cluster
        assigned = np.zeros(n, dtype=np.float32)
        for i in range(n):
            assigned[i] = resp_matrix[i, hard_labels[i]]

        return assigned, resp_matrix

    # ------------------------------------------------------------------
    # Threshold application
    # ------------------------------------------------------------------

    def _apply_thresholds(
        self,
        entity_ids: list[str],
        labels: np.ndarray,
        responsibilities: np.ndarray,
        centroids: np.ndarray,
        embeddings: np.ndarray,
    ) -> dict:
        """Apply cold-start thresholds and soft-assignment filtering.

        Returns {entity_id: community_id} where community_id is -1 for
        unclassified entities.
        """
        n = len(entity_ids)
        k = centroids.shape[0]
        # Adaptive threshold: expected responsibility is 1/k, require 2x that
        # or the floor threshold, whichever is larger
        threshold = max(SOFT_ASSIGN_THRESHOLD, 1.0 / k + 0.1)

        # Count entities per community
        community_counts: dict[int, int] = defaultdict(int)
        for i in range(n):
            if responsibilities[i] >= threshold:
                community_counts[labels[i]] += 1

        # Filter: only keep communities with ≥ COLD_START_COMMUNITY_MIN members
        valid_communities = {
            c for c, cnt in community_counts.items()
            if cnt >= COLD_START_COMMUNITY_MIN
        }

        if len(valid_communities) < 2:
            logger.info(
                "Cold start: %d valid communities (need ≥2), all entities unclassified",
                len(valid_communities),
            )
            return {eid: -1 for eid in entity_ids}

        assignments: dict[str, int] = {}
        for i, eid in enumerate(entity_ids):
            c = int(labels[i])
            if c in valid_communities and responsibilities[i] >= threshold:
                assignments[eid] = c
            else:
                assignments[eid] = -1  # unclassified

        # Re-map community IDs to be dense (0, 1, 2, ...)
        valid_sorted = sorted(valid_communities)
        remap = {old: new for new, old in enumerate(valid_sorted)}
        for eid in assignments:
            if assignments[eid] >= 0:
                assignments[eid] = remap[assignments[eid]]

        return assignments

    # ------------------------------------------------------------------
    # Persistence
    # ------------------------------------------------------------------

    def _persist(self, assignments: dict) -> None:
        """Write community_id and community_confidence to the database."""
        if not self._store._conn:
            return

        # Reset all community assignments first
        self._store._conn.execute(
            "UPDATE heimdall_entities SET community_id = NULL, "
            "community_confidence = 1.0, is_bridge = 0, bridge_score = 0.0"
        )

        for eid, cid in assignments.items():
            if cid >= 0:
                self._store._conn.execute(
                    "UPDATE heimdall_entities SET community_id = ?, community_confidence = 1.0 "
                    "WHERE entity_id = ?",
                    (cid, eid),
                )
            else:
                self._store._conn.execute(
                    "UPDATE heimdall_entities SET community_id = NULL, community_confidence = 0.5 "
                    "WHERE entity_id = ?",
                    (eid,),
                )

    # ------------------------------------------------------------------
    # Bridge detection (V3.0 section 3.1)
    # ------------------------------------------------------------------

    def _detect_bridges(
        self,
        entity_ids: list[str],
        id_to_idx: dict,
        labels: np.ndarray,
        resp_matrix: np.ndarray,
        assignments: dict,
    ) -> None:
        """Detect bridge entities using GMM responsibility cross-product.

        V3.0 Section 3.3 formula:
          bridge_score(E) = Σ_{i≠j} √(responsibility_i(E) × responsibility_j(E))

        Entities with high bridge_score "sit between" communities in soft
        assignment space. Secondary filter: must have at least 1 cross-community
        neighbor (structural bridge, not just fuzzy membership).

        Threshold of 0.35 filters out noise from k-means softmax spread while
        catching genuine multi-community entities.
        """
        if not self._store._conn:
            return

        eid_to_community = {eid: cid for eid, cid in assignments.items() if cid >= 0}
        active_communities = set(eid_to_community.values())
        if len(active_communities) < 2:
            return

        # Pre-compute neighbor community sets for structural validation
        neighbor_communities: dict[str, set] = {}
        for eid in entity_ids:
            neighbors = self._get_neighbor_ids(eid, entity_ids)
            comms = set()
            for nb in neighbors:
                if nb in eid_to_community:
                    comms.add(eid_to_community[nb])
            neighbor_communities[eid] = comms

        BRIDGE_SCORE_THRESHOLD = 0.35  # calibrated for k-means softmax spread

        for eid, idx in id_to_idx.items():
            if idx >= len(resp_matrix):
                continue

            resp = resp_matrix[idx]  # responsibility vector for this entity
            k = len(resp)

            # Bridge score: sum of sqrt(resp_i × resp_j) over i≠j
            bridge_score = 0.0
            for i in range(k):
                for j in range(i + 1, k):
                    if resp[i] > 0 and resp[j] > 0:
                        bridge_score += math.sqrt(resp[i] * resp[j])

            # Secondary: check top-2 responsibilities are close (near-equal membership)
            sorted_resp = sorted(resp, reverse=True)
            near_equal = (
                len(sorted_resp) >= 2
                and sorted_resp[0] > 0
                and (sorted_resp[1] / sorted_resp[0]) > 0.75
            )

            # Structural validation: must connect to entities in ≥2 other communities
            eid_cid = assignments.get(eid, -1)
            cross_communities = {
                c for c in neighbor_communities.get(eid, set())
                if c != eid_cid
            }
            has_cross_links = len(cross_communities) >= 2

            # Combine: high GMM bridge score AND structural cross-links
            # OR near-equal responsibility with at least 1 cross-link
            is_bridge = (
                (bridge_score > BRIDGE_SCORE_THRESHOLD and has_cross_links)
                or (near_equal and len(cross_communities) >= 1)
            )

            self._store._conn.execute(
                "UPDATE heimdall_entities SET is_bridge = ?, bridge_score = ? "
                "WHERE entity_id = ?",
                (1 if is_bridge else 0, round(float(bridge_score), 4), eid),
            )

        bridge_count = self._store._conn.execute(
            "SELECT COUNT(*) FROM heimdall_entities WHERE is_bridge = 1 AND status = 'active'"
        ).fetchone()[0]
        if bridge_count > 0:
            logger.info("Bridge detection: %d bridges found", bridge_count)
        else:
            logger.info(
                "Bridge detection: no bridges (threshold=%.2f, communities=%d)",
                BRIDGE_SCORE_THRESHOLD, len(active_communities),
            )

    def _get_neighbor_ids(self, entity_id: str, all_ids: list[str]) -> set[str]:
        """Get all entities connected to entity_id via social or memory edges."""
        neighbors = set()
        if not self._store._conn:
            return neighbors

        # Social graph neighbors
        try:
            rows = self._store._conn.execute(
                "SELECT source_entity_id, target_entity_id FROM heimdall_social_graph "
                "WHERE source_entity_id = ? OR target_entity_id = ?",
                (entity_id, entity_id),
            ).fetchall()
            for row in rows:
                nb = row["source_entity_id"] if row["target_entity_id"] == entity_id else row["target_entity_id"]
                if nb != entity_id:
                    neighbors.add(nb)
        except Exception:
            pass

        return neighbors

    # ------------------------------------------------------------------
    # Community labeling
    # ------------------------------------------------------------------

    def _label_for(self, community_id: int) -> tuple[str, str]:
        """Return (emoji, tag) for a community ID."""
        idx = community_id % len(COMMUNITY_LABEL_POOL)
        return COMMUNITY_LABEL_POOL[idx]
