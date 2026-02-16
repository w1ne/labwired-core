#!/bin/bash
# LabWired - Batch Hardware Onboarding Utility
# Demonstrates velocity by onboarding dozens of chips in seconds.

SVD_DIR=$1
OUT_DIR=$2

if [ -z "$SVD_DIR" ] || [ -z "$OUT_DIR" ]; then
    echo "Usage: $0 <svd_directory> <output_directory>"
    exit 1
fi

mkdir -p "$OUT_DIR"

echo "🚀 Starting Batch Onboarding from $SVD_DIR..."
count=0

for svd in "$SVD_DIR"/*.svd; do
    [ -e "$svd" ] || continue
    name=$(basename "$svd")
    echo "📦 Processing $name..."
    cargo run --quiet -p svd-ingestor -- --input "$svd" --output-dir "$OUT_DIR"
    count=$((count + 1))
done

echo "✅ Batch completion: $count chips onboarded to $OUT_DIR"
echo "💡 LabWired Architecture: This took seconds. Renode requires manual C# modeling for each."
