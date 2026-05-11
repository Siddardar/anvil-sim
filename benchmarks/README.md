# Benchmarks: anvil-sim vs Verilator

Compares wall-clock time for simulating Anvil designs via:
- **Verilator**: `.anvil` -> anvil compiler -> `.sv` -> verilator C++ compile -> run binary
- **anvil-sim**: `.anvil` -> anvil compiler (FFI) -> JSON IR -> event-driven simulation

## Build

From the `anvil-sim/` root directory:

```bash
docker build -t anvil-bench -f benchmarks/Dockerfile .
```

## Run

```bash
docker run --rm anvil-bench <anvil-file> [--max-cycles N] [--runs N]
```

### Arguments

| Argument | Required | Description |
|----------|----------|-------------|
| `<anvil-file>` | Yes | Path to `.anvil` file **inside the container**. Anvil examples are at `/home/anvil/anvil/examples/`. |
| `--max-cycles N` | No | Stop simulation after N cycles. Only needed for designs without `dfinish`. Default: 100. |
| `--runs N` | No | Number of runs for median timing. Default: 5. |

### Examples

```bash
# Self-terminating design (has dfinish)
docker run --rm anvil-bench /home/anvil/anvil/examples/counter2.anvil --runs 5

# Non-terminating design (needs max-cycles)
docker run --rm anvil-bench /home/anvil/anvil/examples/cache.anvil --max-cycles 100 --runs 5
```

### Running raw commands inside the container

```bash
# Interactive shell
docker run --rm -it --entrypoint /bin/bash anvil-bench

# Run anvil-sim directly
docker run --rm --entrypoint /home/anvil/anvil-sim/target/release/anvil-sim anvil-bench <anvil-file> [--max-cycles N]

# List available example files
docker run --rm --entrypoint ls anvil-bench /home/anvil/anvil/examples/
```

## Notes

- The Dockerfile uses `--platform=linux/amd64` as a workaround for a `c_char` type mismatch on arm64 Linux. This causes a harmless platform warning on Apple Silicon Macs.
- Verilator output includes metadata lines (simulation report, `$finish`) that anvil-sim does not produce, so output line counts may differ slightly even when simulation results match.
