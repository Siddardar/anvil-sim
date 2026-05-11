# TODO: temp fix (I think use std::ffi::c_char instead of i8 in main.rs)
FROM --platform=linux/amd64 ubuntu:22.04

ENV DEBIAN_FRONTEND=noninteractive
ENV TZ=Etc/UTC

# System dependencies
RUN apt-get update && apt-get install -y \
    build-essential gcc g++ make cmake \
    git curl wget \
    python3 python3-pip \
    autoconf automake autotools-dev libtool \
    bison flex gawk gperf \
    libmpc-dev libmpfr-dev libgmp-dev zlib1g-dev \
    perl ccache libgoogle-perftools-dev numactl perl-doc pkg-config \
    opam bubblewrap libfl-dev sudo help2man \
    && rm -rf /var/lib/apt/lists/*

# Create user
RUN useradd -m -s /bin/bash -G sudo anvil && \
    echo "anvil ALL=(ALL) NOPASSWD:ALL" >> /etc/sudoers

# Install Verilator 5.036
WORKDIR /tmp
RUN git clone https://github.com/verilator/verilator.git && \
    cd verilator && unset VERILATOR_ROOT && git checkout v5.036 && \
    autoconf && ./configure && make -j$(nproc) && make install && \
    cd / && rm -rf /tmp/verilator

USER anvil
ENV HOME=/home/anvil
WORKDIR /home/anvil

# Install OCaml toolchain
RUN opam init --disable-sandboxing -y && \
    opam switch create 5.2.0 ocaml-base-compiler.5.2.0 -y

ENV PATH="/home/anvil/.opam/5.2.0/bin:${PATH}"

# Clone and build Anvil compiler
RUN git clone https://github.com/Siddardar/anvil.git && \
    cd anvil && git checkout simulator

RUN eval $(opam env) && \
    cd anvil && \
    opam install ocamlfind -y && \
    opam install . --deps-only -y && \
    dune build

# Install Rust
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
ENV PATH="/home/anvil/.cargo/bin:${PATH}"

# Build anvil-sim
COPY --chown=anvil:anvil . /home/anvil/anvil-sim/
WORKDIR /home/anvil/anvil-sim
RUN cargo build --release

WORKDIR /home/anvil
ENTRYPOINT ["python3", "/home/anvil/anvil-sim/benchmarks/run_benchmark.py"]
