"""HEIMDALL Multi-Path Retrieval — query-classified retrieval engine.

Knowledge Ring V1.0: 6 retrieval paths routed by query type classification:
  1. Exact fact → entities + relations (structured data)
  2. Fuzzy name → entities_fts + aliases_fts
  3. Relation query → relations network
  4. Source request → originals_fts
  5. Preference → profiles_fts
  6. Vague association → all FTS + HRR vector

V2.2: Registry pattern with BaseRetriever ABC + 6 implementations + UnifiedRetrieval.
MultiPathRetriever is retained for backward compatibility.
"""

from __future__ import annotations

import logging
import re
from typing import Any, Optional

from heimdall.core.entity_store import EntityStore

logger = logging.getLogger(__name__)

DEFAULT_VECTOR_WEIGHT = 0.35
DEFAULT_KEYWORD_WEIGHT = 0.25
DEFAULT_GRAPH_WEIGHT = 0.20
DEFAULT_TEMPORAL_WEIGHT = 0.20
DEFAULT_TOP_K = 5

# Query classification patterns (Chinese + English)
_QUERY_PATTERNS = {
    "source_request": [
        r"(把|发|给|发我|给我|发一下|给我看).*(报告|原文|文件|文章)",
        r"(show|send|give).*(report|original|file|document|article)",
        r"(原文|原始|全文).*",
        r"原文",
    ],
    "relation_query": [
        r"(什么关系|有什么关系|关系是什么|关联|联系|怎么关联)",
        r"(how.*(related|connect)|what.*relation)",
        r"(导致|引起|造成|影响|取决于|因为|causes|leads? to|depends on)",
    ],
    "preference": [
        r"(喜欢|偏好|习惯|通常|一般|经常|讨厌|介意|想|要|推荐|建议)",
        r"(prefer|like|hate|usually|often|recommend|suggest)",
        r"(踏青|吃|喝|玩|去|买|看|听|做|怎么|哪里|哪家)",
    ],
    "fuzzy_name": [
        r"(那个|那个叫|叫什么来着|那个谁|上回|上次|之前说的)",
        r"(what was|who was|that thing|remember)",
    ],
    "exact_fact": [
        r"(是多少|等于|定义|什么是|属性|参数|how much|what is|define)",
    ],
}


class MultiPathRetriever:
    """Multi-path retrieval engine with query-type-based routing.

    Merges results from multiple retrieval paths, then scores and ranks.
    """

    def __init__(
        self,
        store: EntityStore,
        vector_weight: float = DEFAULT_VECTOR_WEIGHT,
        keyword_weight: float = DEFAULT_KEYWORD_WEIGHT,
        graph_weight: float = DEFAULT_GRAPH_WEIGHT,
        temporal_weight: float = DEFAULT_TEMPORAL_WEIGHT,
        top_k: int = DEFAULT_TOP_K,
    ):
        self.store = store
        self.vector_weight = vector_weight
        self.keyword_weight = keyword_weight
        self.graph_weight = graph_weight
        self.temporal_weight = temporal_weight
        self.top_k = top_k

    # ------------------------------------------------------------------
    # Main API
    # ------------------------------------------------------------------

    def retrieve(self, query: str, session_id: str = "", namespace: str = None) -> list[dict]:
        """Retrieve relevant entities and knowledge for a query (V2.3: + namespace).

        Routes to different search paths based on query type classification.
        When namespace is provided, filters results to that domain.
        """
        if not query.strip():
            return []

        qtype = self._classify_query(query)
        results: dict[str, dict] = {}

        if qtype == "source_request":
            self._add_originals_results(query, results, self.keyword_weight)
        elif qtype == "relation_query":
            self._add_relation_results(query, results, self.keyword_weight)
            self._add_keyword_results(query, results, self.keyword_weight * 0.5)
        elif qtype == "preference":
            self._add_profiles_results(query, results, self.keyword_weight)
            self._add_keyword_results(query, results, self.keyword_weight * 0.3)
        elif qtype == "fuzzy_name":
            self._add_kr_keyword_results(query, results, self.keyword_weight)
            self._add_alias_results(query, results, self.keyword_weight)
        elif qtype == "exact_fact":
            self._add_keyword_results(query, results, self.keyword_weight)
            self._add_relation_results(query, results, self.keyword_weight * 0.5)

        # Always run default paths for fallback
        if not results:
            self._add_keyword_results(query, results, self.keyword_weight)
            self._add_knowledge_results(query, results, self.keyword_weight * 0.5)
            self._add_graph_results(query, results, self.graph_weight)
        else:
            # Augment with graph neighbors and temporal boost
            self._add_graph_results(query, results, self.graph_weight * 0.5)

        self._add_temporal_boost(results, self.temporal_weight)

        ranked = sorted(results.values(), key=lambda r: r.get("score", 0), reverse=True)

        # V2.3: namespace filter
        if namespace:
            ranked = [r for r in ranked if r.get("namespace", "general") == namespace]

        return ranked[:self.top_k]

    def prefetch_context(self, query: str, namespace: str = None) -> str:
        """Generate a context block for the system prompt (V2.3: + namespace)."""
        items = self.retrieve(query, namespace=namespace)
        if not items:
            return ""

        lines = ["[HEIMDALL 记忆上下文]"]
        for item in items:
            name = item.get("name", "未知")
            etype = item.get("type", "")
            reason = self._explain_result(item)
            type_label = {"person": "联系人", "project": "项目", "skill": "技能",
                          "concept": "概念", "event": "事件", "location": "地点",
                          "organization": "组织", "tool": "工具", "content": "内容",
                          "artifact": "作品"}.get(etype, "实体")
            lines.append(f"- [{type_label}] {name}: {reason}")
        return "\n".join(lines)

    # ------------------------------------------------------------------
    # Query classification
    # ------------------------------------------------------------------

    def _classify_query(self, query: str) -> str:
        """Classify the query type for routing."""
        query_lower = query.lower()
        for qtype, patterns in _QUERY_PATTERNS.items():
            for pat in patterns:
                if re.search(pat, query_lower):
                    return qtype
        return "vague"

    # ------------------------------------------------------------------
    # Retrieval paths (original)
    # ------------------------------------------------------------------

    def _add_keyword_results(self, query: str, results: dict, weight: float) -> None:
        """FTS5 keyword search on entities (old table)."""
        entities = self.store.search_entities(query, limit=10)
        for ent in entities:
            eid = ent["entity_id"]
            score = weight * ent.get("confidence", 0.5)
            if eid in results:
                results[eid]["score"] += score
                results[eid]["sources"].append("keyword")
            else:
                results[eid] = {
                    "entity_id": eid,
                    "name": ent.get("display_name", ""),
                    "type": ent.get("entity_type", ""),
                    "score": score,
                    "confidence": ent.get("confidence", 0.5),
                    "occurrence_count": ent.get("occurrence_count", 1),
                    "last_seen_at": ent.get("last_seen_at", 0),
                    "sources": ["keyword"],
                }

    def _add_knowledge_results(self, query: str, results: dict, weight: float) -> None:
        """FTS5 search on knowledge entries."""
        kentries = self.store.search_knowledge(query, limit=5)
        for k in kentries:
            kid = k["entry_id"]
            score = weight * k.get("confidence", 0.5)
            if kid in results:
                results[kid]["score"] += score
                results[kid]["sources"].append("knowledge")
            else:
                results[kid] = {
                    "entity_id": kid,
                    "name": k.get("title", ""),
                    "type": "knowledge",
                    "score": score,
                    "confidence": k.get("confidence", 0.5),
                    "domain": k.get("domain", ""),
                    "mastery_level": k.get("mastery_level", ""),
                    "sources": ["knowledge"],
                }

    def _add_graph_results(self, query: str, results: dict, weight: float) -> None:
        """Graph-based retrieval: 2-hop neighbors of top keyword matches."""
        top_entities = sorted(
            [(eid, r) for eid, r in results.items() if r.get("type") != "knowledge"],
            key=lambda x: x[1].get("score", 0),
            reverse=True,
        )[:3]

        seen = set(results.keys())
        for eid, _ in top_entities:
            edges = self.store.get_social_edges(eid)
            for edge in edges:
                neighbor_id = (
                    edge["target_entity_id"]
                    if edge["source_entity_id"] == eid
                    else edge["source_entity_id"]
                )
                if neighbor_id in seen:
                    continue
                seen.add(neighbor_id)
                neighbor = self.store.get_entity(neighbor_id)
                if neighbor:
                    results[neighbor_id] = {
                        "entity_id": neighbor_id,
                        "name": neighbor.get("display_name", ""),
                        "type": neighbor.get("entity_type", ""),
                        "score": weight * edge.get("intensity", 0.1),
                        "confidence": neighbor.get("confidence", 0.5),
                        "occurrence_count": neighbor.get("occurrence_count", 1),
                        "last_seen_at": neighbor.get("last_seen_at", 0),
                        "sources": ["graph"],
                        "connected_via": results.get(eid, {}).get("name", ""),
                    }

    def _add_temporal_boost(self, results: dict, weight: float) -> None:
        """Boost recently seen entities."""
        import time
        now = time.time()
        for r in results.values():
            last_seen = r.get("last_seen_at", 0)
            if last_seen > 0:
                hours_ago = (now - last_seen) / 3600
                recency = 1.0 / (1.0 + hours_ago / 24)
                r["score"] += weight * recency
                r["temporal_boost"] = weight * recency

    # ------------------------------------------------------------------
    # Knowledge Ring V1.0 retrieval paths (new)
    # ------------------------------------------------------------------

    def _add_kr_keyword_results(self, query: str, results: dict, weight: float) -> None:
        """FTS5 search on Knowledge Ring entities."""
        try:
            entities = self.store.search_entities_v2(query, limit=10)
        except Exception:
            return
        for ent in entities:
            eid = ent["entity_id"]
            score = weight * ent.get("confidence", 0.5)
            if eid in results:
                results[eid]["score"] += score
                results[eid]["sources"].append("kr_keyword")
            else:
                results[eid] = {
                    "entity_id": eid,
                    "name": ent.get("name", ""),
                    "type": ent.get("type", ""),
                    "score": score,
                    "confidence": ent.get("confidence", 0.5),
                    "type_detail": ent.get("type_detail", ""),
                    "sources": ["kr_keyword"],
                }

    def _add_alias_results(self, query: str, results: dict, weight: float) -> None:
        """FTS5 search on aliases table."""
        try:
            aliases = self.store.search_aliases(query, limit=10)
        except Exception:
            return
        for a in aliases:
            eid = a["entity_id"]
            score = weight * 0.4
            if eid in results:
                results[eid]["score"] += score
                results[eid]["sources"].append("alias")
            else:
                results[eid] = {
                    "entity_id": eid,
                    "name": a.get("entity_name", ""),
                    "type": a.get("entity_type", ""),
                    "score": score,
                    "confidence": 0.5,
                    "sources": ["alias"],
                    "matched_alias": a.get("name", ""),
                }

    def _add_relation_results(self, query: str, results: dict, weight: float) -> None:
        """FTS5 search on relations source_text + structured relation lookup."""
        try:
            rels = self.store.search_relations(query, limit=10)
        except Exception:
            return
        seen = set(results.keys())
        for r in rels:
            # Add source entity
            src_id = r["source_id"]
            if src_id not in seen:
                seen.add(src_id)
                results[src_id] = {
                    "entity_id": src_id,
                    "name": r.get("source_name", ""),
                    "type": r.get("source_type", ""),
                    "score": weight * r.get("confidence", 0.5),
                    "confidence": r.get("confidence", 0.5),
                    "sources": ["relation"],
                    "relation_type": r.get("type", ""),
                }
            # Add target entity
            tgt_id = r["target_id"]
            if tgt_id not in seen:
                seen.add(tgt_id)
                results[tgt_id] = {
                    "entity_id": tgt_id,
                    "name": r.get("target_name", ""),
                    "type": r.get("target_type", ""),
                    "score": weight * r.get("confidence", 0.5) * 0.8,
                    "confidence": r.get("confidence", 0.5),
                    "sources": ["relation"],
                    "relation_type": r.get("type", ""),
                }

    def _add_originals_results(self, query: str, results: dict, weight: float) -> None:
        """FTS5 search on originals (archived source texts)."""
        try:
            originals = self.store.search_originals(query, limit=5)
        except Exception:
            return
        for i, o in enumerate(originals):
            oid = f"original_{o['original_id']}"
            results[oid] = {
                "entity_id": oid,
                "name": f"原文#{o['original_id']}",
                "type": "original",
                "score": weight * 0.6 / (i + 1),
                "confidence": 0.5,
                "sources": ["original"],
                "source_type": o.get("source_type", ""),
                "created_at": str(o.get("created_at", "")),
                "content_preview": (o.get("content", "") or "")[:200],
            }

    def _add_profiles_results(self, query: str, results: dict, weight: float) -> None:
        """FTS5 search on user profiles."""
        try:
            profiles = self.store.search_profiles(query, limit=5)
        except Exception:
            return
        for i, p in enumerate(profiles):
            pid = f"profile_{p['profile_id']}"
            results[pid] = {
                "entity_id": pid,
                "name": f"偏好: {p.get('content', '')[:60]}",
                "type": "profile",
                "score": weight * p.get("confidence", 0.5) / (i + 1),
                "confidence": p.get("confidence", 0.5),
                "sources": ["profile"],
                "profile_type": p.get("type", ""),
            }

    # ------------------------------------------------------------------
    # Helpers
    # ------------------------------------------------------------------

    def _explain_result(self, item: dict) -> str:
        """Generate a human-readable explanation for a retrieval result."""
        sources = item.get("sources", [])
        reasons = []

        if "keyword" in sources:
            reasons.append("关键词匹配")
        if "kr_keyword" in sources:
            reasons.append("知识环匹配")
        if "alias" in sources:
            alias = item.get("matched_alias", "")
            reasons.append(f"别名'{alias}'匹配" if alias else "别名匹配")
        if "relation" in sources:
            rtype = item.get("relation_type", "")
            reasons.append(f"关系'{rtype}'匹配" if rtype else "关系匹配")
        if "original" in sources:
            reasons.append("原文匹配")
        if "profile" in sources:
            reasons.append("偏好匹配")
        if "graph" in sources:
            via = item.get("connected_via", "")
            reasons.append(f"与'{via}'相关" if via else "社交图谱关联")
        if "knowledge" in sources:
            reasons.append("知识库匹配")
        if item.get("temporal_boost", 0) > 0.01:
            reasons.append("最近提及")

        if item.get("occurrence_count", 0) > 3:
            reasons.append(f"提及{item['occurrence_count']}次")

        domain = item.get("domain", "")
        if domain:
            reasons.append(f"领域: {domain}")

        return "；".join(reasons) if reasons else "相关结果"


# ---------------------------------------------------------------------------
# V2.2: Registry pattern — pluggable retrievers
# ---------------------------------------------------------------------------

from abc import ABC, abstractmethod


class BaseRetriever(ABC):
    """Retriever base class (V2.2: registry pattern, no more if-else chains)."""

    name: str = ""
    priority: int = 0

    @abstractmethod
    def search(self, query: str, query_embedding: Optional[bytes] = None,
               top_k: int = 5) -> list[dict]:
        """Execute search. Returns scored result list."""


class StructuredRetriever(BaseRetriever):
    """Structured retriever — precise facts via entities + relations."""

    name = "precise_fact"
    priority = 1

    def __init__(self, store):
        self.store = store

    def search(self, query, query_embedding=None, top_k=5):
        results = {}
        entities = self.store.search_entities_v2(query, limit=top_k)
        for e in entities:
            eid = e["entity_id"]
            results[eid] = {
                "entity_id": eid, "name": e.get("name", ""),
                "type": e.get("type", ""), "types": e.get("types", []),
                "score": 0.8 * e.get("confidence", 0.5),
                "confidence": e.get("confidence", 0.5),
                "source": "structured",
            }
        rels = self.store.search_relations(query, limit=top_k)
        for r in rels:
            rid = f"rel_{r['relation_id']}"
            results[rid] = {
                "entity_id": rid, "name": f"{r.get('source_name','')} → {r.get('target_name','')}",
                "type": "relation", "relation_type": r.get("type", ""),
                "score": 0.7 * r.get("confidence", 0.5),
                "source": "relation",
            }
        return sorted(results.values(), key=lambda x: x["score"], reverse=True)[:top_k]


class FTSRetriever(BaseRetriever):
    """Full-text retriever — fuzzy name matching via entities_fts + aliases_fts."""

    name = "name_fuzzy"
    priority = 2

    def __init__(self, store):
        self.store = store

    def search(self, query, query_embedding=None, top_k=5):
        results = {}
        entities = self.store.search_entities_v2(query, limit=top_k)
        for e in entities:
            eid = e["entity_id"]
            results[eid] = {
                "entity_id": eid, "name": e.get("name", ""),
                "type": e.get("type", ""),
                "score": 0.7 * e.get("confidence", 0.5),
                "source": "fts_entity",
            }
        aliases = self.store.search_aliases(query, limit=top_k)
        for a in aliases:
            eid = a["entity_id"]
            if eid not in results:
                results[eid] = {
                    "entity_id": eid, "name": a.get("entity_name", ""),
                    "type": a.get("entity_types", ""),
                    "score": 0.5,
                    "source": "fts_alias",
                    "matched_alias": a.get("name", ""),
                }
        return sorted(results.values(), key=lambda x: x["score"], reverse=True)[:top_k]


class RelationGraphRetriever(BaseRetriever):
    """Relation graph retriever — structured relation path lookup."""

    name = "relation_path"
    priority = 3

    def __init__(self, store):
        self.store = store

    def search(self, query, query_embedding=None, top_k=5):
        results = {}
        rels = self.store.search_relations(query, limit=top_k * 2)
        seen = set()
        for r in rels:
            for eid, name, etypes in [
                (r["source_id"], r.get("source_name", ""), r.get("source_types", "")),
                (r["target_id"], r.get("target_name", ""), r.get("target_types", "")),
            ]:
                if eid not in seen:
                    seen.add(eid)
                    results[eid] = {
                        "entity_id": eid, "name": name,
                        "type": etypes[0] if isinstance(etypes, list) and etypes else str(etypes),
                        "score": 0.6 * r.get("confidence", 0.5),
                        "source": "graph_relation",
                        "relation_type": r.get("type", ""),
                    }
        return sorted(results.values(), key=lambda x: x["score"], reverse=True)[:top_k]


class OriginalsRetriever(BaseRetriever):
    """Originals retriever — full-text search on archived source texts."""

    name = "original_text"
    priority = 4

    def __init__(self, store):
        self.store = store

    def search(self, query, query_embedding=None, top_k=5):
        originals = self.store.search_originals(query, limit=top_k)
        return [
            {
                "entity_id": f"orig_{o['original_id']}",
                "name": f"原文#{o['original_id']}",
                "type": "original",
                "score": 0.5 / (i + 1),
                "source": "original",
                "source_type": o.get("source_type", ""),
                "content_preview": (o.get("content", "") or "")[:200],
            }
            for i, o in enumerate(originals)
        ]


class ProfileRetriever(BaseRetriever):
    """Profile retriever — encrypted profile search with local decryption."""

    name = "preference"
    priority = 5

    def __init__(self, store):
        self.store = store

    def search(self, query, query_embedding=None, top_k=5):
        profiles = self.store.search_profiles(query, limit=top_k)
        return [
            {
                "entity_id": f"prof_{p['profile_id']}",
                "name": f"偏好: {p.get('content', '')[:60]}",
                "type": "profile",
                "score": 0.4 * p.get("confidence", 0.5) / (i + 1),
                "source": "profile",
                "profile_type": p.get("type", ""),
            }
            for i, p in enumerate(profiles)
        ]


class VectorRetriever(BaseRetriever):
    """Vector retriever — memory_embedding cosine similarity search.

    Uses HNSW approximate index when hnswlib is available (O(log n)),
    falls back to batch numpy cosine similarity (O(n) with small constant).
    Both paths are significantly faster than the per-row Python loop.
    """

    name = "fuzzy_association"
    priority = 6

    def __init__(self, store):
        self.store = store
        self._hnsw = None
        self._index_built = False

    def _get_hnsw(self):
        if self._hnsw is None:
            try:
                from heimdall.core.hnsw_index import HNSWIndex
                self._hnsw = HNSWIndex()
                # Try loading persisted index
                if not self._hnsw.load():
                    self._build_index()
                self._index_built = True
            except Exception:
                self._hnsw = None
        return self._hnsw

    def _build_index(self):
        if not self.store._conn or self._hnsw is None:
            return
        rows = self.store._conn.execute(
            "SELECT entity_id, name, types, memory_embedding FROM kr_entities "
            "WHERE memory_embedding IS NOT NULL"
        ).fetchall()
        for row in rows:
            try:
                self._hnsw.add(row["entity_id"], row["memory_embedding"])
            except Exception:
                continue
        self._index_built = True

    def _maybe_rebuild_if_stale(self):
        """Rebuild HNSW index if DB has new embeddings not yet indexed."""
        if self._hnsw is None or not self.store._conn:
            return
        try:
            db_count = self.store._conn.execute(
                "SELECT COUNT(*) as c FROM kr_entities WHERE memory_embedding IS NOT NULL"
            ).fetchone()["c"]
            if db_count != self._hnsw.count:
                self._build_index()
        except Exception:
            pass

    def search(self, query, query_embedding=None, top_k=5, **kwargs):
        if query_embedding is None:
            return []
        try:
            import numpy as np
        except ImportError:
            return []

        # Try HNSW first
        hnsw = self._get_hnsw()
        if hnsw is not None and hnsw.count > 0:
            try:
                self._maybe_rebuild_if_stale()
                neighbors = hnsw.search(query_embedding, k=max(top_k, 10))
                if neighbors:
                    eids = [n[0] for n in neighbors]
                    scores = {n[0]: n[1] for n in neighbors}
                    placeholders = ",".join("?" for _ in eids)
                    rows = self.store._conn.execute(
                        f"SELECT entity_id, name, types FROM kr_entities "
                        f"WHERE entity_id IN ({placeholders})",
                        eids,
                    ).fetchall() if self.store._conn else []
                    results = []
                    for row in rows:
                        results.append({
                            "entity_id": row["entity_id"],
                            "name": row["name"],
                            "type": row["types"],
                            "similarity": scores.get(row["entity_id"], 0),
                            "score": scores.get(row["entity_id"], 0),
                            "source": "vector",
                        })
                    results.sort(key=lambda x: x["similarity"], reverse=True)
                    return results[:top_k]
            except Exception:
                pass

        # Fallback: batch numpy cosine similarity
        rows = self.store._conn.execute(
            "SELECT entity_id, name, types, memory_embedding FROM kr_entities "
            "WHERE memory_embedding IS NOT NULL"
        ).fetchall() if self.store._conn else []

        if not rows:
            return []

        eids = [r["entity_id"] for r in rows]
        # Stack all vectors into one matrix
        vectors = np.stack([np.frombuffer(r["memory_embedding"], dtype=np.float16).astype(np.float32) for r in rows])
        query_vec = np.frombuffer(query_embedding, dtype=np.float16).astype(np.float32)

        # Normalize for cosine similarity
        q_norm = np.linalg.norm(query_vec)
        if q_norm > 0:
            query_vec = query_vec / q_norm
        v_norms = np.linalg.norm(vectors, axis=1, keepdims=True)
        v_norms[v_norms == 0] = 1.0
        vectors = vectors / v_norms

        # Cosine similarity in one matmul
        scores = np.dot(vectors, query_vec)
        top_indices = np.argsort(scores)[-max(top_k, 10):][::-1]

        results = []
        for idx in top_indices:
            sim = float(scores[idx])
            if sim > 0:
                results.append({
                    "entity_id": rows[idx]["entity_id"],
                    "name": rows[idx]["name"],
                    "type": rows[idx]["types"],
                    "similarity": sim,
                    "score": sim,
                    "source": "vector",
                })
        return results[:top_k]


class UnifiedRetrieval:
    """Unified retrieval entry (V2.2: registry pattern, pluggable).

    Replaces if-else chains with named retrievers. Supports dynamic registration.
    """

    def __init__(self, store):
        self.store = store
        self._retrievers: dict[str, BaseRetriever] = {}
        self._register_defaults()

    def _register_defaults(self):
        for cls in [
            StructuredRetriever, FTSRetriever, RelationGraphRetriever,
            OriginalsRetriever, ProfileRetriever, VectorRetriever,
        ]:
            instance = cls(self.store)
            self._retrievers[instance.name] = instance

    def register(self, retriever: BaseRetriever):
        self._retrievers[retriever.name] = retriever

    def retrieve(self, query: str, query_embedding: Optional[bytes] = None,
                 top_k: int = 5, namespace: str = None) -> list[dict]:
        intent = self._classify_intent(query)
        retriever = self._retrievers.get(intent)
        if retriever:
            results = retriever.search(query, query_embedding, top_k, namespace=namespace)
            if results:
                return results
        return self._hybrid_all(query, query_embedding, top_k, namespace=namespace)

    def _classify_intent(self, query: str) -> str:
        import re
        query_lower = query.lower()
        classifications = [
            (["原文", "原始", "全文", "show.*original", "send.*file", "把.*发"], "original_text"),
            (["什么关系", "关联", "怎么关联", "how.*related", "causes", "导致"], "relation_path"),
            (["喜欢", "偏好", "习惯", "prefer", "recommend", "推荐"], "preference"),
            (["那个", "叫什么来着", "上回", "上次", "who was", "remember"], "name_fuzzy"),
            (["是多少", "等于", "定义", "什么是", "how much", "what is"], "precise_fact"),
        ]
        for patterns, intent in classifications:
            for pat in patterns:
                if re.search(pat, query_lower):
                    return intent
        return "fuzzy_association"

    def _hybrid_all(self, query, query_embedding, top_k, namespace=None):
        all_results = []
        for name, retriever in self._retrievers.items():
            try:
                results = retriever.search(query, query_embedding, top_k * 2)
                for r in results:
                    r["_retriever"] = name
                    r["_score"] = r.get("score", r.get("similarity", 0.3)) * retriever.priority
                all_results.extend(results)
            except Exception:
                continue
        seen = set()
        unique = []
        for r in sorted(all_results, key=lambda x: x.get("_score", 0), reverse=True):
            eid = r.get("entity_id", "")
            if eid and eid not in seen:
                if namespace and r.get("namespace", "general") != namespace:
                    continue
                seen.add(eid)
                unique.append(r)
                if len(unique) >= top_k:
                    break
        return unique
