"""HEIMDALL FTRL-Proximal Online Learning — 6-dim retrieval sorter.

V3.0 Sections 3.3-3.4: FTRL-Proximal learner for retrieval ranking with
6 feature dimensions including learned Serendipity weight.

Phase 1 (MVP): fixed weights per spec
  vector_sim=0.5, pagerank=0.2, time_decay=0.2, freshness=0.1,
  bridge_boost=0.0, serendipity=0.15

Phase 2: FTRL online learning from user click feedback
  Serendipity weight learned; A/B test target: +10% CTR

Memory budget: ≤1MB for FTRL state (6 × 2 × float64 = 96 bytes + overhead)
"""

from __future__ import annotations

import json
import logging
import time
from pathlib import Path
from typing import Optional

import numpy as np

logger = logging.getLogger(__name__)

# Fixed retrieval weights for phase 1 (from V3.0 spec)
PHASE1_WEIGHTS = {
    "vector_sim": 0.5,
    "pagerank": 0.2,
    "time_decay": 0.2,
    "freshness": 0.1,
    "bridge_boost": 0.0,
    "serendipity": 0.15,
}

FEATURE_NAMES = [
    "vector_sim",     # vector similarity
    "pagerank",       # node importance
    "time_decay",     # time decay
    "bridge_boost",   # bridge node bonus
    "freshness",      # freshness
    "serendipity",    # cross-community novelty, learned
]

FEATURE_DIM = 6


class FTRLSorter:
    """V2.0：6维特征，FTRL自适应学习Serendipity权重

    Usage:
        sorter = FTRLSorter(model_path=heimdall_dir / "ftrl_state.json")
        score = sorter.score([0.8, 0.5, 0.3, 0.0, 0.5, 0.15])
        # ... user clicks or ignores ...
        sorter.record_feedback([0.8, 0.5, 0.3, 0.0, 0.5, 0.15], clicked=True)
        sorter.save()
    """

    def __init__(
        self,
        model_path: Optional[Path] = None,
        dim: int = FEATURE_DIM,
        alpha: float = 0.05,
        beta: float = 1.0,
        l1: float = 0.1,
        l2: float = 1.0,
    ):
        self.dim = dim
        self.alpha = alpha
        self.beta = beta
        self.l1 = l1
        self.l2 = l2

        # FTRL state
        self.z = np.zeros(dim, dtype=np.float64)
        self.n = np.zeros(dim, dtype=np.float64)

        self.feature_names = FEATURE_NAMES[:dim]
        self.model_path = model_path

        # Phase 1 flag — when True, uses fixed weights regardless of training
        self._phase1 = True
        self._feedback_count = 0
        self._created_at = time.time()

        # Try to load saved state
        if model_path and model_path.exists():
            self._load()

    # ------------------------------------------------------------------
    # Scoring
    # ------------------------------------------------------------------

    def score(self, features: list[float]) -> float:
        """Compute retrieval score from feature vector.

        Phase 1: fixed weights. Phase 2: FTRL-learned weights.
        Returns score in [0, 1].
        """
        if len(features) < self.dim:
            features = list(features) + [0.0] * (self.dim - len(features))

        x = np.array(features[:self.dim], dtype=np.float64)

        if self._phase1:
            w = np.array(
                [PHASE1_WEIGHTS.get(n, 0.0) for n in self.feature_names],
                dtype=np.float64,
            )
        else:
            w = self._get_weights()

        logit = np.dot(w, x)
        return float(1.0 / (1.0 + np.exp(-logit)))

    def predict(self, x: list[float]) -> float:
        """Alias for score(), compatible with spec naming."""
        return self.score(x)

    # ------------------------------------------------------------------
    # Online learning
    # ------------------------------------------------------------------

    def update(self, x: list[float], y: float):
        """FTRL update step. x = feature vector, y = label (1=click, 0=ignore)."""
        if len(x) < self.dim:
            x = list(x) + [0.0] * (self.dim - len(x))

        x_arr = np.array(x[:self.dim], dtype=np.float64)
        y_val = float(y)

        p = self.predict(x)
        g = (p - y_val) * x_arr

        for i in range(self.dim):
            sigma = (np.sqrt(self.n[i] + g[i] ** 2) - np.sqrt(self.n[i])) / self.alpha
            self.z[i] += g[i] - sigma * self.z[i]
            self.n[i] += g[i] ** 2

    def record_feedback(self, features: list[float], clicked: bool = True):
        """Record user feedback on a retrieval result.

        clicked=True means user clicked/interacted with the result.
        This triggers an FTRL weight update.
        """
        self._feedback_count += 1
        y = 1.0 if clicked else 0.0
        self.update(features, y)

        # After enough feedback, transition to phase 2 (learned weights)
        if self._feedback_count >= 20:
            self._phase1 = False

        logger.debug(
            "FTRL feedback #%d: clicked=%s, phase1=%s",
            self._feedback_count, clicked, self._phase1,
        )

    def get_weights(self) -> dict:
        """Return current feature weights (human-readable)."""
        w = self._get_weights()
        return dict(zip(self.feature_names, [float(v) for v in w]))

    # ------------------------------------------------------------------
    # Persistence
    # ------------------------------------------------------------------

    def save(self) -> None:
        """Save FTRL state to disk."""
        if not self.model_path:
            return
        state = {
            "z": self.z.tolist(),
            "n": self.n.tolist(),
            "feedback_count": self._feedback_count,
            "phase1": self._phase1,
            "feature_names": self.feature_names,
            "created_at": self._created_at,
            "saved_at": time.time(),
            "weights": self.get_weights(),
        }
        self.model_path.parent.mkdir(parents=True, exist_ok=True)
        self.model_path.write_text(
            json.dumps(state, ensure_ascii=False, indent=2), "utf-8"
        )

    def _load(self) -> None:
        """Load FTRL state from disk."""
        try:
            state = json.loads(self.model_path.read_text("utf-8"))
            self.z = np.array(state.get("z", [0.0] * self.dim), dtype=np.float64)
            self.n = np.array(state.get("n", [0.0] * self.dim), dtype=np.float64)
            self._feedback_count = state.get("feedback_count", 0)
            self._phase1 = state.get("phase1", True)
            self._created_at = state.get("created_at", time.time())
            logger.info(
                "FTRL loaded: %d feedback samples, phase1=%s",
                self._feedback_count, self._phase1,
            )
        except Exception as e:
            logger.warning("Failed to load FTRL state: %s", e)

    # ------------------------------------------------------------------
    # Internal
    # ------------------------------------------------------------------

    def _get_weights(self) -> np.ndarray:
        """Compute current weight vector from FTRL state."""
        w = np.zeros(self.dim, dtype=np.float64)
        for i in range(self.dim):
            if abs(self.z[i]) <= self.l1:
                w[i] = 0.0
            else:
                w[i] = -(
                    (self.z[i] - np.sign(self.z[i]) * self.l1)
                    / ((self.beta + np.sqrt(self.n[i])) / self.alpha + self.l2)
                )
        return w


# Module-level convenience
_ftrl_instance: Optional[FTRLSorter] = None


def get_ftrl(model_path: Optional[Path] = None) -> FTRLSorter:
    """Get or create the global FTRL sorter instance."""
    global _ftrl_instance
    if _ftrl_instance is None:
        _ftrl_instance = FTRLSorter(model_path=model_path)
    return _ftrl_instance


# ---------------------------------------------------------------------------
# V2.2: FTRL-driven auto-decision threshold learner
# ---------------------------------------------------------------------------

import math


class FTRLAutoDecision:
    """FTRL-driven auto-decision threshold learner (V2.2).

    Core idea:
      - Each user has independent confidence threshold parameters
      - User corrections → FTRL updates → thresholds adapt automatically
      - Target: 95% auto-confirmation rate, < 2% error rate
    """

    def __init__(self, alpha=0.05, beta=1.0, L1=0.1, L2=0.1):
        self.alpha = alpha
        self.beta = beta
        self.L1 = L1
        self.L2 = L2

        self.weights = {}   # feature_key → weight
        self.z = {}         # accumulated gradients
        self.n = {}         # accumulated squared gradients
        self._correction_count = 0

    def _feature_key(self, entity_type: str, relation_type: str,
                     domain: str, complexity: str) -> str:
        return f"{entity_type}:{relation_type}:{domain}:{complexity}"

    def predict_threshold(self, entity_type: str = "concept",
                          relation_type: str = "relates_to",
                          domain: str = "general",
                          complexity: str = "simple") -> float:
        """Predict optimal confidence threshold for current context.

        Returns 0.0-1.0 threshold; above this value → silent confirm.
        """
        key = self._feature_key(entity_type, relation_type, domain, complexity)
        w = self.weights.get(key, 0.0)
        base_threshold = 0.7
        return max(0.3, min(0.95, base_threshold + w))

    def update(self, entity_type: str, relation_type: str,
               domain: str, complexity: str,
               predicted_conf: float, user_corrected: bool):
        """Update FTRL parameters after user correction.

        Args:
            user_corrected: True if user corrected AI's decision
                           (means threshold was too low, AI was over-confident)
        """
        key = self._feature_key(entity_type, relation_type, domain, complexity)
        self._correction_count += 1

        y = -1.0 if user_corrected else 1.0

        z = self.z.get(key, 0.0)
        n = self.n.get(key, 0.0)

        p = self._sigmoid(predicted_conf * y)
        g = (p - (1.0 if y > 0 else 0.0)) * predicted_conf

        sigma = (math.sqrt(n + g * g) - math.sqrt(n)) / self.alpha
        z += g - sigma * self.weights.get(key, 0.0)
        n += g * g

        if abs(z) <= self.L1:
            self.weights[key] = 0.0
        else:
            self.weights[key] = -(
                (self.beta + math.sqrt(n)) / self.alpha + self.L2
            ) * (z - math.copysign(self.L1, z))

        self.z[key] = z
        self.n[key] = n

    @property
    def correction_count(self) -> int:
        return self._correction_count

    def get_state(self) -> dict:
        """Export FTRL state for persistence."""
        return {
            "weights": self.weights.copy(),
            "z": self.z.copy(),
            "n": self.n.copy(),
            "correction_count": self._correction_count,
        }

    def load_state(self, state: dict) -> None:
        """Import FTRL state from persistence."""
        self.weights = state.get("weights", {})
        self.z = state.get("z", {})
        self.n = state.get("n", {})
        self._correction_count = state.get("correction_count", 0)

    @staticmethod
    def _sigmoid(x: float) -> float:
        return 1.0 / (1.0 + math.exp(-max(min(x, 10.0), -10.0)))
