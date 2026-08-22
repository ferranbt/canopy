#!/bin/sh -e
echo "Hello, $1! (from $(cat /etc/alpine-release) in a container)"
echo "workspace holds: $(ls /github/workspace | head -3 | tr '\n' ' ')"
echo "greeted-by=docker" >> "$GITHUB_OUTPUT"
