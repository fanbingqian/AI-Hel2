"""HEIMDALL configuration schema and defaults."""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Optional

HEIMDALL_CONFIG_VERSION = 1

DEFAULT_HEIMDALL_CONFIG = {
    "heimdall": {
        "enabled": True,
        "hot_cabin_mb": 100,
        "cold_start_threshold": 5,
        "persona": {
            "immunity_enabled": True,
            "vector_guard_similarity": 0.85,
            "max_core_values": 5,
            "max_behavior_lines": 5,
            "user_profile_max_chars": 5000,
        },
        "privacy": {
            "salt": "",  # Auto-generated 128-bit salt on first run
            "hash_algorithm": "sha256",
        },
        "elevator": {
            "l1_safe_zone": "on-device",
            "l2_buffer": "local-best-effort",
            "l3_deep_water": "cloud-required",
            "l2_perplexity_threshold": 3.5,
        },
        "retrieval": {
            "vector_weight": 0.35,
            "keyword_weight": 0.25,
            "graph_weight": 0.20,
            "temporal_weight": 0.20,
            "top_k": 5,
        },
        "extraction": {
            "mode": "llm",  # "llm" or "onnx"
            "confidence_threshold": 0.6,
            "max_entities_per_turn": 8,
            "auto_decision": {
                "confidence_threshold_high": 0.7,
                "confidence_threshold_low": 0.3,
            },
        },
        "encryption": {
            "enabled": True,
            "algorithm": "fernet",
        },
        "ttl": {
            "originals_local_days": 7,
            "originals_summary_days": 30,
            "originals_archive_days": 90,
            "emotion_expire_days": 7,
        },
        "escalation": {
            "enabled": True,
            "max_entities_local": 20,
        },
        "views": {
            "reconnect": {
                "enabled": True,
                "inactivity_days": 90,
                "min_interactions": 10,
            },
            "summary": {
                "monthly_template_enabled": True,
                "pivot_moment_days": 7,
                "micro_agi_insight_max_chars": 200,
                "annual_narrative_enabled": True,
            },
        },
        "knowledge_namespace": None,
        "embedding": {
            "provider": "",
            "model": "",
            "base_url": "",
            "dimension": 1024,
        },
    },
}


@dataclass
class HeimdallConfig:
    """Resolved HEIMDALL configuration with defaults applied."""

    enabled: bool = True
    hot_cabin_mb: int = 100
    cold_start_threshold: int = 5

    # Persona
    persona_immunity_enabled: bool = True
    persona_vector_guard_similarity: float = 0.85
    persona_max_core_values: int = 5
    persona_max_behavior_lines: int = 5
    persona_user_profile_max_chars: int = 5000

    # Privacy
    privacy_salt: str = ""
    privacy_hash_algorithm: str = "sha256"

    # Elevator
    elevator_l1: str = "on-device"
    elevator_l2: str = "local-best-effort"
    elevator_l3: str = "cloud-required"
    elevator_l2_perplexity_threshold: float = 3.5

    # Retrieval
    retrieval_vector_weight: float = 0.35
    retrieval_keyword_weight: float = 0.25
    retrieval_graph_weight: float = 0.20
    retrieval_temporal_weight: float = 0.20
    retrieval_top_k: int = 5

    # Extraction
    extraction_mode: str = "llm"
    extraction_confidence_threshold: float = 0.6
    extraction_max_entities_per_turn: int = 8

    # Extraction auto-decision (V2.2)
    extraction_auto_decision_high: float = 0.7
    extraction_auto_decision_low: float = 0.3

    # Encryption (V2.2)
    encryption_enabled: bool = True
    encryption_algorithm: str = "fernet"

    # TTL (V2.2)
    ttl_originals_local_days: int = 7
    ttl_originals_summary_days: int = 30
    ttl_originals_archive_days: int = 90
    ttl_emotion_expire_days: int = 7

    # Escalation (V2.2)
    escalation_enabled: bool = True
    escalation_max_entities_local: int = 20

    # Views
    views_reconnect_enabled: bool = True
    views_reconnect_inactivity_days: int = 90
    views_reconnect_min_interactions: int = 10
    views_summary_monthly_enabled: bool = True
    views_summary_pivot_moment_days: int = 7
    views_summary_micro_agi_max_chars: int = 200
    views_summary_annual_enabled: bool = True

    # Namespace (V2.3)
    knowledge_namespace: Optional[str] = None

    # Embedding (V2.3)
    embedding_provider: str = ""
    embedding_model: str = ""
    embedding_base_url: str = ""
    embedding_dimension: int = 1024

    @classmethod
    def from_dict(cls, raw: dict) -> HeimdallConfig:
        """Parse from config.yaml dict, applying defaults for missing keys."""
        h = raw.get("heimdall", {})
        p = h.get("persona", {})
        priv = h.get("privacy", {})
        elev = h.get("elevator", {})
        retr = h.get("retrieval", {})
        extr = h.get("extraction", {})
        auto_dec = extr.get("auto_decision", {})
        enc = h.get("encryption", {})
        ttl_cfg = h.get("ttl", {})
        esc = h.get("escalation", {})
        v = h.get("views", {})
        rc = v.get("reconnect", {})
        sm = v.get("summary", {})
        emb = h.get("embedding", {})

        return cls(
            enabled=h.get("enabled", True),
            hot_cabin_mb=h.get("hot_cabin_mb", 100),
            cold_start_threshold=h.get("cold_start_threshold", 5),
            persona_immunity_enabled=p.get("immunity_enabled", True),
            persona_vector_guard_similarity=p.get("vector_guard_similarity", 0.85),
            persona_max_core_values=p.get("max_core_values", 5),
            persona_max_behavior_lines=p.get("max_behavior_lines", 5),
            persona_user_profile_max_chars=p.get("user_profile_max_chars", 5000),
            privacy_salt=priv.get("salt", ""),
            privacy_hash_algorithm=priv.get("hash_algorithm", "sha256"),
            elevator_l1=elev.get("l1_safe_zone", "on-device"),
            elevator_l2=elev.get("l2_buffer", "local-best-effort"),
            elevator_l3=elev.get("l3_deep_water", "cloud-required"),
            elevator_l2_perplexity_threshold=elev.get("l2_perplexity_threshold", 3.5),
            retrieval_vector_weight=retr.get("vector_weight", 0.35),
            retrieval_keyword_weight=retr.get("keyword_weight", 0.25),
            retrieval_graph_weight=retr.get("graph_weight", 0.20),
            retrieval_temporal_weight=retr.get("temporal_weight", 0.20),
            retrieval_top_k=retr.get("top_k", 5),
            extraction_mode=extr.get("mode", "llm"),
            extraction_confidence_threshold=extr.get("confidence_threshold", 0.6),
            extraction_max_entities_per_turn=extr.get("max_entities_per_turn", 8),
            extraction_auto_decision_high=auto_dec.get("confidence_threshold_high", 0.7),
            extraction_auto_decision_low=auto_dec.get("confidence_threshold_low", 0.3),
            encryption_enabled=enc.get("enabled", True),
            encryption_algorithm=enc.get("algorithm", "fernet"),
            ttl_originals_local_days=ttl_cfg.get("originals_local_days", 7),
            ttl_originals_summary_days=ttl_cfg.get("originals_summary_days", 30),
            ttl_originals_archive_days=ttl_cfg.get("originals_archive_days", 90),
            ttl_emotion_expire_days=ttl_cfg.get("emotion_expire_days", 7),
            escalation_enabled=esc.get("enabled", True),
            escalation_max_entities_local=esc.get("max_entities_local", 20),
            views_reconnect_enabled=rc.get("enabled", True),
            views_reconnect_inactivity_days=rc.get("inactivity_days", 90),
            views_reconnect_min_interactions=rc.get("min_interactions", 10),
            views_summary_monthly_enabled=sm.get("monthly_template_enabled", True),
            views_summary_pivot_moment_days=sm.get("pivot_moment_days", 7),
            views_summary_micro_agi_max_chars=sm.get("micro_agi_insight_max_chars", 200),
            views_summary_annual_enabled=sm.get("annual_narrative_enabled", True),
            knowledge_namespace=h.get("knowledge_namespace"),
            embedding_provider=emb.get("provider", ""),
            embedding_model=emb.get("model", ""),
            embedding_base_url=emb.get("base_url", ""),
            embedding_dimension=emb.get("dimension", 1024),
        )
