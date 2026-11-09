# Additional qualification: unmodified official guest reproduces the error

Date: 2026-11-09. This supplements REPORT.md. No GitHub changes or network transactions were performed.

## Why this control matters

The first cross-DApp matrix used an adapted probe contract. This control removes that dependency: the source, manifest, Cargo manifest/lock and resource file of the official SDK 0.5.1 `examples/deploy-callback-repro` are byte-identical to the verified distribution. The current guest SDK dependency is also unchanged. The official builder built this source with its built-in reproducibility checks.

Builder artifact digests:

- WASM: `f938365231294104ff0fbddd80308b1f8362c74e0578712a78bdd59670b09911`
- ACAB: `3d6af2a69e1401589aa4909de2e3aac8231f703e92c37f08e9f6a652e3ee72f0`
- Descriptor: `e47be5c351074cde5b4f16be15b0af2a415b266d68127728c3de0a421f311666`

The stock guest always proposes a 1,000-SHELL outbound reserve and transfers 17 SHELL as independent value. Its callback records all incoming SHELL, so on failed delivery that recorded amount can include returned principal. In particular, 17 recorded in the same-DApp failure case below is returned transferred value, not an unused-fuel refund.

## Procedure

The runner uses the same public-CLI helper and independent retained-state decoder as the earlier probes. It deploys a parent and child in the initialized DApp, captures the CLI-generated selected-version deploy envelope, then forwards it unchanged to an address in a second DApp whose account ID has the same derivation. This creates the remote child through an actual internal deployment, without editing canonical account images or transcript bytes. The remote address is prefunded with the stand facility.

That internal deployment succeeds, and the stock parent's callback records 999 SHELL out of its proposed 1,000 reserve. The child state records an internal init caller. Copies of the resulting valid saved state start eight independent call branches at the same logical time. Successful receiver calls use the existing `repro.ping`; rejected receiver calls use unknown method 123, which the unmodified guest rejects with 1505.

## Results

| Route | Target behavior | Requested callback | Outer result | Parent SHELL debit | Retained state |
|---|---|---|---|---:|---|
| same DApp | success | no | ok | 118 | commits |
| same DApp | success | yes | ok | 119 | commits |
| same DApp | rejection | no | ok | 101 | commits failure notification |
| same DApp | rejection | yes | ok | 102 | commits requested callback |
| cross DApp | success | no | ok | 1,119 | commits |
| cross DApp | success | yes | error | 0 | byte-identical to before |
| cross DApp | rejection | no | ok | 1,102 | commits failure notification |
| cross DApp | rejection | yes | error | 0 | byte-identical to before |

The error in both failing branches is exactly:

```
call rejected canonical operation: local call fees do not conserve canonical debits: debited=0, expected_debits=0, credited=999, burned=1, expected_burns=1000, pre_execution_burns=0, callback_fee=0, prepaid_internal=1000
```

All eight resulting saved states replay with matching account/registry roots, cumulative fees and accepted-input counts. For the two errors this validates the unchanged base state; it does not mean the attempted calls committed. In the committing same-DApp branches, the forward reserve is normalized to zero. In the committing cross-DApp branches without a requested callback, it remains 1,000.

The controls show that this reproducer is independent of staking implementation details and of the adapted probe guest. They establish the behavior of this official local-stand bundle only. They do not identify the exact private host-source line, establish a mainnet bug, or constitute a host fix.

## Reproduction additions

Build the verified bundle's original `examples/deploy-callback-repro` in the existing writable `probe-sdk` copy without changing any of its files. Copy its `target/avm` triple to `work/examples/staking-rewards/contracts/stock-refund-probe/target/avm`, then run `stock_probe.py` from the existing `/tmp/pr33-runtime` layout. The runner requires a fresh `stock-probe` state directory. `stock-results.json` includes input SHA256 identities, the internal-deploy control, all eight branches, signed canonical wire records and the public-CLI journal. The official fixture source is already available at the pinned release; it need not be replaced with the adapted fixture.

Source: https://github.com/gosh-sh/avm-sdk/blob/961760f8cd794d1bb262b06ecac71efb148802eb/examples/deploy-callback-repro/src/lib.rs
