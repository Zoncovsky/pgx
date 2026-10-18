# Benchmark methodology

The headline figures come from two harnesses, both Python-3-stdlib and talking
the wire protocol directly (no driver start-up in any number):

- `benchmarks/pgx/pgxbench.py` — startup, idle memory, memory per connection.
- The local-dev suites — clone latency, clone/drop throughput, restart time and
  per-client op rate, sampled while running real CLI commands.

Record, per run: PGX commit, PostgreSQL version, OS, CPU, RAM, filesystem,
storage backend, dataset, repetitions, and p50/p95/p99 where applicable
(`benchmarks/pgx/README.md`). Numbers in `validation-report.md` §7 were observed
on macOS M1 Pro / APFS and are informational, not guarantees; reproduce on
controlled hardware for a baseline. The frozen v0.1 baseline (idle memory,
density, cold/warm mint, ZFS clone) is in `docs/pgx/v0.1-baseline.md`.

Never trade SQL correctness, isolation or durability for latency without
documenting it; the durability setting is explicit (`pgx init --durability`).
