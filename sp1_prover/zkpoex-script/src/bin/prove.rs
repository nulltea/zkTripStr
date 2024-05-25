//! An end-to-end example of using the SP1 SDK to generate a proof of a program that can be verified
//! on-chain.
//!
//! You can run this script using the following command:
//! ```shell
//! RUST_LOG=info cargo run --package fibonacci-script --bin prove --release
//! ```

use std::{
    fs,
    ops::Add,
    path::PathBuf,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use alloy_sol_types::{sol, SolType};
use clap::Parser;
use drand_core::chain::ChainInfo;
use rand::Rng;
use serde::{Deserialize, Serialize};
use sp1_sdk::{HashableKey, ProverClient, SP1Stdin};

/// The ELF (executable and linkable format) file for the Succinct RISC-V zkVM.
///
/// This file is generated by running `cargo prove build` inside the `program` directory.
pub const ZKPOEX_ELF: &[u8] = include_bytes!("../../../zk-poex/elf/riscv32im-succinct-zkvm-elf");

/// The arguments for the prove command.
#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
struct ProveArgs {
    #[clap(long)]
    calldata: String,
    #[clap(
        long,
        default_value = r#"
    {
        "gas_price": "0",
        "origin": "0x0000000000000000000000000000000000000000",
        "block_hashes": "[]",
        "block_number": "0",
        "block_coinbase": "0x0000000000000000000000000000000000000000",
        "block_timestamp": "0",
        "block_difficulty": "0",
        "block_gas_limit": "0",
        "chain_id": "1",
        "block_base_fee_per_gas": "0"
    }
"#
    )]
    blockchain_settings: String,

    #[clap(
        short,
        long,
        help = "disclose after (y/w/d/h/m/s/ms)",
        default_value = "90d"
    )]
    pub duration: Option<humantime::Duration>,
}

/// A fixture that can be used to test the verification of SP1 zkVM proofs inside Solidity.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SP1ZkPoExProofFixture {
    key: [u8; 32],
    nonce: [u8; 12],
    round: u64,
    before: String,
    after: String,
    hash_private_inputs: String,
    chacha_cipher: Vec<u8>,
    tlock_cipher: Vec<u8>,
    calldata: String,
    blockchain_settings: String,
    vkey: String,
}

fn main() {
    // Setup the logger.
    sp1_sdk::utils::setup_logger();

    // Parse the command line arguments.
    let args = ProveArgs::parse();

    let mut rng = rand::thread_rng();

    let key: [u8; 32] = rng.gen();
    let nonce: [u8; 12] = rng.gen();

    let client: drand_core::HttpClient =
        "https://api.drand.sh/dbd506d6ef76e5f386f41c651dcb808c5bcbd75471cc4eafa3f4df7ad4e4c493"
            .try_into()
            .unwrap();
    let info = client.chain_info().unwrap();

    let drand_master_key = info.public_key();

    let round = {
        let d = args
            .duration
            .expect("duration is expected if round_number isn't specified")
            .into();
        round_after(&info, d)
    };

    let mut tlock_cipher = vec![];
    tlock::encrypt(&mut tlock_cipher, &key[..], &drand_master_key, round).unwrap();

    // Setup the prover client.
    let client = ProverClient::new();

    // Setup the program.
    let (pk, vk) = client.setup(ZKPOEX_ELF);

    // Setup the inputs.;
    let mut stdin = SP1Stdin::new();
    stdin.write(&(
        key,
        nonce,
        args.calldata.clone(),
        args.blockchain_settings.clone(),
        drand_master_key,
        round,
    ));

    // Generate the proof.
    let proof = client
        .prove_compressed(&pk, stdin)
        .expect("failed to generate proof");

    let _ = fs::create_dir_all(PathBuf::from("./data"));
    std::fs::write(PathBuf::from("./data/zkpoex_enc_key"), key).expect("failed to write fixture");

    let (before, after, hash_private_inputs, chacha_cipher, _): (
        String,
        String,
        String,
        Vec<u8>,
        String,
        // Vec<u8>,
        // u64,
    ) = bincode::deserialize(proof.public_values.as_slice())
        .expect("failed to deserialize public values");

    std::fs::write(PathBuf::from("./data/zkpoex_chacha"), &chacha_cipher)
        .expect("failed to write fixture");

    std::fs::write(PathBuf::from("./data/zkpoex_tlock"), &tlock_cipher)
        .expect("failed to write fixture");

    // Create the testing fixture so we can test things end-ot-end.
    let fixture = SP1ZkPoExProofFixture {
        before,
        after,
        hash_private_inputs,
        key,
        nonce,
        round,
        chacha_cipher,
        tlock_cipher,
        calldata: args.calldata,
        blockchain_settings: args.blockchain_settings,
        vkey: vk.bytes32().to_string(),
    };

    let _ = proof.save("./zkpoex.bincode");

    // The verification key is used to verify that the proof corresponds to the execution of the
    // program on the given input.
    //
    // Note that the verification key stays the same regardless of the input.
    println!("Verification Key: {}", fixture.vkey);

    // The public values are the values whicha are publically commited to by the zkVM.
    //
    // If you need to expose the inputs or outputs of your program, you should commit them in
    // the public values.
    println!("Public Values: {}", proof.public_values.bytes());

    // Save the fixture to a file.
    let fixture_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../contracts/src/fixtures");
    std::fs::create_dir_all(&fixture_path).expect("failed to create fixture path");
    std::fs::write(
        fixture_path.join("zkpoex_fixture.json"),
        serde_json::to_string_pretty(&fixture).unwrap(),
    )
    .expect("failed to write fixture");
}

pub fn round_at(chain_info: &ChainInfo, t: SystemTime) -> u64 {
    let since_epoch = t.duration_since(UNIX_EPOCH).unwrap();
    let t_unix = since_epoch.as_secs();
    current_round(
        t_unix,
        Duration::from_secs(chain_info.period()),
        chain_info.genesis_time(),
    )
}

pub fn round_after(chain_info: &ChainInfo, d: Duration) -> u64 {
    let t = SystemTime::now().add(d);
    round_at(chain_info, t)
}

pub fn current_round(now: u64, period: Duration, genesis: u64) -> u64 {
    let (next_round, _) = next_round(now, period, genesis);

    if next_round <= 1 {
        next_round
    } else {
        next_round - 1
    }
}

pub fn next_round(now: u64, period: Duration, genesis: u64) -> (u64, u64) {
    if now < genesis {
        return (1, genesis);
    }

    let from_genesis = now - genesis;
    let next_round = (((from_genesis as f64) / (period.as_secs() as f64)).floor() + 1f64) as u64;
    let next_time = genesis + next_round * period.as_secs();

    (next_round, next_time)
}
