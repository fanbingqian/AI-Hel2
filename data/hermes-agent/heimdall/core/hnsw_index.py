"""HNSW approximate nearest neighbor index for vector search (V2.3).

Provides O(log n) vector similarity search via hnswlib, with numpy batch
fallback when hnswlib is not installed. Replaces the O(n) linear scan in
VectorRetriever when entity count exceeds ~1,000.
"""

from __future__ import annotations

import logging
import os
from pathlib import Path
from typing import Optional

import numpy as np

logger = logging.getLogger(__name__)

HEIMDALL_HOME = Path.home() / ".heimdall"
INDEX_PATH = HEIMDALL_HOME / "cache" / "hnsw_index.bin"
MAPPING_PATH = HEIMDALL_HOME / "cache" / "hnsw_mapping.npz"

try:
    import hnswlib

    _HAS_HNSWLIB = True
except ImportError:
    _HAS_HNSWLIB = False


class HNSWIndex:
    """HNSW approximate vector index with numpy fallback.

    Uses hnswlib when available for O(log n) search.
    Falls back to batch numpy cosine similarity (faster than per-row Python loop
    but still O(n)) when hnswlib is not installed.

    Lazy init: index is created on first add() to avoid overhead when
    no embeddings exist.
    """

    def __init__(self, dim: int = 1024, max_elements: int = 100000):
        self._dim = dim
        self._max_elements = max_elements
        self._index = None
        self._id_map: dict[int, str] = {}  # internal_id → entity_id
        self._reverse_map: dict[str, int] = {}  # entity_id → internal_id
        self._vectors: dict[str, np.ndarray] = {}  # entity_id → float16 vector
        self._next_id = 0
        self._dirty = False  # track unsaved changes
        self._enabled = _HAS_HNSWLIB  # hnswlib mode
        mode = "hnswlib" if _HAS_HNSWLIB else "numpy-fallback"
        logger.info("HNSWIndex init (dim=%d, max=%d, mode=%s)", dim, max_elements, mode)

    # ------------------------------------------------------------------
    # Public API
    # ------------------------------------------------------------------

    @property
    def enabled(self) -> bool:
        return self._enabled

    @property
    def count(self) -> int:
        return len(self._vectors)

    def add(self, entity_id: str, vector: bytes) -> bool:
        """Add or update a vector in the index."""
        if not vector or len(vector) < 2:
            return False

        try:
            vec = np.frombuffer(vector, dtype=np.float16).astype(np.float32)
        except Exception:
            return False

        if vec.shape[0] != self._dim:
            logger.warning("HNSW add: dim mismatch (%d vs %d)", vec.shape[0], self._dim)
            return False

        # Normalize for cosine similarity
        norm = np.linalg.norm(vec)
        if norm > 0:
            vec = vec / norm

        # Remove old entry if exists
        if entity_id in self._reverse_map:
            self.remove(entity_id)

        self._vectors[entity_id] = vec.astype(np.float16)
        self._dirty = True

        if self._enabled and self._index is not None:
            internal_id = self._next_id
            self._next_id += 1
            self._id_map[internal_id] = entity_id
            self._reverse_map[entity_id] = internal_id
            try:
                self._index.add_items(vec.reshape(1, -1), [internal_id])
            except Exception:
                # Mark index for rebuild on next search
                self._index = None

        return True

    def remove(self, entity_id: str):
        """Remove a vector from the index."""
        self._vectors.pop(entity_id, None)
        if entity_id in self._reverse_map:
            internal_id = self._reverse_map.pop(entity_id)
            self._id_map.pop(internal_id, None)
        # HNSW doesn't support efficient deletion — mark dirty for rebuild
        self._dirty = True
        if self._index is not None:
            self._index = None  # will rebuild on next search

    def search(self, query_vector: bytes, k: int = 10) -> list[tuple[str, float]]:
        """Search for k nearest neighbors by cosine similarity.

        Returns list of (entity_id, similarity_score) sorted by similarity desc.
        """
        if not self._vectors:
            return []

        try:
            q = np.frombuffer(query_vector, dtype=np.float16).astype(np.float32)
        except Exception:
            return []

        if q.shape[0] != self._dim:
            return []

        norm = np.linalg.norm(q)
        if norm > 0:
            q = q / norm

        # Try hnswlib path
        if self._enabled:
            self._ensure_index()
            if self._index is not None:
                try:
                    labels, distances = self._index.knn_query(q.reshape(1, -1), k=min(k, len(self._vectors)))
                    results: list[tuple[str, float]] = []
                    for label, dist in zip(labels[0], distances[0]):
                        eid = self._id_map.get(label)
                        if eid:
                            # hnswlib uses L2 by default; convert to similarity
                            sim = 1.0 / (1.0 + float(dist))
                            results.append((eid, sim))
                    return results
                except Exception:
                    pass  # fall through to numpy path

        # Numpy batch cosine similarity (fast O(n) with small constant)
        return self._search_numpy(q, k)

    def rebuild(self, vectors: dict[str, bytes]):
        """Full index rebuild from entity_id → bytes mapping."""
        self._vectors.clear()
        self._id_map.clear()
        self._reverse_map.clear()
        self._next_id = 0
        self._index = None

        for eid, raw in vectors.items():
            try:
                vec = np.frombuffer(raw, dtype=np.float16).astype(np.float32)
                norm = np.linalg.norm(vec)
                if norm > 0:
                    vec = vec / norm
                self._vectors[eid] = vec.astype(np.float16)
            except Exception:
                continue

        self._dirty = True
        logger.info("HNSW rebuilt from %d vectors, now %d stored", len(vectors), len(self._vectors))

    def save(self, path: str | None = None):
        """Persist index and mapping to disk."""
        save_idx = path or str(INDEX_PATH)
        save_map = path + ".npz" if path else str(MAPPING_PATH)

        os.makedirs(os.path.dirname(save_idx), exist_ok=True)

        if self._enabled and self._index is not None:
            try:
                self._index.save_index(save_idx)
            except Exception as e:
                logger.warning("HNSW save_index failed: %s", e)

        # Always save mapping
        try:
            np.savez_compressed(save_map, **{eid: v for eid, v in self._vectors.items()})
        except Exception as e:
            logger.warning("HNSW save mapping failed: %s", e)

        self._dirty = False
        logger.info("HNSW saved (%d vectors)", len(self._vectors))

    def load(self, path: str | None = None) -> bool:
        """Load index and mapping from disk."""
        load_idx = path or str(INDEX_PATH)
        load_map = path + ".npz" if path else str(MAPPING_PATH)

        # Load vector mapping
        if os.path.exists(load_map):
            try:
                data = np.load(load_map, allow_pickle=False)
                self._vectors = {k: data[k] for k in data.files}
                self._next_id = 0
                self._id_map.clear()
                self._reverse_map.clear()
                if self._enabled:
                    self._ensure_index(from_scratch=True)
                    for eid, vec in self._vectors.items():
                        fid = self._next_id
                        self._next_id += 1
                        self._id_map[fid] = eid
                        self._reverse_map[eid] = fid
                        try:
                            self._index.add_items(
                                vec.astype(np.float32).reshape(1, -1), [fid]
                            )
                        except Exception:
                            self._index = None
                            break
                self._dirty = False
                logger.info("HNSW loaded (%d vectors)", len(self._vectors))
                return True
            except Exception as e:
                logger.warning("HNSW load mapping failed: %s", e)

        return False

    # ------------------------------------------------------------------
    # Internal
    # ------------------------------------------------------------------

    def _ensure_index(self, from_scratch: bool = False):
        """Create hnswlib index if not exists."""
        if not self._enabled:
            return
        if self._index is not None and not from_scratch:
            return

        try:
            self._index = hnswlib.Index(space="cosine", dim=self._dim)
            self._index.init_index(
                max_elements=max(self._max_elements, len(self._vectors) + 1000),
                ef_construction=200,
                M=16,
            )
            self._index.set_ef(50)
        except Exception as e:
            logger.warning("HNSW index init failed: %s", e)
            self._index = None

    def _search_numpy(self, query: np.ndarray, k: int) -> list[tuple[str, float]]:
        """Batch numpy cosine similarity search."""
        if not self._vectors:
            return []

        eids = list(self._vectors.keys())
        matrix = np.stack([self._vectors[eid].astype(np.float32) for eid in eids])
        # Cosine similarity via dot product (vectors are already normalized)
        scores = np.dot(matrix, query)
        top_indices = np.argsort(scores)[-k:][::-1]

        results: list[tuple[str, float]] = []
        for idx in top_indices:
            if scores[idx] > 0:
                results.append((eids[idx], float(scores[idx])))
        return results
