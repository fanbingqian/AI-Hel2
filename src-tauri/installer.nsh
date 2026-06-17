; Copy bundled hermes-agent to app directory during installation
!macro customInstall
  SetOutPath "$INSTDIR"
  File /r /x "__pycache__" /x "*.pyc" /x "venv" /x ".git" "hermes-agent\*.*"

  ; Create unified data directory (portable mode — data lives next to the binary)
  CreateDirectory "$INSTDIR\data"
  CreateDirectory "$INSTDIR\data\wiki"
  CreateDirectory "$INSTDIR\data\hermes"
!macroend
