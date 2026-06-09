@echo off
REM Update hermes-agent to latest upstream and re-apply AI-Hel2 patches
cd /d D:\hermes-agent-forAI-Hel2
echo Pulling latest upstream...
git pull origin main
echo Applying AI-Hel2 customizations...
git apply D:\AI-Hel2\scripts\hermes-aihel2.patch
echo Done. Restart the gateway to apply changes.
