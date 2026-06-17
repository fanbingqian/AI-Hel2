"""Embedding vector computation client (V2.3).

Calls the configured embedding model API to generate memory_embedding
vectors for knowledge entities. Runs asynchronously — write path sets
memory_embedding=NULL first; a background consumer computes and backfills.

Config priority: embedding.model → model.default
                 embedding.base_url → provider.base_url
"""

import json
import logging
import struct
import threading
import time
import urllib.request
from pathlib import Path
from typing import Optional

logger = logging.getLogger(__name__)

HEIMDALL_HOME = Path.home() / ".heimdall"

MEMORY_EMBEDDING_SPEC = {
    "dimension": 1024,
    "quantization": "float16",
    "pooling": "mean",
    "normalize": True,
}


class EmbeddingClient:
    """Calls embedding model API to compute memory vectors (1024-dim FP16)."""

    def __init__(self, config: dict):
        embedding_cfg = config.get("embedding", {})
        model_cfg = config.get("model", {})

        self.model = embedding_cfg.get("model") or model_cfg.get("default", "")
        provider = embedding_cfg.get("provider") or model_cfg.get("provider", "deepseek")

        self.base_url = embedding_cfg.get("base_url", "")
        if not self.base_url and provider in config:
            self.base_url = config[provider].get("base_url", "")

        self.dimension = embedding_cfg.get("dimension", 1024)
        self._enabled = bool(self.model and self.base_url)

        if not self._enabled:
            logger.info(
                "EmbeddingClient disabled: model=%r base_url=%r",
                self.model, self.base_url,
            )

    @property
    def enabled(self) -> bool:
        return self._enabled

    def compute(self, text: str) -> Optional[bytes]:
        """POST {base_url}/embeddings → quantize → return 2048 bytes.

        Returns None if disabled or on API failure.
        """
        if not self._enabled or not text.strip():
            return None

        try:
            url = f"{self.base_url.rstrip('/')}/embeddings"
            body = json.dumps({
                "model": self.model,
                "input": text,
                "encoding_format": "float",
            }).encode("utf-8")

            req = urllib.request.Request(
                url,
                data=body,
                headers={"Content-Type": "application/json"},
            )
            resp = urllib.request.urlopen(req, timeout=30)
            data = json.loads(resp.read().decode("utf-8"))

            vector = data["data"][0]["embedding"]
            if len(vector) != self.dimension:
                logger.warning(
                    "Embedding dimension mismatch: expected %d, got %d",
                    self.dimension, len(vector),
                )
                return None

            return self._quantize_float16(vector)

        except Exception as e:
            logger.warning("Embedding API call failed: %s", e)
            return None

    @staticmethod
    def _quantize_float16(vector: list[float]) -> bytes:
        """Quantize float32 list → float16 bytes (2048 bytes for 1024-dim)."""
        import math
        buf = bytearray()
        for v in vector:
            # Clamp to float16 range
            v = max(-65504.0, min(65504.0, v))
            # Simple float32 → float16 conversion
            f32 = struct.pack("f", v)
            # Extract sign, exponent, mantissa
            n = struct.unpack("I", f32)[0]
            sign = (n >> 16) & 0x8000
            exponent = ((n >> 23) & 0xFF) - 127 + 15
            mantissa = (n >> 13) & 0x3FF
            if exponent <= 0:
                exponent = 0
                mantissa = 0
            elif exponent >= 31:
                exponent = 31
                mantissa = 0
            half = sign | (exponent << 10) | mantissa
            buf.extend(struct.pack("H", half))
        return bytes(buf)


class EmbeddingQueue:
    """Background consumer that computes embeddings for entities with NULL memory_embedding."""

    POLL_INTERVAL = 5.0

    def __init__(self, client: EmbeddingClient, store):
        self._client = client
        self._store = store
        self._running = False
        self._thread: Optional[threading.Thread] = None

    def start(self):
        if not self._client.enabled:
            return
        self._running = True
        self._thread = threading.Thread(target=self._loop, daemon=True)
        self._thread.start()
        logger.info("EmbeddingQueue started")

    def stop(self):
        self._running = False
        if self._thread:
            self._thread.join(timeout=10)

    def _loop(self):
        while self._running:
            try:
                self._process_batch()
            except Exception as e:
                logger.warning("EmbeddingQueue error: %s", e)
            time.sleep(self.POLL_INTERVAL)

    def _process_batch(self):
        conn = getattr(self._store, "_conn", None)
        if not conn:
            return

        rows = conn.execute(
            "SELECT entity_id, name, types, properties FROM kr_entities "
            "WHERE memory_embedding IS NULL LIMIT 10"
        ).fetchall()

        for row in rows:
            text_parts = [row["name"]]
            try:
                props = json.loads(row["properties"] or "{}")
                if isinstance(props, dict):
                    desc = props.get("description", "")
                    if desc:
                        text_parts.append(desc)
            except (json.JSONDecodeError, TypeError):
                pass

            text = ". ".join(text_parts)
            embedding = self._client.compute(text)
            if embedding:
                conn.execute(
                    "UPDATE kr_entities SET memory_embedding = ?, "
                    "updated_at = CURRENT_TIMESTAMP WHERE entity_id = ?",
                    (embedding, row["entity_id"]),
                )
                logger.debug("Embedded entity %s", row["entity_id"])
