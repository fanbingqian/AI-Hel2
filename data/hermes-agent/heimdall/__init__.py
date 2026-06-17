"""HEIMDALL — 端侧智能体长期陪伴记忆体系.

Three-layer architecture:
  Core Engine  — Dual-track memory + knowledge (persona, entities, social graph)
  Mapping Fusion — Graph embedding, GMM clustering, PageRank (Phase 2)
  Human Views — Recent, History, Groups, Learning, Summary (Phase 2)

Phase 1 (MVP) delivers the Core Engine with retrieval, elevator, and integration.
"""

from heimdall.config import HeimdallConfig
from heimdall.manager import HeimdallManager
from heimdall.provider import HeimdallProvider

__all__ = ["HeimdallConfig", "HeimdallManager", "HeimdallProvider"]
