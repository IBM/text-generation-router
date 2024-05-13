## Global Args #################################################################
ARG BASE_UBI_MINIMAL_IMAGE_TAG=9.4-949.1714662671
ARG PROTOC_VERSION=26.0


## Rust builder ################################################################
# Specific debian version so that compatible glibc version is used
FROM rust:1.77-bullseye as rust-builder
ARG PROTOC_VERSION

ENV CARGO_REGISTRIES_CRATES_IO_PROTOCOL=sparse

# Install protoc, no longer included in prost crate
RUN cd /tmp && \
    curl -L -O https://github.com/protocolbuffers/protobuf/releases/download/v${PROTOC_VERSION}/protoc-${PROTOC_VERSION}-linux-x86_64.zip && \
    unzip protoc-*.zip -d /usr/local && rm protoc-*.zip

WORKDIR /usr/src

COPY rust-toolchain.toml rust-toolchain.toml

RUN rustup component add rustfmt


## FMaaS Router builder ########################################################
FROM rust-builder as fmaas-router-builder

COPY proto proto
COPY fmaas-router fmaas-router

WORKDIR /usr/src/fmaas-router

RUN cargo install --path .


## Final FMaaS Router image ####################################################
FROM registry.access.redhat.com/ubi9/ubi-minimal:${BASE_UBI_MINIMAL_IMAGE_TAG} as router-release

WORKDIR /usr/src

COPY --from=fmaas-router-builder /usr/local/cargo/bin/fmaas-router /usr/local/bin/fmaas-router

ENV GRPC_PORT=8033

RUN microdnf install -y --disableplugin=subscription-manager shadow-utils && \
    microdnf clean all --disableplugin=subscription-manager && \
    useradd -u 2000 router -g 0

# Temporary for dev
#RUN chmod -R g+w /usr/src /usr/local/bin

# Run as non-root user by default
USER 2000

EXPOSE ${GRPC_PORT}

CMD fmaas-router
