#!/usr/bin/env bash
set -e

# HalpTask Installer Script
# ⚠️ WARNING: Never trust install scripts blindly! Always inspect the source code before executing.
# Usage: curl -fsSL https://raw.githubusercontent.com/halpworld/halptask/main/scripts/install.sh | bash

REPO="halpworld/halptask"
BINARY_NAME="halptask"

echo "🚀 Installing HalpTask..."

# 1. Detect OS
OS_RAW=$(uname -s)
BIN_EXT=""
DEST_BINARY_NAME="${BINARY_NAME}"

case "${OS_RAW}" in
    Linux*)
        OS="Linux"
        ;;
    Darwin*)
        OS="Darwin"
        ;;
    MINGW*|MSYS*|CYGWIN*|Windows*)
        OS="Windows"
        BIN_EXT=".exe"
        DEST_BINARY_NAME="${BINARY_NAME}.exe"
        ;;
    *)
        echo "❌ Unsupported operating system: ${OS_RAW}. HalpTask install script supports Linux, macOS, and Windows."
        exit 1
        ;;
esac

# 2. Detect Architecture
ARCH_RAW=$(uname -m)
case "${ARCH_RAW}" in
    x86_64|amd64)   ARCH="x86_64";;
    arm64|aarch64)  ARCH="arm64";;
    *)              echo "❌ Unsupported architecture: ${ARCH_RAW}. Supported architectures: x86_64, arm64."; exit 1;;
esac

# 3. Determine Version & Download URL
if [ -z "${VERSION}" ]; then
    echo "🔍 Finding latest release..."
    LATEST_RELEASE=$(curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" 2>/dev/null || true)
    if [ -n "${LATEST_RELEASE}" ]; then
        TAG=$(echo "${LATEST_RELEASE}" | grep '"tag_name":' | sed -E 's/.*"([^"]+)".*/\1/')
    fi
    if [ -z "${TAG}" ]; then
        echo "⚠️ Could not automatically detect latest tag from GitHub API, falling back to latest release download..."
        DOWNLOAD_URL_PRIMARY="https://github.com/${REPO}/releases/latest/download/${BINARY_NAME}_${OS}_${ARCH}${BIN_EXT}"
        DOWNLOAD_URL_FALLBACK="https://github.com/${REPO}/releases/latest/download/${BINARY_NAME}_${OS}_${ARCH}"
    else
        DOWNLOAD_URL_PRIMARY="https://github.com/${REPO}/releases/download/${TAG}/${BINARY_NAME}_${OS}_${ARCH}${BIN_EXT}"
        DOWNLOAD_URL_FALLBACK="https://github.com/${REPO}/releases/download/${TAG}/${BINARY_NAME}_${OS}_${ARCH}"
    fi
else
    TAG="${VERSION}"
    DOWNLOAD_URL_PRIMARY="https://github.com/${REPO}/releases/download/${TAG}/${BINARY_NAME}_${OS}_${ARCH}${BIN_EXT}"
    DOWNLOAD_URL_FALLBACK="https://github.com/${REPO}/releases/download/${TAG}/${BINARY_NAME}_${OS}_${ARCH}"
fi

# 4. Determine Destination Directory
INSTALL_DIR="/usr/local/bin"
USE_SUDO=false

if [ "${OS}" = "Windows" ]; then
    if [ -d "$HOME/bin" ]; then
        INSTALL_DIR="$HOME/bin"
    elif [ -d "$HOME/.local/bin" ]; then
        INSTALL_DIR="$HOME/.local/bin"
    else
        INSTALL_DIR="$HOME/bin"
        mkdir -p "${INSTALL_DIR}"
    fi
else
    if [ ! -w "${INSTALL_DIR}" ]; then
        if [ -d "$HOME/.local/bin" ]; then
            INSTALL_DIR="$HOME/.local/bin"
        elif command -v sudo >/dev/null 2>&1; then
            USE_SUDO=true
        else
            INSTALL_DIR="$HOME/bin"
            mkdir -p "${INSTALL_DIR}"
        fi
    fi
fi

# 5. Download Binary
TMP_DIR=$(mktemp -d)
trap 'rm -rf "${TMP_DIR}"' EXIT

TMP_BINARY="${TMP_DIR}/${DEST_BINARY_NAME}"
echo "📥 Downloading binary for ${OS}/${ARCH}..."
if ! curl -fsSL "${DOWNLOAD_URL_PRIMARY}" -o "${TMP_BINARY}" 2>/dev/null; then
    if ! curl -fsSL "${DOWNLOAD_URL_FALLBACK}" -o "${TMP_BINARY}"; then
        echo "❌ Download failed! Could not download from ${DOWNLOAD_URL_PRIMARY} or ${DOWNLOAD_URL_FALLBACK}"
        exit 1
    fi
fi

chmod +x "${TMP_BINARY}"

# 6. Install Binary
TARGET_PATH="${INSTALL_DIR}/${DEST_BINARY_NAME}"
echo "📦 Installing to ${TARGET_PATH}..."
if [ "${USE_SUDO}" = true ]; then
    sudo mv "${TMP_BINARY}" "${TARGET_PATH}"
else
    mv "${TMP_BINARY}" "${TARGET_PATH}"
fi

echo "✅ HalpTask successfully installed!"
echo "🎉 Run '${DEST_BINARY_NAME}' to get started."
