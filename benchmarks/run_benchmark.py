#!/usr/bin/env python3
"""
Benchmark: anvil-sim vs Verilator for Anvil designs.

Measures wall-clock time for:
  - Verilator path: anvil compile → verilator C++ build → run binary
  - anvil-sim path: anvil-sim (FFI compile + simulate)

Usage:
  python3 run_benchmark.py [--max-cycles N] [--runs N] [--anvil-file PATH]
"""

import argparse
import os
import subprocess
import shutil
import tempfile
import time

SCRIPT_DIR = os.path.dirname(os.path.abspath(__file__))
ANVIL_SIM_ROOT = os.path.dirname(SCRIPT_DIR)
ANVIL_SIM_BIN = os.path.join(ANVIL_SIM_ROOT, "target", "release", "anvil-sim")
ANVIL_EXAMPLES = os.path.join(ANVIL_SIM_ROOT, "..", "anvil", "examples")

DEFAULT_MAX_CYCLES = 100
DEFAULT_RUNS = 5


def find_anvil_compiler():
    """Find the anvil compiler binary."""
    result = subprocess.run(["which", "anvil"], capture_output=True, text=True)
    if result.returncode == 0:
        return result.stdout.strip()
    # Try dune exec path
    anvil_dir = os.path.join(ANVIL_SIM_ROOT, "..", "anvil")
    if os.path.isdir(anvil_dir):
        return f"dune exec --root {anvil_dir} anvil --"
    return None


def time_command(cmd, cwd=None, timeout=300):
    """Run a command and return (wall_seconds, stdout, stderr, returncode)."""
    start = time.perf_counter()
    try:
        result = subprocess.run(
            cmd, shell=isinstance(cmd, str), capture_output=True,
            text=True, cwd=cwd, timeout=timeout
        )
        elapsed = time.perf_counter() - start
        return elapsed, result.stdout, result.stderr, result.returncode
    except subprocess.TimeoutExpired:
        return timeout, "", "TIMEOUT", -1
    except FileNotFoundError as e:
        return 0, "", f"NOT FOUND: {e}", -1


def run_verilator_path(anvil_file, max_cycles, work_dir):
    """Run the Verilator path and return timing dict."""
    module_name = os.path.splitext(os.path.basename(anvil_file))[0]
    sv_file = os.path.join(work_dir, f"{module_name}.sv")
    driver_file = os.path.join(work_dir, f"{module_name}_driver.cpp")

    # Step 1: Anvil compile to SV
    anvil_cmd = f"dune exec --root {os.path.join(ANVIL_SIM_ROOT, '..', 'anvil')} anvil -- {anvil_file}"
    t_anvil, sv_out, stderr, rc = time_command(anvil_cmd)
    if rc != 0:
        return {"error": f"anvil compile failed: {stderr}"}
    with open(sv_file, "w") as f:
        f.write(sv_out)

    # Step 2: Create sim driver
    sim_main = os.path.join(ANVIL_EXAMPLES, "sim_main.cpp")
    with open(sim_main) as f:
        driver_src = f.read().replace("Vtop", f"V{module_name}")
    with open(driver_file, "w") as f:
        f.write(driver_src)

    # Step 3: Verilator C++ compile
    if shutil.which("verilator") is None:
        return {"error": "verilator not found in PATH"}
    veri_cmd = [
        "verilator", "--cc", "--exe", "--build",
        "--top", module_name, "-j", "4",
        "-Wno-fatal",
        sv_file, driver_file
    ]
    t_veri_compile, _, stderr, rc = time_command(veri_cmd, cwd=work_dir)
    if rc != 0:
        return {"error": f"verilator compile failed: {stderr[:200]}"}

    # Step 4: Run simulation
    exe = os.path.join(work_dir, "obj_dir", f"V{module_name}")
    # Verilator timeout is in clock half-cycles; multiply by 2 for equivalent cycles
    t_sim, stdout, stderr, rc = time_command([exe, str(max_cycles * 2)])

    return {
        "anvil_compile": t_anvil,
        "verilator_compile": t_veri_compile,
        "verilator_sim": t_sim,
        "total": t_anvil + t_veri_compile + t_sim,
        "output_lines": len(stdout.strip().split("\n")) if stdout.strip() else 0,
    }


def run_anvil_sim_path(anvil_file, max_cycles):
    """Run the anvil-sim path and return timing dict."""
    cmd = [ANVIL_SIM_BIN, anvil_file, "--max-cycles", str(max_cycles)]
    t_total, stdout, stderr, rc = time_command(cmd)
    if rc != 0:
        return {"error": f"anvil-sim failed: {stderr[:200]}"}
    return {
        "total": t_total,
        "output_lines": len(stdout.strip().split("\n")) if stdout.strip() else 0,
    }


def median(values):
    s = sorted(values)
    n = len(s)
    if n % 2 == 1:
        return s[n // 2]
    return (s[n // 2 - 1] + s[n // 2]) / 2


def run_benchmark(anvil_file, max_cycles, num_runs):
    """Run both paths multiple times and report results."""
    module_name = os.path.splitext(os.path.basename(anvil_file))[0]
    print(f"\n{'='*60}")
    print(f"Benchmark: {module_name}")
    print(f"Max cycles: {max_cycles} | Runs: {num_runs}")
    print(f"{'='*60}")

    # Verilator path (build once, time sim separately)
    work_dir = tempfile.mkdtemp(prefix=f"anvil_bench_{module_name}_")
    print(f"\n[Verilator] Building in {work_dir}...")

    veri_result = run_verilator_path(anvil_file, max_cycles, work_dir)
    if "error" in veri_result:
        print(f"  ERROR: {veri_result['error']}")
        veri_summary = None
    else:
        print(f"  Anvil->SV:         {veri_result['anvil_compile']:.3f}s")
        print(f"  Verilator compile: {veri_result['verilator_compile']:.3f}s")
        print(f"  Verilator sim:     {veri_result['verilator_sim']:.3f}s")
        print(f"  Total:             {veri_result['total']:.3f}s")
        print(f"  Output lines:      {veri_result['output_lines']}")

        # Re-run just the simulation for median timing
        exe = os.path.join(work_dir, "obj_dir", f"V{module_name}")
        sim_times = []
        for _ in range(num_runs):
            t, _, _, _ = time_command([exe, str(max_cycles * 2)])
            sim_times.append(t)
        veri_summary = {
            "anvil_compile": veri_result["anvil_compile"],
            "verilator_compile": veri_result["verilator_compile"],
            "sim_median": median(sim_times),
            "total": veri_result["anvil_compile"] + veri_result["verilator_compile"] + median(sim_times),
        }

    # anvil-sim path
    print(f"\n[anvil-sim] Running {num_runs} times...")
    sim_times = []
    sim_result = None
    for i in range(num_runs):
        result = run_anvil_sim_path(anvil_file, max_cycles)
        if "error" in result:
            print(f"  ERROR: {result['error']}")
            break
        sim_times.append(result["total"])
        sim_result = result

    if sim_times:
        sim_median = median(sim_times)
        print(f"  Median time:  {sim_median:.3f}s")
        print(f"  Output lines: {sim_result['output_lines']}")
    else:
        sim_median = None

    # Summary
    print(f"\n{'─'*60}")
    print(f"RESULTS: {module_name} ({max_cycles} cycles)")
    print(f"{'─'*60}")
    if veri_summary:
        print(f"  Verilator:")
        print(f"    Anvil compile:     {veri_summary['anvil_compile']:.3f}s")
        print(f"    Verilator compile: {veri_summary['verilator_compile']:.3f}s")
        print(f"    Simulation:        {veri_summary['sim_median']:.3f}s")
        print(f"    Total:             {veri_summary['total']:.3f}s")
    if sim_median is not None:
        print(f"  anvil-sim:")
        print(f"    Total:             {sim_median:.3f}s")
    if veri_summary and sim_median is not None:
        speedup = veri_summary["total"] / sim_median if sim_median > 0 else float("inf")
        print(f"  Speedup (anvil-sim vs Verilator total): {speedup:.1f}x")

    # Cleanup
    shutil.rmtree(work_dir, ignore_errors=True)


def main():
    parser = argparse.ArgumentParser(description="Benchmark anvil-sim vs Verilator")
    parser.add_argument("anvil_file", help="Anvil source file to benchmark")
    parser.add_argument("--max-cycles", type=int, default=DEFAULT_MAX_CYCLES)
    parser.add_argument("--runs", type=int, default=DEFAULT_RUNS)
    args = parser.parse_args()

    anvil_file = os.path.abspath(args.anvil_file)
    if not anvil_file.endswith(".anvil"):
        print(f"Error: expected .anvil file, got {anvil_file}")
        return 1
    if not os.path.isfile(anvil_file):
        print(f"Error: {anvil_file} not found")
        return 1

    if not os.path.isfile(ANVIL_SIM_BIN):
        print(f"Error: anvil-sim binary not found at {ANVIL_SIM_BIN}")
        print("Run: cargo build --release")
        return 1

    run_benchmark(anvil_file, args.max_cycles, args.runs)
    return 0


if __name__ == "__main__":
    exit(main())
