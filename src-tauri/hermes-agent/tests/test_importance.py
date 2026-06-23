"""Tests for P2.3 importance scoring engine."""
import pytest
from heimdall.core.entity_store.importance import (
    compute_importance,
    importance_level,
    batch_compute_importance,
    ImportanceEngine,
    W_CONFIDENCE,
    W_PAGERANK,
    W_OCCURRENCE,
    W_RECENCY,
    W_BRIDGE,
)


class TestImportanceLevel:
    def test_critical(self):
        assert importance_level(0.80) == "critical"
        assert importance_level(0.95) == "critical"
        assert importance_level(1.0) == "critical"

    def test_high(self):
        assert importance_level(0.60) == "high"
        assert importance_level(0.79) == "high"

    def test_medium(self):
        assert importance_level(0.35) == "medium"
        assert importance_level(0.59) == "medium"

    def test_low(self):
        assert importance_level(0.0) == "low"
        assert importance_level(0.34) == "low"
        assert importance_level(0.10) == "low"


class TestComputeImportance:
    def test_max_values(self):
        """All inputs at max → score should be close to 1.0."""
        score = compute_importance(
            confidence=1.0,
            pagerank=10.0,
            occurrence_count=100,
            last_seen_at=None,  # neutral recency
            bridge_score=1.0,
        )
        assert score > 0.8

    def test_min_values(self):
        """All inputs at min → score should be low."""
        score = compute_importance(
            confidence=0.0,
            pagerank=0.0,
            occurrence_count=0,
            last_seen_at=None,
            bridge_score=0.0,
        )
        assert score < 0.3

    def test_confidence_weight(self):
        """High confidence alone should contribute ~0.30."""
        score_high = compute_importance(confidence=1.0, pagerank=0.0, occurrence_count=0, bridge_score=0.0)
        score_low = compute_importance(confidence=0.0, pagerank=0.0, occurrence_count=0, bridge_score=0.0)
        diff = score_high - score_low
        assert 0.25 < diff < 0.35, f"Expected ~0.30 diff, got {diff}"

    def test_pagerank_weight(self):
        """High pagerank alone should contribute ~0.30."""
        score_high = compute_importance(confidence=0.0, pagerank=10.0, occurrence_count=0, bridge_score=0.0)
        score_low = compute_importance(confidence=0.0, pagerank=0.0, occurrence_count=0, bridge_score=0.0)
        diff = score_high - score_low
        assert 0.25 < diff < 0.35, f"Expected ~0.30 diff, got {diff}"

    def test_recency_fresh(self):
        """Recently seen entity should have high recency."""
        import time
        score_fresh = compute_importance(confidence=0.0, pagerank=0.0, occurrence_count=0, bridge_score=0.0, last_seen_at=time.time())
        # recency should be close to 1.0 for just-seen, contributing 0.15
        assert 0.10 < score_fresh < 0.20

    def test_recency_old(self):
        """Entity seen 90 days ago should have low recency."""
        import time
        ninety_days = 90 * 86400
        score_old = compute_importance(confidence=0.0, pagerank=0.0, occurrence_count=0, bridge_score=0.0, last_seen_at=time.time() - ninety_days)
        assert score_old < 0.05

    def test_output_range(self):
        """Output should always be in [0, 1]."""
        import time
        for conf in [0.0, 0.5, 1.0]:
            for pr in [0.0, 1.0, 5.0, 10.0, 50.0]:
                for occ in [0, 1, 10, 100, 1000]:
                    score = compute_importance(confidence=conf, pagerank=pr, occurrence_count=occ)
                    assert 0.0 <= score <= 1.0, f"Out of range: {score} for conf={conf} pr={pr} occ={occ}"

    def test_weights_sum_to_one(self):
        total = W_CONFIDENCE + W_PAGERANK + W_OCCURRENCE + W_RECENCY + W_BRIDGE
        assert abs(total - 1.0) < 0.001


class TestBatchCompute:
    def test_batch_empty(self):
        assert batch_compute_importance([]) == {}

    def test_batch_multiple(self):
        entities = [
            {"entity_id": "e1", "confidence": 1.0, "pagerank": 10.0, "occurrence_count": 100, "bridge_score": 1.0},
            {"entity_id": "e2", "confidence": 0.1, "pagerank": 0.1, "occurrence_count": 1, "bridge_score": 0.0},
        ]
        results = batch_compute_importance(entities)
        assert len(results) == 2
        assert "e1" in results
        assert "e2" in results
        score1, level1 = results["e1"]
        score2, level2 = results["e2"]
        assert score1 > score2
        assert level1 in ("critical", "high")
        assert level2 == "low"

    def test_batch_skips_empty_id(self):
        entities = [{"confidence": 0.5}]  # no entity_id
        assert batch_compute_importance(entities) == {}


class TestImportanceEngine:
    def test_level_counts_empty(self):
        """Engine.get_level_counts with no DB should return zeros."""
        class FakeStore:
            _conn = None
        engine = ImportanceEngine(FakeStore())
        counts = engine.get_level_counts()
        assert counts == {"critical": 0, "high": 0, "medium": 0, "low": 0}

    def test_top_entities_empty(self):
        class FakeStore:
            _conn = None
        engine = ImportanceEngine(FakeStore())
        assert engine.get_top_entities() == []

    def test_recalc_empty(self):
        class FakeStore:
            _conn = None
        engine = ImportanceEngine(FakeStore())
        assert engine.recalc_all() == 0
