; Copy bundled hermes-agent to app directory during installation
!macro customInstall
  SetOutPath "$INSTDIR"
  File /r /x "__pycache__" /x "*.pyc" /x "venv" /x ".git" "hermes-agent\*.*"
!macroend
