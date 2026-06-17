#!/bin/bash
# HEIMDALL Web Console — startup script
# Usage: bash start_heimdall.sh

cd "$(dirname "$0")"

echo "Starting HEIMDALL Web Console..."
python3 -m uvicorn heimdall.web.api:app --host 0.0.0.0 --port 8765

echo "Server stopped."
