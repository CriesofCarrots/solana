#![allow(clippy::integer_arithmetic)]
use log::*;
use solana_bench_tps_tpu_client::bench::{
    do_bench_tps, fund_keypairs, generate_and_fund_keypairs, generate_keypairs,
};
use solana_bench_tps_tpu_client::cli;
use solana_client::{
    rpc_client::RpcClient,
    thin_client::ThinClient,
    tpu_client::{TpuClient, TpuClientConfig},
};
use solana_genesis::Base64Account;
use solana_gossip::cluster_info::VALIDATOR_PORT_RANGE;
use solana_sdk::{
    commitment_config::CommitmentConfig,
    fee_calculator::FeeRateGovernor,
    signature::{Keypair, Signer},
    system_program,
};
use std::{
    collections::HashMap,
    fs::File,
    io::prelude::*,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    path::Path,
    process::exit,
    sync::Arc,
};

/// Number of signatures for all transactions in ~1 week at ~100K TPS
pub const NUM_SIGNATURES_FOR_TXS: u64 = 100_000 * 60 * 60 * 24 * 7;

fn main() {
    solana_logger::setup_with_default("solana=info");
    solana_metrics::set_panic_hook("bench-tps");

    let matches = cli::build_args(solana_version::version!()).get_matches();
    let cli_config = cli::extract_args(&matches);

    let cli::Config {
        json_rpc_url,
        websocket_url,
        id,
        tx_count,
        keypair_multiplier,
        client_ids_and_stake_file,
        write_to_client_file,
        read_from_client_file,
        target_lamports_per_signature,
        num_lamports_per_account,
        ..
    } = &cli_config;

    let keypair_count = *tx_count * keypair_multiplier;
    if *write_to_client_file {
        info!("Generating {} keypairs", keypair_count);
        let (keypairs, _) = generate_keypairs(id, keypair_count as u64);
        let num_accounts = keypairs.len() as u64;
        let max_fee =
            FeeRateGovernor::new(*target_lamports_per_signature, 0).max_lamports_per_signature;
        let num_lamports_per_account = (num_accounts - 1 + NUM_SIGNATURES_FOR_TXS * max_fee)
            / num_accounts
            + num_lamports_per_account;
        let mut accounts = HashMap::new();
        keypairs.iter().for_each(|keypair| {
            accounts.insert(
                serde_json::to_string(&keypair.to_bytes().to_vec()).unwrap(),
                Base64Account {
                    balance: num_lamports_per_account,
                    executable: false,
                    owner: system_program::id().to_string(),
                    data: String::new(),
                },
            );
        });

        info!("Writing {}", client_ids_and_stake_file);
        let serialized = serde_yaml::to_string(&accounts).unwrap();
        let path = Path::new(&client_ids_and_stake_file);
        let mut file = File::create(path).unwrap();
        file.write_all(&serialized.into_bytes()).unwrap();
        return;
    }

    let rpc_client =
        RpcClient::new_with_commitment(json_rpc_url.to_string(), CommitmentConfig::confirmed());
    // Build a ThinClient to avoid churn. Should not be used for transaction submission.
    let dummy_tpu_addr = SocketAddr::from(([127, 0, 0, 1], 0));
    let dummy_transactions_socket = solana_net_utils::bind_in_range(
        IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)),
        VALIDATOR_PORT_RANGE,
    )
    .unwrap()
    .1;
    let client = Arc::new(ThinClient::new_from_client(
        dummy_tpu_addr,
        dummy_transactions_socket,
        rpc_client,
    ));
    let rpc_client = Arc::new(RpcClient::new_with_commitment(
        json_rpc_url.to_string(),
        CommitmentConfig::confirmed(),
    ));
    let tpu_client = Arc::new(
        TpuClient::new(rpc_client, websocket_url, TpuClientConfig::default()).unwrap_or_else(
            |err| {
                eprintln!("Could not create TpuClient {:?}", err);
                exit(1);
            },
        ),
    );

    let keypairs = if *read_from_client_file {
        let path = Path::new(&client_ids_and_stake_file);
        let file = File::open(path).unwrap();

        info!("Reading {}", client_ids_and_stake_file);
        let accounts: HashMap<String, Base64Account> = serde_yaml::from_reader(file).unwrap();
        let mut keypairs = vec![];
        let mut last_balance = 0;

        accounts
            .into_iter()
            .for_each(|(keypair, primordial_account)| {
                let bytes: Vec<u8> = serde_json::from_str(keypair.as_str()).unwrap();
                keypairs.push(Keypair::from_bytes(&bytes).unwrap());
                last_balance = primordial_account.balance;
            });

        if keypairs.len() < keypair_count {
            eprintln!(
                "Expected {} accounts in {}, only received {} (--tx_count mismatch?)",
                keypair_count,
                client_ids_and_stake_file,
                keypairs.len(),
            );
            exit(1);
        }
        // Sort keypairs so that do_bench_tps() uses the same subset of accounts for each run.
        // This prevents the amount of storage needed for bench-tps accounts from creeping up
        // across multiple runs.
        keypairs.sort_by_key(|x| x.pubkey().to_string());
        fund_keypairs(
            client.clone(),
            id,
            &keypairs,
            keypairs.len().saturating_sub(keypair_count) as u64,
            last_balance,
        )
        .unwrap_or_else(|e| {
            eprintln!("Error could not fund keys: {:?}", e);
            exit(1);
        });
        keypairs
    } else {
        generate_and_fund_keypairs(client.clone(), id, keypair_count, *num_lamports_per_account)
            .unwrap_or_else(|e| {
                eprintln!("Error could not fund keys: {:?}", e);
                exit(1);
            })
    };

    do_bench_tps(client, tpu_client, cli_config, keypairs);
}
