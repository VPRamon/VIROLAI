#!/bin/bash

# Quality Assurance Pipeline
# Runs clippy, format check, and tests

set -e

echo "================================"
echo "Quality Assurance Pipeline"
echo "================================"
echo ""

# Run clippy
echo "1. Running clippy..."
if cargo clippy --all-targets -- -D warnings; then
    echo "✓ Clippy passed"
else
    echo "✗ Clippy failed"
    exit 1
fi
echo ""

# Run format check
echo "2. Running format check..."
if cargo fmt --all -- --check; then
    echo "✓ Format check passed"
else
    echo "✗ Format check failed"
    exit 1
fi
echo ""

# Run tests
echo "3. Running tests..."
if cargo test --all-features; then
    echo "✓ Tests passed"
else
    echo "✗ Tests failed"
    exit 1
fi
echo ""

echo "================================"
echo "All checks passed! ✓"
echo "================================"
