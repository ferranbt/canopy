#!/bin/sh -e
# Every file the runner exchanges state through has to be named, and named where the
# container can reach it rather than where it sits on the host.
for name in GITHUB_ENV GITHUB_OUTPUT GITHUB_PATH GITHUB_STEP_SUMMARY GITHUB_STATE; do
    eval "path=\$$name"
    test -n "$path" || { echo "$name is not set"; exit 1; }
    case "$path" in
        /github/file_commands/*) ;;
        *) echo "$name points at $path, which is not reachable here"; exit 1 ;;
    esac
done

echo "savedby=container" >> "$GITHUB_STATE"
echo "FROM_CONTAINER=yes" >> "$GITHUB_ENV"
echo "wrote-state=yes" >> "$GITHUB_OUTPUT"
