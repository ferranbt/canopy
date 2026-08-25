#!/bin/sh -e
echo "arguments: [$1] [$2] [$3]"
echo "dashed input: [${INPUT_WHO_TO_GREET:-}] [$(printenv 'INPUT_WHO-TO-GREET' || true)]"
echo "spaced input: [${INPUT_TWO_WORDS:-}]"
echo "declared env: [${FROM_THE_ACTION:-}]"
echo "declared from an input: [${NAMED_AFTER_AN_INPUT:-}]"
echo "workspace: [$(basename "$GITHUB_WORKSPACE")]"
echo "state files: [$(dirname "$GITHUB_OUTPUT")]"

echo "said-by=the entrypoint the action asked for" >> "$GITHUB_OUTPUT"
exit "$3"
