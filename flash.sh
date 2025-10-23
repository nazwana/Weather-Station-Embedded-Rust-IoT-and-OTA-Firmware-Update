#!/bin/bash
PORT="/dev/ttyACM0"
TARGET="xtensa-esp32s3-espidf"
PROJECT_NAME="week-1"
BUILD_PATH="target/$TARGET/debug/$PROJECT_NAME"
PARTITION_TABLE="./partitions.csv"

echo "🔨 Building project..."
cargo build

if [ $? -eq 0 ]; then
    echo "✅ Build sukses! Melakukan flash ke board..."
    espflash flash --partition-table "$PARTITION_TABLE" target/xtensa-esp32s3-espidf/debug/week-1 --monitor
else
    echo "❌ Build gagal! Periksa kode Anda."
    exit 1
fi