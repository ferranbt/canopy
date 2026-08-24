# The runner GitHub ships, in the image canopy runs jobs in.
#
# The image GitHub ships the runner in carries the agent and little else: the tools a step
# expects come from a separate image on the hosted runners. Putting the agent in the same
# image canopy uses means a difference between the two is about the runner rather than
# about what happened to be installed.
FROM ghcr.io/actions/actions-runner:latest AS agent

FROM catthehacker/ubuntu:act-latest
COPY --from=agent /home/runner /home/runner

# The runner refuses to start as root unless told that it is on purpose, and this image,
# like the one a job runs in, is root.
ENV RUNNER_ALLOW_RUNASROOT=1
# What the runner did, on stdout, so a job that went wrong says why without digging in
# `_diag` inside a container that is already gone.
ENV ACTIONS_RUNNER_PRINT_LOG_TO_STDOUT=1
WORKDIR /home/runner
