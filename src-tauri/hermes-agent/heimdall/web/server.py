"""HEIMDALL Web Console server — FastAPI + uvicorn entry point.

Start with:
    python -m heimdall.web.server
    http://localhost:8765
"""

from __future__ import annotations

import os
import signal
import socket
import sys
from pathlib import Path

import uvicorn


def _free_port(port: int) -> None:
    """Kill any process holding the given port."""
    try:
        import subprocess
        result = subprocess.run(
            ["fuser", f"{port}/tcp"],
            capture_output=True, text=True, timeout=5,
        )
        if result.stdout.strip():
            pids = result.stdout.strip().split()
            for pid in pids:
                try:
                    os.kill(int(pid), signal.SIGTERM)
                except (OSError, ValueError):
                    pass
    except Exception:
        pass


def run(host: str = "0.0.0.0", port: int = 8765):
    """Launch the HEIMDALL web console."""
    _free_port(port)

    config = uvicorn.Config(
        "heimdall.web.api:app",
        host=host,
        port=port,
        log_level="info",
    )
    server = uvicorn.Server(config)
    try:
        server.run()
    except KeyboardInterrupt:
        print("\nServer stopped.")


if __name__ == "__main__":
    run()
