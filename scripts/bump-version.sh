#!/bin/bash
#
# Bump version of all crates in the workspace.
# Usage: ./scripts/bump-version.sh <new_version>
#

if [ -z "$1" ]; then
    echo "Usage: $0 <new_version>"
    exit 1
fi

NEW_VERSION=$1

# List of crates to bump
CRATES=(
    "package" # Root workspace
    "crates/cli"
    "crates/config"
    "crates/core"
    "crates/dap"
    "crates/loader"
    "crates/svd-ingestor"
    "crates/firmware-stm32f103-uart"
    "crates/firmware-hal-test"
    "crates/firmware-armv6m-ci-fixture"
    "crates/firmware-rv32i-ci-fixture"
)

echo "Bumping version to $NEW_VERSION..."

for crate in "${CRATES[@]}"; do
    if [ "$crate" == "package" ]; then
        TOML_FILE="Cargo.toml"
    else
        TOML_FILE="$crate/Cargo.toml"
    fi

    if [ -f "$TOML_FILE" ]; then
        # Use sed to replace version (simple approach, assumes specific formatting)
        # Matches: version = "..." -> version = "NEW_VERSION"
        # Only does first match if formatted typically at top
        sed -i "s/^version = \".*\"/version = \"$NEW_VERSION\"/" "$TOML_FILE"
        echo "Updated $TOML_FILE"
    else
        echo "Warning: $TOML_FILE not found"
    fi
done

# Update Cargo.lock
echo "Updating Cargo.lock..."
cargo check > /dev/null 2>&1

echo "Done! Please verify changes with 'git diff'."
