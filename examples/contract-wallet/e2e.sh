#!/usr/bin/env bash
# Local-stand journey for contract-wallet: deploy two wallets, set a SHELL
# spend limit, transfer within the limit, and check that over-limit and
# non-owner transfers are rejected. Run from the repository root after
# ./scripts/install.sh.
set -euo pipefail

export PATH="${HOME}/.local/bin:${PATH}"
sdk_home="${AVM_SDK_HOME:-${HOME}/.local/share/avm-sdk}"
profile="${AVM_WASMTIME_PROFILE:-${sdk_home}/current/profiles/wasmtime-release-profile.v1}"
dapp_id="1111111111111111111111111111111111111111111111111111111111111111"
# The identity that signs the deploy becomes the wallet owner.
owner_identity="${OWNER_IDENTITY:-developer}"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
state="${work}/wallet.state"
artifacts="examples/contract-wallet/target/avm"
t=1800000000

stand() { avm-local-stand "$1" --state "$state" --wasmtime-profile "$profile" "${@:2}"; }
times() { echo --block-time "$1" --created-at "$1" --expire-at "$(($1 + 3600))" --fuel-fee-shells 1000; }
le128() { python3 -c "import sys;print(int(sys.argv[1],0).to_bytes(16,'little').hex())" "$1"; }
le64() { python3 -c "import sys;print(int(sys.argv[1],0).to_bytes(8,'little').hex())" "$1"; }
le32() { python3 -c "import sys;print(int(sys.argv[1],0).to_bytes(4,'little').hex())" "$1"; }
# Method id = first eight little-endian bytes of blake3(name).
receive_id=545c8dab459b5349

avm-contract-build build --manifest-path examples/contract-wallet/Cargo.toml >/dev/null
stand init --dapp-id "$dapp_id" >"${work}/init.json"

deploy() {
    local salt="$1" out="${work}/deploy-$1.json" address
    set +e
    stand deploy --wasm "${artifacts}/contract.wasm" --acab "${artifacts}/contract.acab" \
        --descriptor "${artifacts}/contract.descriptor" --salt-hex "$salt" --init-json null \
        $(times "$t") >"$out"
    set -e
    address="$(jq -r .address "$out")"
    stand fund-user --address "$address" --shells 100000 >/dev/null
    stand deploy --wasm "${artifacts}/contract.wasm" --acab "${artifacts}/contract.acab" \
        --descriptor "${artifacts}/contract.descriptor" --salt-hex "$salt" --init-json null \
        $(times "$t") >/dev/null
    echo "$address"
}

wallet_a="$(deploy 77616c6c657461)"
wallet_b="$(deploy 77616c6c657462)"
echo "wallet A: ${wallet_a}"
echo "wallet B: ${wallet_b}"

call() {
    local identity="$1" address="$2" method="$3" args="$4" at="$5"
    stand call --address "$address" --method "$method" --args-json "\"0x${args}\"" \
        --identity "$identity" --value-main 0 --extra-currencies '' $(times "$at")
}

# wallet.set_limit: no main-value limit, 500 SHELL per hour.
call "$owner_identity" "$wallet_a" wallet.set_limit \
    "$(le128 0)$(le128 500)$(le32 3600)00000000" $((t + 10)) >"${work}/limit.json"

transfer_args() {
    local to="$1" shell="$2"
    echo "${to%%:*}${to##*:}$(le128 0)$(le128 "$shell")$(le128 0)$(le64 0x${receive_id})00000000000000"
}

expect_ok() {
    if ! "$@" >"${work}/out.json" 2>&1; then
        cat "${work}/out.json" >&2
        echo "expected success" >&2
        exit 1
    fi
}

expect_status() {
    local code="$1"
    shift
    set +e
    "$@" >"${work}/out.json" 2>&1
    set -e
    if ! grep -q "$code" "${work}/out.json"; then
        cat "${work}/out.json" >&2
        echo "expected status ${code}" >&2
        exit 1
    fi
}

expect_ok call "$owner_identity" "$wallet_a" wallet.transfer "$(transfer_args "$wallet_b" 100)" $((t + 20))
expect_status 1204 call "$owner_identity" "$wallet_a" wallet.transfer \
    "$(transfer_args "$wallet_b" 401)" $((t + 30))
expect_status 1200 call user "$wallet_a" wallet.transfer "$(transfer_args "$wallet_b" 1)" $((t + 40))
expect_ok call "$owner_identity" "$wallet_a" wallet.transfer "$(transfer_args "$wallet_b" 400)" $((t + 50))

stand events >"${work}/events.json"
grep -q wallet.transferred "${work}/events.json"
stand replay >/dev/null
echo "contract-wallet stand journey passed"
