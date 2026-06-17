"""Knowledge tree assembly — hierarchical organization of entities.

Builds a multi-way tree from domain/subdomain relationships.
Each node represents an entity; the tree is organized by:
  - Domain entities at the root level
  - Subdomain entities as children
  - All other entities leaf under their domain via belongs_to relations
"""

from __future__ import annotations

import json
import logging
import time
from dataclasses import dataclass, field
from typing import Any, Optional

logger = logging.getLogger(__name__)


@dataclass
class TreeNode:
    entity_id: str
    name: str
    entity_type: str
    children: list["TreeNode"] = field(default_factory=list)
    importance: float = 0.5
    confidence: float = 0.5
    namespace: str = "general"
    is_leaf: bool = True
    depth: int = 0


class KnowledgeTreeBuilder:
    """Assemble entities into a hierarchical domain-based tree."""

    def __init__(self, store):
        self._store = store

    # ------------------------------------------------------------------
    # Public API
    # ------------------------------------------------------------------

    def build(
        self,
        namespace: str = "general",
        max_depth: int = 10,
    ) -> list[TreeNode]:
        """Build the full knowledge tree for a namespace.

        Root level: entities of type "domain" or "discipline"
        Next levels: entities with belongs_to→parent relations
        Leaf level: all other entities
        """
        entities = self._load_entities(namespace)
        relations = self._load_relations(namespace)

        # Identify domain/root entities
        domain_types = {"domain", "discipline"}
        roots = [e for e in entities if e.get("type_detail") in domain_types]

        # If no explicit domain entities, use entities with no incoming belongs_to
        if not roots:
            children_of = set()
            parent_of: dict[str, str] = {}
            for rel in relations:
                if rel["type"] == "belongs_to":
                    children_of.add(rel["source"])
                    parent_of[rel["source"]] = rel["target"]

            # Roots are entities that have children but no parent
            for e in entities:
                eid = e["entity_id"]
                if eid in children_of and eid not in parent_of:
                    roots.append(e)

            # Fallback: use high-importance entities
            if not roots:
                roots = sorted(entities, key=lambda e: e.get("confidence", 0), reverse=True)[:10]

        # Build children map
        children_map: dict[str, list[str]] = {r["entity_id"]: [] for r in roots}
        for rel in relations:
            if rel["type"] == "belongs_to":
                parent = rel["target"]
                child = rel["source"]
                if parent not in children_map:
                    children_map[parent] = []
                children_map[parent].append(child)

        # Build entity lookup
        entity_map: dict[str, dict] = {e["entity_id"]: e for e in entities}

        # Recursively build tree
        tree: list[TreeNode] = []
        for root_entity in roots:
            if root_entity["entity_id"] not in entity_map:
                continue
            node = self._build_node(
                root_entity["entity_id"], entity_map, children_map, 0, max_depth
            )
            tree.append(node)

        logger.info(
            "Tree built: %d root nodes, namespace=%s", len(tree), namespace
        )
        return tree

    def get_subtree(
        self,
        entity_id: str,
        namespace: str = "general",
        max_depth: int = 5,
    ) -> Optional[TreeNode]:
        """Get the subtree rooted at a specific entity."""
        entities = self._load_entities(namespace)
        relations = self._load_relations(namespace)
        entity_map = {e["entity_id"]: e for e in entities}

        if entity_id not in entity_map:
            return None

        children_map: dict[str, list[str]] = {}
        for rel in relations:
            if rel["type"] == "belongs_to":
                parent = rel["target"]
                child = rel["source"]
                children_map.setdefault(parent, []).append(child)

        return self._build_node(entity_id, entity_map, children_map, 0, max_depth)

    def get_breadcrumb(
        self,
        entity_id: str,
        namespace: str = "general",
    ) -> list[dict]:
        """Get the path from root to a given entity."""
        relations = self._load_relations(namespace)

        # Build parent map
        parent_of: dict[str, str] = {}
        for rel in relations:
            if rel["type"] == "belongs_to":
                parent_of[rel["source"]] = rel["target"]

        path: list[dict] = []
        current = entity_id
        visited: set[str] = set()

        conn = self._store._conn
        while current and current not in visited:
            row = conn.execute(
                "SELECT entity_id, name FROM kr_entities WHERE entity_id = ?",
                (current,),
            ).fetchone()
            if row:
                path.append({"entity_id": row["entity_id"], "name": row["name"]})
            visited.add(current)
            current = parent_of.get(current, "")

        path.reverse()
        return path

    def get_leaves(
        self,
        entity_id: str,
        namespace: str = "general",
    ) -> list[dict]:
        """Get all leaf entities under a given node."""
        subtree = self.get_subtree(entity_id, namespace)
        if not subtree:
            return []

        leaves: list[dict] = []

        def _collect_leaves(node: TreeNode):
            if node.is_leaf:
                leaves.append({
                    "entity_id": node.entity_id,
                    "name": node.name,
                    "type": node.entity_type,
                })
            for child in node.children:
                _collect_leaves(child)

        _collect_leaves(subtree)
        return leaves

    def search(
        self,
        query: str,
        namespace: str = "general",
        limit: int = 20,
    ) -> list[dict]:
        """Search the tree for entities matching a query, with breadcrumb context."""
        conn = self._store._conn
        rows = conn.execute(
            """SELECT entity_id, name, types FROM kr_entities
               WHERE namespace = ? AND name LIKE ?
               LIMIT ?""",
            (namespace, f"%{query}%", limit),
        ).fetchall()

        results: list[dict] = []
        for row in rows:
            breadcrumb = self.get_breadcrumb(row["entity_id"], namespace)
            results.append({
                "entity_id": row["entity_id"],
                "name": row["name"],
                "breadcrumb": [b["name"] for b in breadcrumb],
            })

        return results

    # ------------------------------------------------------------------
    # Internal
    # ------------------------------------------------------------------

    def _load_entities(self, namespace: str) -> list[dict]:
        conn = self._store._conn
        rows = conn.execute(
            """SELECT entity_id, name, types, type_detail, confidence, domains
               FROM kr_entities WHERE namespace = ? AND status != 'archived'""",
            (namespace,),
        ).fetchall()
        return [dict(r) for r in rows]

    def _load_relations(self, namespace: str) -> list[dict]:
        conn = self._store._conn
        rows = conn.execute(
            "SELECT source_id, target_id, type FROM kr_relations WHERE namespace = ?",
            (namespace,),
        ).fetchall()
        return [{"source": r[0], "target": r[1], "type": r[2]} for r in rows]

    def _build_node(
        self,
        entity_id: str,
        entity_map: dict[str, dict],
        children_map: dict[str, list[str]],
        depth: int,
        max_depth: int,
    ) -> TreeNode:
        entity = entity_map.get(entity_id, {})
        types_raw = entity.get("types", "concept")
        if isinstance(types_raw, str):
            try:
                types_list = json.loads(types_raw)
            except Exception:
                types_list = ["concept"]
        else:
            types_list = types_raw if types_raw else ["concept"]

        child_ids = children_map.get(entity_id, [])
        is_leaf = len(child_ids) == 0 or depth >= max_depth

        children: list[TreeNode] = []
        if not is_leaf:
            for cid in child_ids:
                if cid in entity_map:
                    children.append(
                        self._build_node(cid, entity_map, children_map, depth + 1, max_depth)
                    )

        return TreeNode(
            entity_id=entity_id,
            name=entity.get("name", entity_id),
            entity_type=types_list[0] if types_list else "concept",
            children=children,
            importance=entity.get("importance", 0.5),
            confidence=entity.get("confidence", 0.5),
            namespace=entity.get("namespace", "general"),
            is_leaf=is_leaf and len(children) == 0,
            depth=depth,
        )
