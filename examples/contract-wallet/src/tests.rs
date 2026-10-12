//! Host-side tests. They build canonical ACMI frames, run the contract
//! entrypoints, and check that every output encodes as canonical ACMO.

use avm_contract_sdk::decode_message_input;
use avm_contract_sdk::encode_message_output;
use avm_contract_sdk::AccountAddress;
use avm_contract_sdk::Contract;
use avm_contract_sdk::EventKind;
use avm_contract_sdk::MessageOutput;
use avm_contract_sdk::StateUpdate;

use super::*;

const OWNER_KEY: [u8; 32] = [0xa1; 32];
const OTHER_KEY: [u8; 32] = [0xb2; 32];
const BLOCK_TIME: u32 = 1_800_000_000;

fn address(dapp: u8, account: u8) -> AccountAddress {
    AccountAddress { dapp_id: [dapp; 32], account_id: [account; 32] }
}

fn wallet_address() -> AccountAddress {
    address(0x11, 0x22)
}

#[derive(Clone)]
struct Env {
    block_time: u32,
    balance_main: u128,
    balance_shell: u128,
    state: Vec<u8>,
}

impl Env {
    fn funded(state: Vec<u8>) -> Self {
        Self { block_time: BLOCK_TIME, balance_main: 1_000, balance_shell: 1_000, state }
    }
}

fn currency_list(shell: u128) -> Vec<u8> {
    let mut bytes = Vec::new();
    if shell == 0 {
        bytes.extend_from_slice(&0u16.to_le_bytes());
    } else {
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&SHELL_CURRENCY_ID.to_le_bytes());
        bytes.extend_from_slice(&shell.to_le_bytes());
    }
    bytes
}

/// Canonical signed external message (208-byte header + body + signature).
fn external_message(pubkey: [u8; 32], method_id: u64, body: &[u8], deploy: bool) -> Vec<u8> {
    let destination = wallet_address();
    let mut bytes = vec![0; 208];
    bytes[0] = 1;
    bytes[1] = 1 | if deploy { 2 } else { 0 };
    bytes[4..8].copy_from_slice(&(body.len() as u32).to_le_bytes());
    bytes[8..16].copy_from_slice(&method_id.to_le_bytes());
    bytes[48..56].copy_from_slice(&7u64.to_le_bytes());
    bytes[56..60].copy_from_slice(&BLOCK_TIME.to_le_bytes());
    bytes[60..64].copy_from_slice(&(BLOCK_TIME + 3_600).to_le_bytes());
    bytes[64..96].copy_from_slice(&destination.dapp_id);
    bytes[96..128].copy_from_slice(&destination.account_id);
    bytes[128..160].copy_from_slice(&pubkey);
    bytes[160..162].copy_from_slice(&64u16.to_le_bytes());
    bytes.extend_from_slice(body);
    bytes.extend_from_slice(&[0x5a; 64]);
    bytes
}

/// Canonical internal message (192-byte header).
fn internal_message(
    source: AccountAddress,
    method_id: u64,
    body: &[u8],
    value_main: u128,
    flags: u8,
    callback_status: u32,
) -> Vec<u8> {
    let destination = wallet_address();
    let mut bytes = vec![0; 192];
    bytes[1] = flags;
    bytes[4..8].copy_from_slice(&(body.len() as u32).to_le_bytes());
    bytes[8..16].copy_from_slice(&method_id.to_le_bytes());
    bytes[16..32].copy_from_slice(&value_main.to_le_bytes());
    bytes[48..56].copy_from_slice(&3u64.to_le_bytes());
    bytes[64..96].copy_from_slice(&source.dapp_id);
    bytes[96..128].copy_from_slice(&source.account_id);
    bytes[128..160].copy_from_slice(&destination.dapp_id);
    bytes[160..192].copy_from_slice(&destination.account_id);
    if flags & 1 != 0 {
        // Callback header: cause message hash, cause address, status code.
        // The cause address is the original sender, i.e. this wallet.
        bytes.extend_from_slice(&[0x77; 32]);
        bytes.extend_from_slice(&destination.dapp_id);
        bytes.extend_from_slice(&destination.account_id);
        bytes.extend_from_slice(&callback_status.to_le_bytes());
    }
    bytes.extend_from_slice(body);
    bytes
}

fn acmi(env: &Env, caller: Option<AccountAddress>, pubkey: [u8; 32], message: &[u8]) -> Vec<u8> {
    let destination = wallet_address();
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"ACMI");
    bytes.extend_from_slice(&9u64.to_le_bytes());
    bytes.extend_from_slice(&env.block_time.to_le_bytes());
    bytes.extend_from_slice(&11u32.to_le_bytes());
    bytes.extend_from_slice(&[0x12; 32]);
    match caller {
        Some(source) => {
            bytes.push(0);
            bytes.extend_from_slice(&[0; 7]);
            bytes.extend_from_slice(&source.dapp_id);
            bytes.extend_from_slice(&source.account_id);
            bytes.extend_from_slice(&[0; 32]);
        }
        None => {
            bytes.push(1);
            bytes.extend_from_slice(&[0; 7]);
            bytes.extend_from_slice(&[0; 64]);
            bytes.extend_from_slice(&pubkey);
        }
    }
    bytes.extend_from_slice(&destination.dapp_id);
    bytes.extend_from_slice(&destination.account_id);
    bytes.extend_from_slice(&[0x44; 32]);
    bytes.push(0);
    bytes.extend_from_slice(&[0; 7]);
    bytes.extend_from_slice(&env.balance_main.to_le_bytes());
    bytes.extend_from_slice(&currency_list(env.balance_shell));
    bytes.extend_from_slice(&(env.state.len() as u32).to_le_bytes());
    bytes.extend_from_slice(&env.state);
    bytes.extend_from_slice(&(message.len() as u32).to_le_bytes());
    bytes.extend_from_slice(message);
    bytes.extend_from_slice(&0u32.to_le_bytes());
    bytes
}

fn checked(output: MessageOutput) -> MessageOutput {
    encode_message_output(&output).expect("contract output must be canonical ACMO");
    output
}

fn call_external(env: &Env, pubkey: [u8; 32], method_id: u64, body: &[u8]) -> MessageOutput {
    let frame = acmi(env, None, pubkey, &external_message(pubkey, method_id, body, false));
    checked(Wallet::process_message(decode_message_input(&frame).expect("valid ACMI")))
}

fn call_internal(
    env: &Env,
    source: AccountAddress,
    method_id: u64,
    body: &[u8],
    flags: u8,
) -> MessageOutput {
    let frame =
        acmi(env, Some(source), [0; 32], &internal_message(source, method_id, body, 5, flags, 0));
    checked(Wallet::process_message(decode_message_input(&frame).expect("valid ACMI")))
}

fn next_state(output: &MessageOutput) -> Vec<u8> {
    assert_eq!(output.status_code, 0, "expected success");
    match output.state_update.as_ref() {
        Some(StateUpdate::FullReplace(state)) => state.clone(),
        other => panic!("expected full state replace, got {other:?}"),
    }
}

fn deployed_by_key() -> Vec<u8> {
    let env = Env::funded(Vec::new());
    let frame = acmi(&env, None, OWNER_KEY, &external_message(OWNER_KEY, 0, &[], true));
    next_state(&checked(Wallet::init(decode_message_input(&frame).unwrap())))
}

fn transfer(destination: AccountAddress, main: u128, shell: u128, flags: u8) -> Vec<u8> {
    TransferArgs {
        destination,
        value_main: main,
        shell,
        fuel_fee_shells: 2,
        method_id: RECEIVE_METHOD_ID,
        flags,
    }
    .encode()
}

#[test]
fn method_ids_match_blake3_names() {
    // Values precomputed as u64::from_le_bytes(blake3(name)[..8]) — the same
    // derivation the ABI builder uses for examples in this repository.
    let pairs = [
        (TRANSFER_METHOD_ID, 0xb2c9_9efe_4c19_31d1),
        (SET_LIMIT_METHOD_ID, 0x6c1e_10fa_05f0_fdce),
        (SET_OWNER_METHOD_ID, 0x66ab_5367_66ca_d200),
        (RECEIVE_METHOD_ID, 0x545c_8dab_459b_5349),
        (ON_CALLBACK_METHOD_ID, 0xbe8d_8868_828e_0373),
    ];
    for (constant, expected) in pairs {
        assert_eq!(constant, expected);
    }
}

#[test]
fn init_by_external_key_sets_key_owner() {
    let state = WalletState::decode(&deployed_by_key()).unwrap();
    assert_eq!(state, WalletState::new(Owner::PublicKey(OWNER_KEY)));
}

#[test]
fn init_by_contract_sets_account_owner() {
    let parent = address(0x11, 0x99);
    let env = Env::funded(Vec::new());
    let frame = acmi(&env, Some(parent), [0; 32], &internal_message(parent, 0, &[], 0, 2, 0));
    let state = next_state(&checked(Wallet::init(decode_message_input(&frame).unwrap())));
    assert_eq!(WalletState::decode(&state).unwrap().owner, Owner::Account(parent));
}

#[test]
fn state_round_trips_and_rejects_corruption() {
    let mut state = WalletState::new(Owner::Account(address(3, 4)));
    state.limit_main = 10;
    state.window_seconds = 60;
    state.spent_shell = 9;
    state.receive_count = 2;
    let bytes = state.encode();
    assert_eq!(bytes.len(), STATE_BYTES);
    assert_eq!(WalletState::decode(&bytes), Some(state));

    let mut bad = bytes.clone();
    bad[0] = 2;
    assert_eq!(WalletState::decode(&bad), None);
    let mut bad = bytes.clone();
    bad[3] = 1;
    assert_eq!(WalletState::decode(&bad), None);
    let mut bad = bytes.clone();
    bad[104..108].copy_from_slice(&0u32.to_le_bytes());
    assert_eq!(WalletState::decode(&bad), None, "limit without window");
    assert_eq!(WalletState::decode(&bytes[..STATE_BYTES - 1]), None);
}

#[test]
fn owner_transfer_emits_outbound_callback_and_event() {
    let env = Env::funded(deployed_by_key());
    let destination = address(0x11, 0x55);
    let output = call_external(
        &env,
        OWNER_KEY,
        TRANSFER_METHOD_ID,
        &transfer(destination, 40, 30, TRANSFER_FLAG_CALLBACK),
    );
    let state = WalletState::decode(&next_state(&output)).unwrap();
    assert_eq!(state.transfer_count, 1);

    assert_eq!(output.outbound_intents.len(), 1);
    let outbound = &output.outbound_intents[0];
    assert_eq!(outbound.destination, destination);
    assert_eq!(outbound.value_main, 40);
    assert_eq!(outbound.fuel_fee_shells, 2);
    assert_eq!(outbound.method_id, RECEIVE_METHOD_ID);
    assert_eq!(outbound.extra, vec![CurrencyAmount { currency_id: SHELL_CURRENCY_ID, amount: 30 }]);
    assert!(outbound.body.is_empty());

    assert_eq!(output.callback_intents.len(), 1);
    assert_eq!(output.callback_intents[0].handler_method_id, ON_CALLBACK_METHOD_ID);

    assert_eq!(output.event_intents.len(), 1);
    let event = &output.event_intents[0];
    assert_eq!(event.kind, EventKind::External);
    assert_eq!(event.event_class, TRANSFERRED_EVENT_CLASS);
    assert_eq!(event.topics, vec![destination.account_id.to_vec()]);
}

#[test]
fn transfer_without_callback_flag_and_shell_omits_extra() {
    let env = Env::funded(deployed_by_key());
    let output =
        call_external(&env, OWNER_KEY, TRANSFER_METHOD_ID, &transfer(address(1, 2), 1, 0, 0));
    assert_eq!(output.status_code, 0);
    assert!(output.callback_intents.is_empty());
    assert!(output.outbound_intents[0].extra.is_empty());
}

#[test]
fn non_owner_is_rejected_everywhere() {
    let env = Env::funded(deployed_by_key());
    let stranger = address(0x11, 0x66);
    for (method, body) in [
        (TRANSFER_METHOD_ID, transfer(address(1, 2), 1, 0, 0)),
        (SET_LIMIT_METHOD_ID, encode_set_limit_args(0, 0, 0)),
        (SET_OWNER_METHOD_ID, encode_set_owner_args(Owner::PublicKey(OTHER_KEY))),
    ] {
        assert_eq!(call_external(&env, OTHER_KEY, method, &body).status_code, ERR_UNAUTHORIZED);
        assert_eq!(call_internal(&env, stranger, method, &body, 0).status_code, ERR_UNAUTHORIZED);
    }
}

#[test]
fn malformed_transfers_are_rejected() {
    let env = Env::funded(deployed_by_key());
    let call = |body: &[u8]| call_external(&env, OWNER_KEY, TRANSFER_METHOD_ID, body).status_code;

    let good = transfer(address(1, 2), 1, 1, 0);
    assert_eq!(call(&good[..TRANSFER_ARGS_BYTES - 1]), ERR_INVALID_ARGS);
    let mut reserved = good.clone();
    reserved[127] = 1;
    assert_eq!(call(&reserved), ERR_INVALID_ARGS);
    let mut unknown_flag = good.clone();
    unknown_flag[120] = 0x80;
    assert_eq!(call(&unknown_flag), ERR_INVALID_ARGS);
    assert_eq!(call(&transfer(address(1, 0), 1, 0, 0)), ERR_INVALID_ARGS, "zero account");
    assert_eq!(call(&transfer(wallet_address(), 1, 0, 0)), ERR_INVALID_ARGS, "self transfer");
    assert_eq!(call(&transfer(address(1, 2), 0, 0, 0)), ERR_INVALID_ARGS, "empty transfer");
}

#[test]
fn transfers_cannot_exceed_balances() {
    let env = Env { balance_main: 10, balance_shell: 10, ..Env::funded(deployed_by_key()) };
    let call = |body: Vec<u8>| call_external(&env, OWNER_KEY, TRANSFER_METHOD_ID, &body);
    assert_eq!(call(transfer(address(1, 2), 11, 0, 0)).status_code, ERR_INSUFFICIENT_FUNDS);
    // 9 SHELL transferred + 2 SHELL FUEL reserve > 10.
    assert_eq!(call(transfer(address(1, 2), 0, 9, 0)).status_code, ERR_INSUFFICIENT_FUNDS);
    assert_eq!(call(transfer(address(1, 2), 10, 8, 0)).status_code, 0);
    let overflow = TransferArgs {
        fuel_fee_shells: u128::MAX,
        ..TransferArgs::decode(&transfer(address(1, 2), 0, 1, 0)).unwrap()
    };
    assert_eq!(call(overflow.encode()).status_code, ERR_INSUFFICIENT_FUNDS);
}

#[test]
fn spend_limit_is_enforced_per_window_and_resets() {
    let mut env = Env::funded(deployed_by_key());
    env.state = next_state(&call_external(
        &env,
        OWNER_KEY,
        SET_LIMIT_METHOD_ID,
        &encode_set_limit_args(100, 50, 3_600),
    ));

    let destination = address(1, 2);
    env.state = next_state(&call_external(
        &env,
        OWNER_KEY,
        TRANSFER_METHOD_ID,
        &transfer(destination, 60, 30, 0),
    ));
    let over_main =
        call_external(&env, OWNER_KEY, TRANSFER_METHOD_ID, &transfer(destination, 41, 0, 0));
    assert_eq!(over_main.status_code, ERR_LIMIT_EXCEEDED);
    let over_shell =
        call_external(&env, OWNER_KEY, TRANSFER_METHOD_ID, &transfer(destination, 0, 21, 0));
    assert_eq!(over_shell.status_code, ERR_LIMIT_EXCEEDED);
    env.state = next_state(&call_external(
        &env,
        OWNER_KEY,
        TRANSFER_METHOD_ID,
        &transfer(destination, 40, 20, 0),
    ));

    // One second before the window ends: still exhausted.
    env.block_time = BLOCK_TIME + 3_599;
    let early = call_external(&env, OWNER_KEY, TRANSFER_METHOD_ID, &transfer(destination, 1, 0, 0));
    assert_eq!(early.status_code, ERR_LIMIT_EXCEEDED);

    // New window: spending resets.
    env.block_time = BLOCK_TIME + 3_600;
    env.state = next_state(&call_external(
        &env,
        OWNER_KEY,
        TRANSFER_METHOD_ID,
        &transfer(destination, 100, 0, 0),
    ));
    let state = WalletState::decode(&env.state).unwrap();
    assert_eq!(
        (state.window_start, state.spent_main, state.spent_shell),
        (BLOCK_TIME + 3_600, 100, 0)
    );
    assert_eq!(state.transfer_count, 3);
}

#[test]
fn set_limit_validates_window_and_can_clear_limits() {
    let env = Env::funded(deployed_by_key());
    let call = |body: Vec<u8>| call_external(&env, OWNER_KEY, SET_LIMIT_METHOD_ID, &body);
    assert_eq!(call(encode_set_limit_args(1, 0, 0)).status_code, ERR_INVALID_ARGS);
    assert_eq!(call(encode_set_limit_args(0, 0, 60)).status_code, ERR_INVALID_ARGS);
    let mut reserved = encode_set_limit_args(1, 0, 60);
    reserved[39] = 1;
    assert_eq!(call(reserved).status_code, ERR_INVALID_ARGS);
    let cleared = WalletState::decode(&next_state(&call(encode_set_limit_args(0, 0, 0)))).unwrap();
    assert_eq!((cleared.limit_main, cleared.limit_shell, cleared.window_seconds), (0, 0, 0));
}

#[test]
fn owner_rotation_hands_over_control() {
    let mut env = Env::funded(deployed_by_key());
    let output = call_external(
        &env,
        OWNER_KEY,
        SET_OWNER_METHOD_ID,
        &encode_set_owner_args(Owner::PublicKey(OTHER_KEY)),
    );
    assert_eq!(output.event_intents[0].event_class, OWNER_CHANGED_EVENT_CLASS);
    env.state = next_state(&output);

    let body = transfer(address(1, 2), 1, 0, 0);
    assert_eq!(
        call_external(&env, OWNER_KEY, TRANSFER_METHOD_ID, &body).status_code,
        ERR_UNAUTHORIZED
    );
    assert_eq!(call_external(&env, OTHER_KEY, TRANSFER_METHOD_ID, &body).status_code, 0);

    // Hand the wallet to a controlling contract.
    let controller = address(0x11, 0x88);
    env.state = next_state(&call_external(
        &env,
        OTHER_KEY,
        SET_OWNER_METHOD_ID,
        &encode_set_owner_args(Owner::Account(controller)),
    ));
    assert_eq!(
        call_external(&env, OTHER_KEY, TRANSFER_METHOD_ID, &body).status_code,
        ERR_UNAUTHORIZED
    );
    assert_eq!(call_internal(&env, controller, TRANSFER_METHOD_ID, &body, 0).status_code, 0);
}

#[test]
fn set_owner_rejects_invalid_targets() {
    let env = Env::funded(deployed_by_key());
    let call =
        |body: Vec<u8>| call_external(&env, OWNER_KEY, SET_OWNER_METHOD_ID, &body).status_code;
    assert_eq!(
        call(encode_set_owner_args(Owner::PublicKey(OWNER_KEY))),
        ERR_INVALID_ARGS,
        "same owner"
    );
    assert_eq!(
        call(encode_set_owner_args(Owner::Account(wallet_address()))),
        ERR_INVALID_ARGS,
        "self"
    );
    assert_eq!(
        call(encode_set_owner_args(Owner::PublicKey([0; 32]))),
        ERR_INVALID_ARGS,
        "zero key"
    );
    let mut unknown_kind = encode_set_owner_args(Owner::PublicKey(OTHER_KEY));
    unknown_kind[0] = 9;
    assert_eq!(call(unknown_kind), ERR_INVALID_ARGS);
    let mut key_with_tail = encode_set_owner_args(Owner::PublicKey(OTHER_KEY));
    key_with_tail[71] = 1;
    assert_eq!(call(key_with_tail), ERR_INVALID_ARGS);
}

#[test]
fn anyone_can_deposit_but_only_internally_with_empty_body() {
    let mut env = Env::funded(deployed_by_key());
    let sender = address(0x22, 0x33);
    env.state = next_state(&call_internal(&env, sender, RECEIVE_METHOD_ID, &[], 0));
    assert_eq!(WalletState::decode(&env.state).unwrap().receive_count, 1);
    assert_eq!(
        call_internal(&env, sender, RECEIVE_METHOD_ID, b"x", 0).status_code,
        ERR_INVALID_ARGS
    );
    assert_eq!(
        call_external(&env, OTHER_KEY, RECEIVE_METHOD_ID, &[]).status_code,
        ERR_UNKNOWN_METHOD
    );
}

#[test]
fn callbacks_are_accepted_without_state_change() {
    let env = Env::funded(deployed_by_key());
    let destination = address(0x11, 0x55);
    let output = call_internal(&env, destination, ON_CALLBACK_METHOD_ID, &[], 1);
    assert_eq!(next_state(&output), env.state);

    // A failed delivery comes back with a non-zero status and may carry no
    // handler id; the wallet must still accept it so returned value is kept.
    let failed = internal_message(destination, 0, &[], 40, 1, 1101);
    let frame = acmi(&env, Some(destination), [0; 32], &failed);
    let output = checked(Wallet::process_message(decode_message_input(&frame).unwrap()));
    assert_eq!(next_state(&output), env.state);

    // Callbacks never authorize owner actions, even from the owner account.
    let controller = address(0x11, 0x88);
    let mut owned = WalletState::decode(&env.state).unwrap();
    owned.owner = Owner::Account(controller);
    let env = Env { state: owned.encode(), ..env };
    let body = transfer(address(1, 2), 1, 0, 0);
    let output = call_internal(&env, controller, TRANSFER_METHOD_ID, &body, 1);
    assert!(output.outbound_intents.is_empty());
    assert_eq!(next_state(&output), env.state);
}

#[test]
fn unknown_methods_and_bad_state_fail_closed() {
    let env = Env::funded(deployed_by_key());
    assert_eq!(call_external(&env, OWNER_KEY, 42, &[]).status_code, ERR_UNKNOWN_METHOD);
    let corrupt = Env { state: vec![0; STATE_BYTES], ..env };
    assert_eq!(
        call_external(&corrupt, OWNER_KEY, TRANSFER_METHOD_ID, &transfer(address(1, 2), 1, 0, 0))
            .status_code,
        ERR_INVALID_STATE
    );
}
