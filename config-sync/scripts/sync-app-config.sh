#!/usr/bin/env bash
set -euo pipefail

MODE="${1:-}"
APP_CONFIG_PATH="${2:-}"
REPO_ROOT="${3:-}"

usage() {
  cat <<'EOF'
Usage:
  sync-app-config.sh <mode> <app_config_path> <repo_root>

Modes:
  pull  - copy from repo/app-config/config.json -> app_config_path
  push  - copy from app_config_path -> repo/app-config/config.json

Examples:

  # Once: set paths (adjust to your environment)
  export SPC_APP_CONFIG="$HOME/.config/SuperControlPanel/config.json"
  export SPC_CONFIG_REPO="$HOME/config-sync"

  # Before using the app on a new machine
  ./scripts/sync-app-config.sh pull "$SPC_APP_CONFIG" "$SPC_CONFIG_REPO"

  # After changing settings you want to share
  ./scripts/sync-app-config.sh push "$SPC_APP_CONFIG" "$SPC_CONFIG_REPO"
EOF
}

if [[ -z "$MODE" || -z "$APP_CONFIG_PATH" || -z "$REPO_ROOT" ]]; then
  usage
  exit 1
fi

REPO_CONFIG_PATH="$REPO_ROOT/app-config/config.json"

mkdir -p "$(dirname "$APP_CONFIG_PATH")"
mkdir -p "$(dirname "$REPO_CONFIG_PATH")"

case "$MODE" in
  pull)
    if [[ ! -f "$REPO_CONFIG_PATH" ]]; then
      echo "Repo config not found at '$REPO_CONFIG_PATH'. Make sure you've cloned the repo and committed app-config/config.json." >&2
      exit 1
    fi
    cp "$REPO_CONFIG_PATH" "$APP_CONFIG_PATH"
    echo "Pulled config from repo to app path:"
    echo "  $REPO_CONFIG_PATH -> $APP_CONFIG_PATH"
    ;;
  push)
    if [[ ! -f "$APP_CONFIG_PATH" ]]; then
      echo "App config not found at '$APP_CONFIG_PATH'. Launch SuperControlPanel once and save settings, or run in pull mode first." >&2
      exit 1
    fi
    cp "$APP_CONFIG_PATH" "$REPO_CONFIG_PATH"
    echo "Pushed config from app path to repo:"
    echo "  $APP_CONFIG_PATH -> $REPO_CONFIG_PATH"
    ;;
  *)
    echo "Invalid mode: '$MODE'" >&2
    usage
    exit 1
    ;;
esac

