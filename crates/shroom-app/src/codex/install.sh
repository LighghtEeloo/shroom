#!/bin/sh
set -eu

# Preserve an existing installation, including a broken one that needs an explicit repair.
if command -v codex >/dev/null 2>&1; then
    exec codex --version
fi
if [ -x "$HOME/.local/bin/codex" ]; then
    exec "$HOME/.local/bin/codex" --version
fi

# Download completely before executing, so a failed transfer cannot run a partial installer.
installer=$(mktemp)
trap 'rm -f "$installer"' EXIT
trap 'exit 1' HUP INT TERM
curl --fail --silent --show-error --location --proto '=https' --proto-redir '=https' \
    --connect-timeout 15 --max-time 60 https://chatgpt.com/codex/install.sh -o "$installer"
CODEX_NON_INTERACTIVE=1 CODEX_INSTALL_DIR="$HOME/.local/bin" /bin/sh "$installer"
"$HOME/.local/bin/codex" --version
