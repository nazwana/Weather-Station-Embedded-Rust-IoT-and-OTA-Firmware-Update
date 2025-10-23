#!/bin/bash
set -euo pipefail

TARGET="xtensa-esp32s3-espidf"
PROJECT_NAME="week-1"
BUILD_TYPE="release"

ELF_PATH="target/${TARGET}/${BUILD_TYPE}/${PROJECT_NAME}"
OUT_DIR="firmware"
TIMESTAMP=$(date +%Y%m%d-%H%M%S)
BIN_NAME="${PROJECT_NAME}-${TIMESTAMP}.bin"
BIN_PATH="${OUT_DIR}/${BIN_NAME}"

mkdir -p "${OUT_DIR}"

echo "🔨 Building project..."
cargo build --release

echo "📦 Generating BIN file for OTA..."
# Note: order: <ELF> <BIN>, and --chip must be specified
espflash save-image --chip esp32s3 "${ELF_PATH}" "${BIN_PATH}"

echo "🎉 Firmware siap: ${BIN_PATH}"
echo "Upload ${BIN_PATH} to ThingsBoard OTA packages (choose Device Profile: Weather Station)."
