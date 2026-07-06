#!/bin/bash

set -euo pipefail
IFS=$'\n\t'

# ============================================================
# git-ai uninstaller.
#
# Reverses everything install.sh does:
#   1. Stops the background daemon.
#   2. Removes IDE/agent hooks (via `git-ai uninstall-hooks`).
#   3. Logs out and clears stored credentials.
#   4. Removes the ~/.git-ai/bin/git-ai binary.
#   5. Removes the ~/.local/bin/git-ai symlink.
#   6. Cleans the PATH entries install.sh added to shell configs.
#
# With --purge it also removes the entire ~/.git-ai directory
# (config.json, internal state, skills, daemon lock/sockets, db).
#
# Usage:
#   ./uninstall.sh              # interactive (prompts to confirm)
#   ./uninstall.sh -y           # non-interactive, keep data/config
#   ./uninstall.sh -y --purge   # non-interactive, remove everything
# ============================================================

# ============================================================
# Ensure HOME is set when running via MDMs (e.g. JAMF) or other environments where HOME may be unbound.
# Mirrors the bootstrap in install.sh so this script works in the same contexts.
# ============================================================
INSTALL_USER=""

if [ -z "${HOME:-}" ]; then
    if command -v scutil >/dev/null 2>&1; then
        CURRENT_USER=$( /usr/sbin/scutil <<< "show State:/Users/ConsoleUser" | awk '/Name :/ { print $3 }' || true )
        if [ -n "${CURRENT_USER:-}" ] && [ "$CURRENT_USER" != "loginwindow" ] && [ "$CURRENT_USER" != "_mbsetupuser" ]; then
            export HOME=$( /usr/bin/dscl . -read "/Users/$CURRENT_USER" NFSHomeDirectory | awk '{print $2}' )
            INSTALL_USER="$CURRENT_USER"
        else
            echo "Error: No console user logged in. Deferring uninstall." >&2
            exit 1
        fi
    elif id -un >/dev/null 2>&1; then
        INSTALL_USER="$(id -un)"
        export HOME=$(getent passwd "$INSTALL_USER" | cut -d: -f6)
        if [ -z "$HOME" ]; then
            export HOME="/root"
        fi
    else
        export HOME="/root"
    fi
fi

# Ensure SHELL is set (also may be unbound in JAMF)
if [ -z "${SHELL:-}" ]; then
    if command -v zsh >/dev/null 2>&1; then
        SHELL="$(command -v zsh)"
    elif command -v bash >/dev/null 2>&1; then
        SHELL="$(command -v bash)"
    else
        SHELL="/bin/sh"
    fi
    export SHELL
fi

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[0;33m'
NC='\033[0m' # No Color

# Paths install.sh writes to
INSTALL_DIR="$HOME/.git-ai/bin"
GIT_AI_DIR="$HOME/.git-ai"
GIT_AI_BIN="$INSTALL_DIR/git-ai"
LOCAL_BIN_LINK="$HOME/.local/bin/git-ai"

# Defaults
ASSUME_YES=false
PURGE=false

# Function to print error messages
error() {
    echo -e "${RED}Error: $1${NC}" >&2
    exit 1
}

warn() {
    echo -e "${YELLOW}Warning: $1${NC}" >&2
}

# Function to print success messages
success() {
    echo -e "${GREEN}$1${NC}"
}

info() {
    echo -e "$1"
}

# Print usage and exit
print_help() {
    cat <<'EOF'
git-ai uninstaller

Usage: uninstall.sh [options]

Options:
  -y, --yes     Skip confirmation prompts (non-interactive)
  --purge       Also remove ~/.git-ai entirely (config, state, skills, db)
  -h, --help    Show this help message

Without --purge, the binary, symlink, hooks, and shell PATH entries are
removed, but your ~/.git-ai config and data are preserved.
EOF
}

# Parse command-line arguments
while [ $# -gt 0 ]; do
    case "$1" in
        -y|--yes)
            ASSUME_YES=true
            ;;
        --purge)
            PURGE=true
            ;;
        -h|--help)
            print_help
            exit 0
            ;;
        *)
            error "Unknown option: $1 (use --help for usage)"
            ;;
    esac
    shift
done

# ============================================================
# Propagate GIT_AI_ALLOW_SUPERUSER when running as root so the
# git-ai binary's uninstall-hooks / daemon commands do not refuse.
# ============================================================
if [ "$(id -u)" = "0" ]; then
    export GIT_AI_ALLOW_SUPERUSER=1
fi

# Function to prompt for confirmation. Returns 0 (yes) or 1 (no).
confirm() {
    if [ "$ASSUME_YES" = "true" ]; then
        return 0
    fi
    if [ ! -t 0 ]; then
        echo "Error: stdin is not a terminal. Re-run with -y/--yes to uninstall non-interactively." >&2
        exit 1
    fi
    printf "%s [y/N] " "$1"
    read -r response
    case "$response" in
        [yY][eE][sS]|[yY]) return 0 ;;
        *) return 1 ;;
    esac
}

# Function to detect all shells with existing config files.
# Returns shell configurations in format: "shell_name|config_file" (one per line).
# Mirrors detect_all_shells in install.sh so we clean exactly what was touched.
detect_all_shells() {
    local shells=""

    # Check for bash configs (prefer .bashrc over .bash_profile)
    if [ -f "$HOME/.bashrc" ]; then
        shells="${shells}bash|$HOME/.bashrc\n"
    elif [ -f "$HOME/.bash_profile" ]; then
        shells="${shells}bash|$HOME/.bash_profile\n"
    fi

    # Check for zsh config
    if [ -f "$HOME/.zshrc" ]; then
        shells="${shells}zsh|$HOME/.zshrc\n"
    fi

    # Check for fish config
    if [ -f "$HOME/.config/fish/config.fish" ]; then
        shells="${shells}fish|$HOME/.config/fish/config.fish\n"
    fi

    # Fall back to $SHELL detection (no config files to clean, but kept for parity)
    if [ -z "$shells" ]; then
        local login_shell=""
        if [ -n "${SHELL:-}" ]; then
            login_shell=$(basename "$SHELL")
        fi
        case "$login_shell" in
            fish)
                shells="fish|$HOME/.config/fish/config.fish"
                ;;
            zsh)
                shells="zsh|$HOME/.zshrc"
                ;;
            bash|*)
                shells="bash|$HOME/.bashrc"
                ;;
        esac
    fi

    # Remove trailing newline and output
    printf '%b' "$shells" | sed '/^$/d'
}

# Remove the PATH entries install.sh added to a single shell config.
# install.sh appends a marker comment followed by a PATH/fish_add_path line
# referencing $INSTALL_DIR. We strip any line containing $INSTALL_DIR plus
# the marker comment, in-place so permissions/ownership are preserved.
# Returns 0 if the file was modified, 1 otherwise.
clean_shell_config() {
    local config_file="$1"

    [ -f "$config_file" ] || return 1

    local marker='# Added by git-ai installer on'

    # Only touch the file if it actually contains our additions
    if ! grep -qF "$INSTALL_DIR" "$config_file" 2>/dev/null \
       && ! grep -qF "$marker" "$config_file" 2>/dev/null; then
        return 1
    fi

    # Overwrite in place to preserve the original file's permissions/owner.
    local tmp="${config_file}.tmp.$$"
    if ! grep -v -F -e "$INSTALL_DIR" -e "$marker" "$config_file" > "$tmp" 2>/dev/null; then
        rm -f "$tmp" 2>/dev/null || true
        warn "Failed to clean $config_file"
        return 1
    fi
    cat "$tmp" > "$config_file"
    rm -f "$tmp" 2>/dev/null || true
    success "  ✓ Cleaned git-ai PATH entries from $config_file"
    return 0
}

# ============================================================
# Resolve the git-ai binary to invoke for daemon/hooks/logout.
# Prefer the installed location; fall back to anything on PATH.
# ============================================================
RESOLVED_BIN=""
if [ -x "$GIT_AI_BIN" ]; then
    RESOLVED_BIN="$GIT_AI_BIN"
elif command -v git-ai >/dev/null 2>&1; then
    RESOLVED_BIN="$(command -v git-ai)"
fi

# ============================================================
# Print the plan and confirm.
# ============================================================
echo ""
echo "This will uninstall git-ai:"
echo "  • Stop the git-ai background daemon"
echo "  • Remove IDE/agent hooks (claude code, codex, cursor, etc.) and skills"
echo "  • Log out and clear stored credentials"
echo "  • Remove the binary at $GIT_AI_BIN"
echo "  • Remove the symlink at $LOCAL_BIN_LINK"
echo "  • Clean git-ai PATH entries from your shell configs"
if [ "$PURGE" = "true" ]; then
    echo "  • Remove the entire $GIT_AI_DIR directory (config, state, skills, db)"
else
    echo ""
    echo "  Your $GIT_AI_DIR directory (config, state, skills, db) will be KEPT."
    echo "  Pass --purge to remove it as well."
fi
echo ""

if ! confirm "Proceed with uninstall?"; then
    echo "Aborted."
    exit 0
fi

# ============================================================
# 1. Stop the daemon so it is not holding the lock or rewriting files.
# ============================================================
echo ""
echo "Stopping git-ai daemon..."
if [ -n "$RESOLVED_BIN" ]; then
    if "$RESOLVED_BIN" daemon shutdown >/dev/null 2>&1; then
        success "  ✓ Daemon stopped"
    else
        info "  • Daemon not running (or already stopped)"
    fi
else
    info "  • git-ai binary not found; skipping daemon shutdown"
fi

# ============================================================
# 2. Remove IDE/agent hooks and skills.
#    `uninstall-hooks` defaults to dry-run, so --dry-run=false is required
#    to actually remove anything.
# ============================================================
echo ""
echo "Removing IDE/agent hooks..."
if [ -n "$RESOLVED_BIN" ]; then
    if "$RESOLVED_BIN" uninstall-hooks --dry-run=false >/dev/null 2>&1; then
        success "  ✓ Hooks removed"
    else
        warn "Failed to remove some hooks. You can retry with: git-ai uninstall-hooks --dry-run=false"
    fi
else
    info "  • git-ai binary not found; skipping hook removal"
    info "    (any hooks left behind must be removed manually per-tool)"
fi

# ============================================================
# 3. Log out and clear stored credentials.
# ============================================================
echo ""
echo "Clearing credentials..."
if [ -n "$RESOLVED_BIN" ]; then
    if "$RESOLVED_BIN" logout >/dev/null 2>&1; then
        success "  ✓ Logged out"
    else
        info "  • Not logged in (nothing to clear)"
    fi
else
    info "  • git-ai binary not found; skipping logout"
fi

# ============================================================
# 4. Remove the ~/.local/bin/git-ai symlink.
# ============================================================
echo ""
echo "Removing symlink..."
if [ -L "$LOCAL_BIN_LINK" ] || [ -e "$LOCAL_BIN_LINK" ]; then
    if rm -f "$LOCAL_BIN_LINK" 2>/dev/null; then
        success "  ✓ Removed $LOCAL_BIN_LINK"
    else
        warn "Failed to remove $LOCAL_BIN_LINK"
    fi
else
    info "  • No symlink at $LOCAL_BIN_LINK"
fi

# ============================================================
# 5. Remove the binary (and the now-empty bin directory).
# ============================================================
echo ""
echo "Removing binary..."
if [ -f "$GIT_AI_BIN" ]; then
    if rm -f "$GIT_AI_BIN" 2>/dev/null; then
        success "  ✓ Removed $GIT_AI_BIN"
    else
        warn "Failed to remove $GIT_AI_BIN"
    fi
else
    info "  • No binary at $GIT_AI_BIN"
fi

# Remove the bin directory if it is empty (leave it if other files remain)
if [ -d "$INSTALL_DIR" ] && [ -z "$(ls -A "$INSTALL_DIR" 2>/dev/null)" ]; then
    rmdir "$INSTALL_DIR" 2>/dev/null || true
fi

# ============================================================
# 6. Sweep any lingering process still running the deleted binary.
#    Best-effort; matches the install path so it only targets git-ai.
# ============================================================
if command -v pkill >/dev/null 2>&1; then
    if pgrep -f "\.git-ai/bin/git-ai" >/dev/null 2>&1; then
        pkill -f "\.git-ai/bin/git-ai" 2>/dev/null || true
        info "Terminated lingering git-ai processes"
    fi
fi

# ============================================================
# 7. Optional full purge of ~/.git-ai (config, state, skills, db).
# ============================================================
if [ "$PURGE" = "true" ] && [ -d "$GIT_AI_DIR" ]; then
    echo ""
    if confirm "Remove the entire $GIT_AI_DIR directory (config, state, skills, db)?"; then
        if rm -rf "$GIT_AI_DIR" 2>/dev/null; then
            success "  ✓ Removed $GIT_AI_DIR"
        else
            warn "Failed to remove $GIT_AI_DIR"
        fi
    else
        info "  • Kept $GIT_AI_DIR"
    fi
fi

# ============================================================
# 8. Clean the PATH entries install.sh added to shell configs.
# ============================================================
echo ""
echo "Cleaning shell configurations..."
SHELLS_CLEANED=false
while IFS='|' read -r shell_name config_file; do
    [ -z "$shell_name" ] && continue
    if clean_shell_config "$config_file"; then
        SHELLS_CLEANED=true
    fi
done <<< "$(detect_all_shells)"

if [ "$SHELLS_CLEANED" != "true" ]; then
    info "  • No git-ai PATH entries found in detected shell configs"
fi

# ============================================================
# Done.
# ============================================================
echo ""
success "git-ai has been uninstalled."
echo ""
echo -e "${YELLOW}Close and reopen your terminal and IDE sessions for the changes to take effect.${NC}"

if [ "$PURGE" != "true" ] && [ -d "$GIT_AI_DIR" ]; then
    echo ""
    info "Your data was kept at $GIT_AI_DIR. To remove it completely, re-run with --purge."
fi
