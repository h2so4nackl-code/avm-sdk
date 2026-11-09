# SDK 0.5.1 cross-DApp callback conservation reproducer

Prepared local results from 9 November 2026. This directory publishes an existing reproduction; publication did not re-execute the runtime. Synthetic local funds only. No network transactions, upstream qualification or host fix is claimed.

## Pinned inputs

- SDK release: [avm-dev-e2c6abe2bbea-sdk-961760f8cd79](https://github.com/gosh-sh/avm-sdk/releases/tag/avm-dev-e2c6abe2bbea-sdk-961760f8cd79).
- SDK commit: `961760f8cd794d1bb262b06ecac71efb148802eb`.
- Runtime identity: `e2c6abe2bbea133cab98884a3b032acd9abc8406`.
- Release archive SHA256: `0dd4d5292ab3c8d758bd068d69e803080b286bcecd9c75d285a66adc598a6dd8`.
- Rust/Cargo 1.99.1, wasm32-unknown-unknown; Linux x86-64-v3.
- CLI helper only: PR #33 `examples/staking-rewards/scripts/e2e.py` at `75fda9d45692acaefe52aef0603e4396720e45c8` in `kevinushana/avm-sdk`.
- Guest: the official SDK's unmodified `examples/deploy-callback-repro`. No staking guest or adapted refund guest is built or executed by this reproduction.

## Setup and execution

Use a fresh `/tmp/pr33-runtime` layout. Download and verify the pinned official SDK archive and its internal checksums using that release's documented installation procedure, then unpack its full source/tools bundle at `/tmp/pr33-runtime/avm-sdk-linux-x86_64`. Do not substitute a source-only checkout for the runtime bundle.

Place this published evidence directory at `/tmp/pr33-runtime/evidence`. Obtain the pinned PR source (for its Python CLI helper) at `/tmp/pr33-runtime/work`, for example:

```bash
git clone --no-checkout https://github.com/kevinushana/avm-sdk.git /tmp/pr33-runtime/work
git -C /tmp/pr33-runtime/work checkout --detach 75fda9d45692acaefe52aef0603e4396720e45c8
```

With those prerequisites prepared:

```bash
set -euo pipefail
repro_root=/tmp/pr33-runtime
test ! -e "$repro_root/probe-sdk"
test ! -e "$repro_root/stock-probe"
cp -a "$repro_root/avm-sdk-linux-x86_64" "$repro_root/probe-sdk"
"$repro_root/avm-sdk-linux-x86_64/bin/avm-contract-build" build \
  --manifest-path "$repro_root/probe-sdk/examples/deploy-callback-repro/Cargo.toml"
mkdir -p "$repro_root/work/examples/staking-rewards/contracts/stock-refund-probe/target/avm"
cp "$repro_root/probe-sdk/examples/deploy-callback-repro/target/avm/contract.wasm" \
   "$repro_root/probe-sdk/examples/deploy-callback-repro/target/avm/contract.acab" \
   "$repro_root/probe-sdk/examples/deploy-callback-repro/target/avm/contract.descriptor" \
   "$repro_root/work/examples/staking-rewards/contracts/stock-refund-probe/target/avm/"
python3 "$repro_root/evidence/stock_probe.py"
```

The runner imports `fee_probe.py` beside itself, which imports only the pinned PR helper. The fee probe's own `run()` is not invoked. Both Python files are included unchanged from the prepared validation package. The runner checks all five official guest input files against the pristine bundle. It produces `/tmp/pr33-runtime/stock-results.json`.

These setup commands document the existing layout; they have not been rerun in the publication session. The previously executed command journal and results are in the committed `stock-results.json`.

## Expected observation

Two of eight branches return the conservation error: cross-DApp ordinary calls with a requested callback, on receiver success and guest rejection. The six controls commit. Both error branches retain byte-identical saved state, so matching replay checks their unchanged base state rather than a committed call. The successful internal-deploy control separately returns 999 of the 1,000 SHELL reserve.

Read [STOCK_CONTROL.md](STOCK_CONTROL.md) for the full matrix and exact diagnostic. `stock-results.json` contains all eight branches, source hashes, public synthetic message wires and the CLI journal. It does not include persisted stand files or private test keys.

The internal-deploy refund and the ordinary-call conservation failure are separate observations. This does not demonstrate mainnet loss, and it does not establish which private runtime implementation line is responsible.
