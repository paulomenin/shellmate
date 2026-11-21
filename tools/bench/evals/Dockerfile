# Build from the repository root. Override with image digests for a release baseline.
ARG RUST_IMAGE=rust:1.99.1-slim-trixie
ARG BASE_IMAGE=debian:trixie-slim
FROM ${RUST_IMAGE} AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates/ crates/
COPY tools/shellmate-testing/ tools/shellmate-testing/
RUN cargo build --release --locked -p shellmate

FROM ${BASE_IMAGE}
# A fixed Debian snapshot controls the package set. Record the resulting image ID in every run.
ARG DEBIAN_SNAPSHOT=20261101T000000Z
RUN rm -f /etc/apt/sources.list.d/debian.sources && \
    printf 'deb [check-valid-until=no] http://snapshot.debian.org/archive/debian/%s/ trixie main\n' "$DEBIAN_SNAPSHOT" > /etc/apt/sources.list && \
    rm -f /etc/dpkg/dpkg.cfg.d/docker && \
    apt-get update && apt-get install -y --no-install-recommends \
      bash zsh fish python3 git tar gzip jq ripgrep findutils coreutils grep sed gawk \
      procps iproute2 lsof net-tools psmisc util-linux systemd ca-certificates man-db groff-base && \
    dpkg-query -W > /opt-packages.txt && rm -rf /var/lib/apt/lists/*
# A made-up command and its manual page: what the tool cases ask about, so the answer can only come from the tool.
COPY tools/bench/evals/image/quokka /usr/local/bin/quokka
COPY tools/bench/evals/image/kiwi /usr/local/bin/kiwi
COPY tools/bench/evals/image/quokka.1 /usr/local/share/man/man1/quokka.1
COPY tools/bench/evals/image/quokka.conf.5 /usr/local/share/man/man5/quokka.conf.5
RUN chmod 755 /usr/local/bin/quokka /usr/local/bin/kiwi && chmod 644 /usr/local/share/man/man1/quokka.1 /usr/local/share/man/man5/quokka.conf.5
COPY --from=build /src/target/release/shellmate /usr/local/bin/shellmate
COPY tools/bench/evals/worker.py /opt/eval/worker.py
USER 65532:65532
WORKDIR /work
CMD ["python3", "-I", "/opt/eval/worker.py"]
