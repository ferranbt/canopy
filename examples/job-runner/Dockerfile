FROM rust:1-bookworm AS build
WORKDIR /src
COPY . .
RUN cargo build --release -p job-runner

FROM catthehacker/ubuntu:act-latest
RUN curl -fsSL "https://awscli.amazonaws.com/awscli-exe-linux-$(uname -m).zip" -o /tmp/awscli.zip \
    && unzip -q /tmp/awscli.zip -d /tmp \
    && /tmp/aws/install \
    && rm -rf /tmp/awscli.zip /tmp/aws
COPY --from=build /src/target/release/job-runner /usr/local/bin/job-runner
WORKDIR /work
ENTRYPOINT ["job-runner"]
