#!/usr/bin/env bash
#
# Run clone detection v2 tests.
#
# Usage:
#   ./run_tests.sh              # Run all v2 tests
#   ./run_tests.sh compile      # Just check if tests compile
#   ./run_tests.sh <filter>     # Run tests matching filter (e.g., "preview")
#
# Prerequisites:
#   - The clones_v2 module must be declared in analysis/mod.rs
#   - The clones_v2_tests module must be declared in analysis/mod.rs
#
# Expected initial state:
#   These tests will FAIL TO COMPILE until the clones_v2 module is created.
#   That is intentional -- TDD workflow: write tests first, then implement.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CRATE_DIR="$(cd "$SCRIPT_DIR/../../../../.." && pwd)"  # tldr-rs-v2-canonical root

cd "$CRATE_DIR"

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

echo "============================================="
echo " Clone Detection v2 Test Runner"
echo "============================================="
echo ""
echo "Crate root: $CRATE_DIR"
echo "Test file:  crates/tldr-core/src/analysis/clones_v2_tests.rs"
echo ""

# Check if the v2 module is declared
if ! grep -q 'pub mod clones_v2' crates/tldr-core/src/analysis/mod.rs 2>/dev/null; then
    echo -e "${YELLOW}WARNING: 'pub mod clones_v2' not found in analysis/mod.rs${NC}"
    echo "The clones_v2 module must be declared for tests to compile."
    echo ""
fi

if ! grep -q 'mod clones_v2_tests' crates/tldr-core/src/analysis/mod.rs 2>/dev/null; then
    echo -e "${YELLOW}WARNING: 'mod clones_v2_tests' not found in analysis/mod.rs${NC}"
    echo "The test module must be declared for tests to compile."
    echo ""
fi

MODE="${1:-all}"

case "$MODE" in
    compile)
        echo "--- Compile check only ---"
        cargo test -p tldr-core --no-run 2>&1 | tail -30
        STATUS=$?
        if [ $STATUS -eq 0 ]; then
            echo -e "\n${GREEN}Compilation successful.${NC}"
        else
            echo -e "\n${RED}Compilation failed.${NC}"
            echo "This is expected if clones_v2 module does not exist yet."
        fi
        exit $STATUS
        ;;

    all)
        echo "--- Running all v2 tests ---"
        echo ""
        # Run all test modules that are part of clones_v2_tests
        cargo test -p tldr-core \
            -- \
            --test-threads=1 \
            -Z unstable-options \
            accurate_line_numbers \
            function_level_extraction \
            no_false_positives \
            preview_populated \
            include_within_file \
            min_lines_enforced \
            sequence_matching \
            json_serialization \
            preserved_behaviors \
            edge_cases \
            2>&1

        STATUS=$?
        echo ""
        if [ $STATUS -eq 0 ]; then
            echo -e "${GREEN}All v2 tests passed!${NC}"
        else
            echo -e "${RED}Some v2 tests failed.${NC}"
        fi
        exit $STATUS
        ;;

    *)
        echo "--- Running tests matching: $MODE ---"
        echo ""
        cargo test -p tldr-core -- --test-threads=1 "$MODE" 2>&1
        STATUS=$?
        echo ""
        if [ $STATUS -eq 0 ]; then
            echo -e "${GREEN}Filtered tests passed!${NC}"
        else
            echo -e "${RED}Some filtered tests failed.${NC}"
        fi
        exit $STATUS
        ;;
esac
