#!/bin/sh -e
# What the main entrypoint saved reaches its post step as `STATE_<name>`.
test "$STATE_savedby" = "container" || {
    echo "the post step did not see the state the main step saved"
    exit 1
}
echo "the post step read the state the main step saved"
