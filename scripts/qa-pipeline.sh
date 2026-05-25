#!/bin/bash

# Quality Assurance Pipeline
# Runs clippy, format check, and tests
#
# Pass `--with-webapp` to also run the webapp/TSI QA pipeline
# (`webapp/qa-pipeline.sh`).

set -e

WITH_WEBAPP=0
for arg in "$@"; do
    case "${arg}" in
        --with-webapp) WITH_WEBAPP=1 ;;
        *)
            echo "Unknown argument: ${arg}" >&2
            exit 2
            ;;
    esac
done

echo "================================"
echo "Quality Assurance Pipeline"
echo "================================"
echo ""

echo "1. Running clippy..."
if cargo clippy --workspace --exclude tsi-rust --all-targets -- -D warnings; then
    echo "✓ Clippy passed"
else
    echo "✗ Clippy failed"
    exit 1
fi
echo ""

echo "2. Running format check..."
if cargo fmt --all -- --check; then
    echo "✓ Format check passed"
else
    echo "✗ Format check failed"
    exit 1
fi
echo ""

echo "3. Running tests..."
if cargo test --workspace --exclude tsi-rust --all-features; then
    echo "✓ Tests passed"
else
    echo "✗ Tests failed"
    exit 1
fi
echo ""

echo "================================"
echo "All checks passed! ✓"
echo "================================"

if [[ "${WITH_WEBAPP}" == "1" ]]; then
    echo ""
    echo "================================"
    echo "Webapp QA Pipeline"
    echo "================================"
    "$(dirname "$0")/../webapp/qa-pipeline.sh"
fi
