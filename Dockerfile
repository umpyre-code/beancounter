FROM gcr.io/umpyre/github.com/umpyre-code/rust:latest

ARG SSH_KEY
ARG SCCACHE_KEY

WORKDIR /app

ADD out/* /usr/bin/
ADD entrypoint.sh /app

ENV RUST_LOG=info
ENV RUST_BACKTRACE=full

ENTRYPOINT [ "/app/entrypoint.sh" ]
