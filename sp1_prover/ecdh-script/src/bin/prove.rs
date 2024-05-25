//! An end-to-end example of using the SP1 SDK to generate a proof of a program that can be verified
//! on-chain.
//!
//! You can run this script using the following command:
//! ```shell
//! RUST_LOG=info cargo run --package fibonacci-script --bin prove --release
//! ```

use std::{fs, io::Read, path::PathBuf};

use alloy_sol_types::{sol, SolType};
use clap::Parser;
use rand::Rng;
use serde::{Deserialize, Serialize};
use sp1_sdk::{Groth16Proof, HashableKey, ProverClient, SP1Stdin};

/// The ELF (executable and linkable format) file for the Succinct RISC-V zkVM.
///
/// This file is generated by running `cargo prove build` inside the `program` directory.
pub const ECDH_ELF: &[u8] = include_bytes!("../../../ecdh/elf/riscv32im-succinct-zkvm-elf");

/// The arguments for the prove command.
#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
struct ProveArgs {
    // #[clap(long)]
    // local_sk: String,

    // #[clap(long)]
    // vendor_pk: String,
}

/// A fixture that can be used to test the verification of SP1 zkVM proofs inside Solidity.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SP1EcdhProofFixture {
    local_sk: String,
    vendor_pk: String,
    vkey: String,
    key_hash: String,
    public_values: String,
    proof: String,
}

sol! {
    struct KeyEncOut {
        bytes32 keyHash;
        bytes keyCipher;
    }
}

fn main() {
    // Setup the logger.
    sp1_sdk::utils::setup_logger();

    // Parse the command line arguments.
    let args = ProveArgs::parse();

    use static_dh_ecdh::ecdh::ecdh::{
        FromBytes, KeyExchange, Pkk256, Skk256, ToBytes, ECDHNISTK256,
    };

    let local_sk = ECDHNISTK256::generate_private_key([12; 32])
        .to_bytes()
        .to_vec();

    let vendor_sk = ECDHNISTK256::generate_private_key([13; 32]);
    let vendor_pk = ECDHNISTK256::generate_public_key(&vendor_sk)
        .to_bytes()
        .to_vec();

    let local_sk_hex = hex::encode(&local_sk);
    let vendor_pk_hex = hex::encode(&vendor_pk);

    println!("local sk: {}", local_sk_hex);
    println!("vendor pk: {}", vendor_pk_hex);

    // let local_sk = hex::decode(&args.local_sk).unwrap();
    // let vendor_pk = hex::decode(&args.vendor_pk).unwrap();

    let mut rng = rand::thread_rng();

    let nonce: [u8; 12] = rng.gen();

    let key: [u8; 32] = fs::read("./data/zkpoex_enc_key")
        .unwrap()
        .try_into()
        .unwrap();

    // Setup the prover client.
    let client = ProverClient::new();

    // Setup the program.
    let (pk, vk) = client.setup(ECDH_ELF);

    // Setup the inputs.;
    let mut stdin = SP1Stdin::new();
    stdin.write(&(key, nonce, local_sk, vendor_pk));

    // Generate the proof.
    let proof = client
        .prove_groth16(&pk, stdin)
        .expect("failed to generate proof");

    let KeyEncOut {
        keyHash,
        keyCipher,
    } = KeyEncOut::abi_decode(proof.public_values.as_slice(), false).unwrap();

    let key_hash = hex::encode(keyHash);
    println!("Key Hash: {}", key_hash);

    // Create the testing fixture so we can test things end-ot-end.
    let fixture = SP1EcdhProofFixture {
        local_sk: local_sk_hex,
        vendor_pk: vendor_pk_hex,
        vkey: vk.bytes32().to_string(),
        public_values: proof.public_values.bytes().to_string(),
        proof: proof.bytes().to_string(),
        key_hash,
    };

    // The verification key is used to verify that the proof corresponds to the execution of the
    // program on the given input.
    //
    // Note that the verification key stays the same regardless of the input.
    println!("Verification Key: {}", fixture.vkey);

    // The public values are the values whicha are publically commited to by the zkVM.
    //
    // If you need to expose the inputs or outputs of your program, you should commit them in
    // the public values.
    println!("Public Values: {}", fixture.public_values);

    // The proof proves to the verifier that the program was executed with some inputs that led to
    // the give public values.
    println!("Proof Bytes: {}", fixture.proof);

    // Save the fixture to a file.
    let fixture_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixtures");
    std::fs::create_dir_all(&fixture_path).expect("failed to create fixture path");
    std::fs::write(
        fixture_path.join("ecdh_fixture.json"),
        serde_json::to_string_pretty(&fixture).unwrap(),
    )
    .expect("failed to write fixture");
}
