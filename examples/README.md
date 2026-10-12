# Contract Examples

These contracts are deterministic fixtures for learning the SDK and exercising
the local AVM stand. They use real SDK exports, canonical ABI metadata, state
updates, messages, events, and upgrade intents. They are deliberately small and
are not audited application contracts.

For setup and deployment commands, start with [Getting Started](../docs/getting-started.md).
The available stand commands are listed in the
[Command-Line Reference](../docs/cli-reference.md). Return to the
[SDK overview](../README.md) for installation and release information.

## Example Set

### `contract-a-v1`

A message-isolated sender and the first version in the upgrade pair. It:

- initializes a fixed 72-byte state;
- accepts an external forwarding call;
- sends main value and one extra currency in an internal message;
- requests and validates a callback, then records the callback result;
- exposes a version probe;
- authorizes a package-owner upgrade and applies a deterministic state migration.

Use it with `contract-b` to exercise outbound delivery and callbacks, or with
`contract-a-v2` to exercise publication, selection, and account upgrade.

### `contract-a-v2`

The compatible second version of contract A. It keeps the same state schema and
ABI while changing the version marker. It also exercises contract-initiated
deployment by forwarding a canonical deploy body to an internal destination.
The fixture rejects another code upgrade after version 2.

Use the v1 and v2 pair when validating upgrade authorization, migrated state,
selection changes, and behavior before and after an upgrade.

### `contract-b`

A message-isolated receiver for internal traffic. It:

- validates the transferred main value, extra currency, body, and callback request;
- supports initialization from a contract-initiated deploy;
- emits canonical external and internal events;
- returns callback result data and can drain retained value;
- consumes an internal-event read result;
- supports same-DApp and cross-DApp stand journeys.

Its source contains disabled negative-path hooks selected only by the stand's
failure-flow build. Normal builds do not enable them. Use this contract with
contract A when testing routing, callbacks, refunds, events, rollback, and
cross-DApp behavior.

### `contract-batch-patch`

An account-batch contract with a 65,540-byte state. Initialization uses
`FullReplace`; batch execution accepts one canonical internal message and
returns a `DataPatchList` that changes only the eight-byte counter. The isolated
message entry point intentionally fails.

Use it to validate `AccountBatch`, bounded batch input, large retained state,
and narrow state patches without replacing the complete state image.

### `contract-wallet`

A message-isolated owner wallet. It:

- takes its owner from the deploy: the signing public key for an external
  deploy, or the deploying account for a contract-initiated deploy;
- lets only the owner send main value and SHELL (`currency_id = 2`) to any
  account, with an optional FUEL reserve and an optional callback;
- enforces optional per-window spend limits for main value and SHELL;
- rotates ownership between keys and controlling accounts;
- accepts deposits through `wallet.receive` and keeps value returned by
  callbacks, including failed-delivery callbacks;
- emits canonical `wallet.transferred` and `wallet.owner_changed` events.

`cargo test --manifest-path examples/contract-wallet/Cargo.toml` runs host-side
tests that build canonical ACMI frames and check every output encodes as ACMO.
`examples/contract-wallet/e2e.sh` runs a two-wallet stand journey.

## Build An Example

Install the SDK commands first, then build any example from the repository or
an installed release:

```bash
avm-contract-build build \
  --manifest-path examples/contract-a-v1/Cargo.toml
```

The command writes `contract.wasm`, `contract.acab`, and
`contract.descriptor` under that example's `target/avm` directory. It performs
two builds and rejects non-reproducible output.

For direct source checks:

```bash
cargo +1.93.0 fmt --manifest-path examples/contract-a-v1/Cargo.toml -- --check
cargo +1.93.0 clippy \
  --manifest-path examples/contract-a-v1/Cargo.toml \
  --target wasm32-unknown-unknown -- -D warnings
```

## Recommended Use

Keep the checked-in examples unchanged when using them as release fixtures.
Copy the closest example to a new directory for application work, then change
the package name, state schema, ABI, error codes, authorization values, and
contract logic together. If the copy is moved outside this repository, update
the `avm-contract-sdk` path dependency to its installed or vendored location.

Use the examples as focused references:

- start with `contract-a-v1` for a message-isolated contract;
- add `contract-b` for internal messaging, callbacks, events, and deployment;
- use the v1/v2 pair for upgrades;
- use `contract-batch-patch` only when batch execution and patch output are required.

The fixed identities, values, salts, payloads, and failure hooks are test data.
Replace them before using copied code in an application.
