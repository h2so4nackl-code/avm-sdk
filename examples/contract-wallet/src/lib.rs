#![forbid(unsafe_code)]

//! Owner-controlled AVM wallet with optional per-window spend limits.
//!
//! The wallet is owned either by an external public key (signed external
//! deploy) or by another account (contract-initiated deploy). Only the owner
//! can move funds, change spend limits, or hand the wallet to a new owner.
//! Anyone can send value to the wallet through `wallet.receive`.

use avm_contract_sdk::encode_event_fields;
use avm_contract_sdk::AccountAddress;
use avm_contract_sdk::AvmAmount;
use avm_contract_sdk::CallbackIntent;
use avm_contract_sdk::Caller;
use avm_contract_sdk::Contract;
use avm_contract_sdk::CurrencyAmount;
use avm_contract_sdk::EventField;
use avm_contract_sdk::EventIntent;
use avm_contract_sdk::EventKind;
use avm_contract_sdk::EventValue;
use avm_contract_sdk::MessageInput;
use avm_contract_sdk::MessageOutput;
use avm_contract_sdk::OutboundIntent;
use avm_contract_sdk::StateUpdate;

/// Method ids are the first eight little-endian bytes of `blake3(name)`.
pub const TRANSFER_METHOD_ID: u64 = 0xb2c9_9efe_4c19_31d1;
pub const SET_LIMIT_METHOD_ID: u64 = 0x6c1e_10fa_05f0_fdce;
pub const SET_OWNER_METHOD_ID: u64 = 0x66ab_5367_66ca_d200;
pub const RECEIVE_METHOD_ID: u64 = 0x545c_8dab_459b_5349;
pub const ON_CALLBACK_METHOD_ID: u64 = 0xbe8d_8868_828e_0373;
pub const TRANSFERRED_EVENT_CLASS: u64 = 0x60a4_d6e6_f3a9_7a2f;
pub const OWNER_CHANGED_EVENT_CLASS: u64 = 0xb23f_e234_d241_22fe;

/// SHELL is ordinary extra currency 2.
pub const SHELL_CURRENCY_ID: u32 = 2;

pub const ERR_UNAUTHORIZED: u32 = 1200;
pub const ERR_INVALID_ARGS: u32 = 1201;
pub const ERR_INVALID_STATE: u32 = 1202;
pub const ERR_INSUFFICIENT_FUNDS: u32 = 1203;
pub const ERR_LIMIT_EXCEEDED: u32 = 1204;
pub const ERR_UNKNOWN_METHOD: u32 = 1205;
pub const ERR_EVENT_ENCODING: u32 = 1206;
pub const ERR_INVALID_INIT: u32 = 1207;

pub const STATE_VERSION: u8 = 1;
pub const OWNER_KIND_PUBKEY: u8 = 1;
pub const OWNER_KIND_ACCOUNT: u8 = 2;

pub const STATE_BYTES: usize = 160;
pub const TRANSFER_ARGS_BYTES: usize = 128;
pub const SET_LIMIT_ARGS_BYTES: usize = 40;
pub const SET_OWNER_ARGS_BYTES: usize = 72;

/// `wallet.transfer` flag: request a callback to `wallet.on_callback`.
pub const TRANSFER_FLAG_CALLBACK: u8 = 1;

// State layout (160 bytes, little-endian integers):
//   0        version (1)
//   1        owner kind (1 = public key, 2 = account)
//   2..8     reserved, zero
//   8..72    owner: public key + 32 zero bytes, or dapp id + account id
//   72..88   main-value limit per window (0 = unlimited)
//   88..104  SHELL limit per window (0 = unlimited)
//   104..108 window length in seconds
//   108..112 current window start (block time)
//   112..128 main value spent in the current window
//   128..144 SHELL spent in the current window
//   144..152 outgoing transfer count
//   152..160 incoming receive count

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Owner {
    PublicKey([u8; 32]),
    Account(AccountAddress),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WalletState {
    pub owner: Owner,
    pub limit_main: AvmAmount,
    pub limit_shell: AvmAmount,
    pub window_seconds: u32,
    pub window_start: u32,
    pub spent_main: AvmAmount,
    pub spent_shell: AvmAmount,
    pub transfer_count: u64,
    pub receive_count: u64,
}

impl WalletState {
    pub fn new(owner: Owner) -> Self {
        Self {
            owner,
            limit_main: 0,
            limit_shell: 0,
            window_seconds: 0,
            window_start: 0,
            spent_main: 0,
            spent_shell: 0,
            transfer_count: 0,
            receive_count: 0,
        }
    }

    pub fn decode(bytes: &[u8]) -> Option<Self> {
        if bytes.len() != STATE_BYTES || bytes[0] != STATE_VERSION || !is_zero(&bytes[2..8]) {
            return None;
        }
        let owner = decode_owner(bytes[1], &bytes[8..72])?;
        let state = Self {
            owner,
            limit_main: read_u128(bytes, 72)?,
            limit_shell: read_u128(bytes, 88)?,
            window_seconds: read_u32(bytes, 104)?,
            window_start: read_u32(bytes, 108)?,
            spent_main: read_u128(bytes, 112)?,
            spent_shell: read_u128(bytes, 128)?,
            transfer_count: read_u64(bytes, 144)?,
            receive_count: read_u64(bytes, 152)?,
        };
        if state.has_limit() && state.window_seconds == 0 {
            return None;
        }
        Some(state)
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut bytes = vec![0; STATE_BYTES];
        bytes[0] = STATE_VERSION;
        let (kind, owner) = encode_owner(self.owner);
        bytes[1] = kind;
        bytes[8..72].copy_from_slice(&owner);
        bytes[72..88].copy_from_slice(&self.limit_main.to_le_bytes());
        bytes[88..104].copy_from_slice(&self.limit_shell.to_le_bytes());
        bytes[104..108].copy_from_slice(&self.window_seconds.to_le_bytes());
        bytes[108..112].copy_from_slice(&self.window_start.to_le_bytes());
        bytes[112..128].copy_from_slice(&self.spent_main.to_le_bytes());
        bytes[128..144].copy_from_slice(&self.spent_shell.to_le_bytes());
        bytes[144..152].copy_from_slice(&self.transfer_count.to_le_bytes());
        bytes[152..160].copy_from_slice(&self.receive_count.to_le_bytes());
        bytes
    }

    fn has_limit(&self) -> bool {
        self.limit_main != 0 || self.limit_shell != 0
    }

    /// Applies one outgoing transfer against the spend window.
    fn spend(&mut self, now: u32, main: AvmAmount, shell: AvmAmount) -> Result<(), u32> {
        if !self.has_limit() {
            return Ok(());
        }
        let window_end = self.window_start.checked_add(self.window_seconds);
        if window_end.is_none_or(|end| now >= end) || now < self.window_start {
            self.window_start = now;
            self.spent_main = 0;
            self.spent_shell = 0;
        }
        let spent_main = self.spent_main.checked_add(main).ok_or(ERR_LIMIT_EXCEEDED)?;
        let spent_shell = self.spent_shell.checked_add(shell).ok_or(ERR_LIMIT_EXCEEDED)?;
        if (self.limit_main != 0 && spent_main > self.limit_main)
            || (self.limit_shell != 0 && spent_shell > self.limit_shell)
        {
            return Err(ERR_LIMIT_EXCEEDED);
        }
        self.spent_main = spent_main;
        self.spent_shell = spent_shell;
        Ok(())
    }
}

/// Decoded `wallet.transfer` arguments (128 bytes):
///   0..32 destination dapp id, 32..64 destination account id,
///   64..80 main value, 80..96 SHELL to transfer, 96..112 FUEL reserve in SHELL,
///   112..120 destination method id, 120 flags, 121..128 reserved zero.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TransferArgs {
    pub destination: AccountAddress,
    pub value_main: AvmAmount,
    pub shell: AvmAmount,
    pub fuel_fee_shells: AvmAmount,
    pub method_id: u64,
    pub flags: u8,
}

impl TransferArgs {
    pub fn decode(bytes: &[u8]) -> Option<Self> {
        if bytes.len() != TRANSFER_ARGS_BYTES
            || !is_zero(&bytes[121..128])
            || bytes[120] & !TRANSFER_FLAG_CALLBACK != 0
        {
            return None;
        }
        let destination = AccountAddress {
            dapp_id: bytes[0..32].try_into().ok()?,
            account_id: bytes[32..64].try_into().ok()?,
        };
        if is_zero(&destination.account_id) {
            return None;
        }
        Some(Self {
            destination,
            value_main: read_u128(bytes, 64)?,
            shell: read_u128(bytes, 80)?,
            fuel_fee_shells: read_u128(bytes, 96)?,
            method_id: read_u64(bytes, 112)?,
            flags: bytes[120],
        })
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut bytes = vec![0; TRANSFER_ARGS_BYTES];
        bytes[0..32].copy_from_slice(&self.destination.dapp_id);
        bytes[32..64].copy_from_slice(&self.destination.account_id);
        bytes[64..80].copy_from_slice(&self.value_main.to_le_bytes());
        bytes[80..96].copy_from_slice(&self.shell.to_le_bytes());
        bytes[96..112].copy_from_slice(&self.fuel_fee_shells.to_le_bytes());
        bytes[112..120].copy_from_slice(&self.method_id.to_le_bytes());
        bytes[120] = self.flags;
        bytes
    }
}

struct Wallet;

impl Contract for Wallet {
    fn init(input: MessageInput<'_>) -> MessageOutput {
        let owner = match input.caller {
            Caller::External(pubkey) if !is_zero(&pubkey) => Owner::PublicKey(pubkey),
            Caller::Internal(address) if address != input.destination => Owner::Account(address),
            _ => return MessageOutput::failure(ERR_INVALID_INIT),
        };
        MessageOutput::success(StateUpdate::FullReplace(WalletState::new(owner).encode()))
    }

    fn process_message(input: MessageInput<'_>) -> MessageOutput {
        let Some(mut state) = WalletState::decode(input.state) else {
            return MessageOutput::failure(ERR_INVALID_STATE);
        };
        if input.message.is_callback() {
            return process_callback(&input, state);
        }
        match input.message.method_id() {
            RECEIVE_METHOD_ID => process_receive(&input, state),
            TRANSFER_METHOD_ID => {
                if !is_owner(&input, &state) {
                    return MessageOutput::failure(ERR_UNAUTHORIZED);
                }
                process_transfer(&input, &mut state)
            }
            SET_LIMIT_METHOD_ID => {
                if !is_owner(&input, &state) {
                    return MessageOutput::failure(ERR_UNAUTHORIZED);
                }
                process_set_limit(&input, &mut state)
            }
            SET_OWNER_METHOD_ID => {
                if !is_owner(&input, &state) {
                    return MessageOutput::failure(ERR_UNAUTHORIZED);
                }
                process_set_owner(&input, &mut state)
            }
            _ => MessageOutput::failure(ERR_UNKNOWN_METHOD),
        }
    }
}

fn is_owner(input: &MessageInput<'_>, state: &WalletState) -> bool {
    match (state.owner, input.caller) {
        (Owner::PublicKey(owner), Caller::External(caller)) => owner == caller,
        (Owner::Account(owner), Caller::Internal(caller)) => owner == caller,
        _ => false,
    }
}

fn process_receive(input: &MessageInput<'_>, mut state: WalletState) -> MessageOutput {
    let Caller::Internal(_) = input.caller else {
        return MessageOutput::failure(ERR_UNKNOWN_METHOD);
    };
    if !input.message.body().is_empty() {
        return MessageOutput::failure(ERR_INVALID_ARGS);
    }
    let Some(receive_count) = state.receive_count.checked_add(1) else {
        return MessageOutput::failure(ERR_INVALID_STATE);
    };
    state.receive_count = receive_count;
    MessageOutput::success(StateUpdate::FullReplace(state.encode()))
}

fn process_transfer(input: &MessageInput<'_>, state: &mut WalletState) -> MessageOutput {
    let Some(args) = TransferArgs::decode(input.message.body()) else {
        return MessageOutput::failure(ERR_INVALID_ARGS);
    };
    if args.destination == input.destination || (args.value_main == 0 && args.shell == 0) {
        return MessageOutput::failure(ERR_INVALID_ARGS);
    }
    let shell_balance = input
        .extra
        .iter()
        .find(|amount| amount.currency_id == SHELL_CURRENCY_ID)
        .map_or(0, |amount| amount.amount);
    let Some(shell_needed) = args.shell.checked_add(args.fuel_fee_shells) else {
        return MessageOutput::failure(ERR_INSUFFICIENT_FUNDS);
    };
    if args.value_main > input.balance_main || shell_needed > shell_balance {
        return MessageOutput::failure(ERR_INSUFFICIENT_FUNDS);
    }
    if let Err(code) = state.spend(input.context.block_time, args.value_main, args.shell) {
        return MessageOutput::failure(code);
    }
    let Some(transfer_count) = state.transfer_count.checked_add(1) else {
        return MessageOutput::failure(ERR_INVALID_STATE);
    };
    state.transfer_count = transfer_count;

    let fields = match encode_event_fields(&[
        EventField { name: "count".into(), value: EventValue::U64(transfer_count) },
        EventField { name: "destination".into(), value: EventValue::Address(args.destination) },
        EventField { name: "shell".into(), value: EventValue::U128(args.shell) },
        EventField { name: "value_main".into(), value: EventValue::U128(args.value_main) },
    ]) {
        Ok(fields) => fields,
        Err(_) => return MessageOutput::failure(ERR_EVENT_ENCODING),
    };

    let extra = if args.shell == 0 {
        Vec::new()
    } else {
        vec![CurrencyAmount { currency_id: SHELL_CURRENCY_ID, amount: args.shell }]
    };
    let mut output = MessageOutput::success(StateUpdate::FullReplace(state.encode()));
    output.outbound_intents = vec![OutboundIntent {
        input_index: 0,
        destination: args.destination,
        value_main: args.value_main,
        fuel_fee_shells: args.fuel_fee_shells,
        extra,
        method_id: args.method_id,
        body: Vec::new(),
    }];
    if args.flags & TRANSFER_FLAG_CALLBACK != 0 {
        output.callback_intents =
            vec![CallbackIntent { outbound_index: 0, handler_method_id: ON_CALLBACK_METHOD_ID }];
    }
    output.event_intents = vec![EventIntent {
        input_index: 0,
        kind: EventKind::External,
        event_class: TRANSFERRED_EVENT_CLASS,
        event_name: "wallet.transferred".into(),
        topics: vec![args.destination.account_id.to_vec()],
        fields_binary: fields,
    }];
    output
}

/// `wallet.set_limit` arguments (40 bytes): main limit u128, SHELL limit u128,
/// window seconds u32, 4 reserved zero bytes. Zero limits disable that limit.
fn process_set_limit(input: &MessageInput<'_>, state: &mut WalletState) -> MessageOutput {
    let body = input.message.body();
    if body.len() != SET_LIMIT_ARGS_BYTES || !is_zero(&body[36..40]) {
        return MessageOutput::failure(ERR_INVALID_ARGS);
    }
    let (Some(limit_main), Some(limit_shell), Some(window_seconds)) =
        (read_u128(body, 0), read_u128(body, 16), read_u32(body, 32))
    else {
        return MessageOutput::failure(ERR_INVALID_ARGS);
    };
    let has_limit = limit_main != 0 || limit_shell != 0;
    if has_limit != (window_seconds != 0) {
        return MessageOutput::failure(ERR_INVALID_ARGS);
    }
    state.limit_main = limit_main;
    state.limit_shell = limit_shell;
    state.window_seconds = window_seconds;
    state.window_start = input.context.block_time;
    state.spent_main = 0;
    state.spent_shell = 0;
    MessageOutput::success(StateUpdate::FullReplace(state.encode()))
}

/// `wallet.set_owner` arguments (72 bytes): owner kind u8, 7 reserved zero
/// bytes, then the 64-byte owner encoding used in the state.
fn process_set_owner(input: &MessageInput<'_>, state: &mut WalletState) -> MessageOutput {
    let body = input.message.body();
    if body.len() != SET_OWNER_ARGS_BYTES || !is_zero(&body[1..8]) {
        return MessageOutput::failure(ERR_INVALID_ARGS);
    }
    let Some(owner) = decode_owner(body[0], &body[8..72]) else {
        return MessageOutput::failure(ERR_INVALID_ARGS);
    };
    if owner == state.owner || owner == Owner::Account(input.destination) {
        return MessageOutput::failure(ERR_INVALID_ARGS);
    }
    state.owner = owner;
    let fields = match encode_event_fields(&[
        EventField { name: "owner".into(), value: EventValue::Data(body[8..72].to_vec()) },
        EventField { name: "owner_kind".into(), value: EventValue::U64(u64::from(body[0])) },
    ]) {
        Ok(fields) => fields,
        Err(_) => return MessageOutput::failure(ERR_EVENT_ENCODING),
    };
    let mut output = MessageOutput::success(StateUpdate::FullReplace(state.encode()));
    output.event_intents = vec![EventIntent {
        input_index: 0,
        kind: EventKind::External,
        event_class: OWNER_CHANGED_EVENT_CLASS,
        event_name: "wallet.owner_changed".into(),
        topics: Vec::new(),
        fields_binary: fields,
    }];
    output
}

/// Callbacks return unused FUEL reserve and, for a failed delivery, the
/// bounced value. The SDK decoder guarantees the callback names this wallet
/// as its cause. The wallet accepts every such callback, including failure
/// callbacks that carry no handler id, so returned value is retained. A
/// callback never authorizes an owner action; state is unchanged.
fn process_callback(input: &MessageInput<'_>, state: WalletState) -> MessageOutput {
    let Caller::Internal(_) = input.caller else {
        return MessageOutput::failure(ERR_UNKNOWN_METHOD);
    };
    MessageOutput::success(StateUpdate::FullReplace(state.encode()))
}

pub fn encode_set_limit_args(limit_main: u128, limit_shell: u128, window_seconds: u32) -> Vec<u8> {
    let mut bytes = vec![0; SET_LIMIT_ARGS_BYTES];
    bytes[0..16].copy_from_slice(&limit_main.to_le_bytes());
    bytes[16..32].copy_from_slice(&limit_shell.to_le_bytes());
    bytes[32..36].copy_from_slice(&window_seconds.to_le_bytes());
    bytes
}

pub fn encode_set_owner_args(owner: Owner) -> Vec<u8> {
    let (kind, encoded) = encode_owner(owner);
    let mut bytes = vec![0; SET_OWNER_ARGS_BYTES];
    bytes[0] = kind;
    bytes[8..72].copy_from_slice(&encoded);
    bytes
}

fn decode_owner(kind: u8, bytes: &[u8]) -> Option<Owner> {
    let first: [u8; 32] = bytes.get(0..32)?.try_into().ok()?;
    let second: [u8; 32] = bytes.get(32..64)?.try_into().ok()?;
    match kind {
        OWNER_KIND_PUBKEY if !is_zero(&first) && is_zero(&second) => Some(Owner::PublicKey(first)),
        OWNER_KIND_ACCOUNT if !is_zero(&second) => {
            Some(Owner::Account(AccountAddress { dapp_id: first, account_id: second }))
        }
        _ => None,
    }
}

fn encode_owner(owner: Owner) -> (u8, [u8; 64]) {
    let mut bytes = [0; 64];
    match owner {
        Owner::PublicKey(pubkey) => {
            bytes[..32].copy_from_slice(&pubkey);
            (OWNER_KIND_PUBKEY, bytes)
        }
        Owner::Account(address) => {
            bytes[..32].copy_from_slice(&address.dapp_id);
            bytes[32..].copy_from_slice(&address.account_id);
            (OWNER_KIND_ACCOUNT, bytes)
        }
    }
}

fn is_zero(bytes: &[u8]) -> bool {
    bytes.iter().all(|byte| *byte == 0)
}

fn read_u32(bytes: &[u8], offset: usize) -> Option<u32> {
    bytes.get(offset..offset + 4)?.try_into().ok().map(u32::from_le_bytes)
}

fn read_u64(bytes: &[u8], offset: usize) -> Option<u64> {
    bytes.get(offset..offset + 8)?.try_into().ok().map(u64::from_le_bytes)
}

fn read_u128(bytes: &[u8], offset: usize) -> Option<u128> {
    bytes.get(offset..offset + 16)?.try_into().ok().map(u128::from_le_bytes)
}

avm_contract_sdk::export_message_contract!(Wallet, arena_bytes = 262_144);

#[cfg(test)]
mod tests;
