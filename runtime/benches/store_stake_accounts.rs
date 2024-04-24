#![feature(test)]

extern crate test;

use {
    solana_runtime::bank::Bank,
    solana_sdk::{
        account::AccountSharedData,
        account_utils::StateMut,
        genesis_config::create_genesis_config,
        native_token::sol_to_lamports,
        pubkey::Pubkey,
        stake::{
            self,
            stake_flags::StakeFlags,
            state::{Delegation, Meta, Stake, StakeStateV2},
        },
    },
    std::path::PathBuf,
    test::Bencher,
};

const NUM_ACCOUNTS: usize = 4096;

// fn populate_bank(
//     bank: &Bank,
//     num_accounts: usize,
//     starting_stake: u64,
// ) -> Vec<(Pubkey, AccountSharedData)> {
//     let mut accounts = vec![];
//     let rent_exempt_reserve = bank.get_minimum_balance_for_rent_exemption(StakeStateV2::size_of());
//     let balance = rent_exempt_reserve + starting_stake;
//     let voter_pubkey = Pubkey::new_unique();

//     for _ in 0..num_accounts {
//         let stake_pubkey = Pubkey::new_unique();
//         let mut stake_account =
//             AccountSharedData::new(balance, StakeStateV2::size_of(), &stake::program::id());
//         let stake = Stake {
//             delegation: Delegation {
//                 voter_pubkey,
//                 stake: starting_stake,
//                 ..Delegation::default()
//             },
//             credits_observed: 11,
//         };
//         stake_account
//             .set_state(&StakeStateV2::Stake(
//                 Meta::default(),
//                 stake,
//                 StakeFlags::default(),
//             ))
//             .unwrap();
//         bank.store_account(&stake_pubkey, &stake_account);
//         accounts.push((stake_pubkey, stake_account));
//     }

//     accounts
// }

// #[bench]
// fn bench_store_stake_bulk(bencher: &mut Bencher) {
//     let (genesis_config, _) = create_genesis_config(10_000);
//     let bank = Bank::new_with_paths_for_benches(
//         &genesis_config,
//         vec![PathBuf::from("bench_store_stake_bulk")],
//     );
//     let accounts = populate_bank(&bank, NUM_ACCOUNTS, sol_to_lamports(1.0));
//     let ref_accounts: Vec<_> = accounts.iter().collect();
//     bencher.iter(|| {
//         bank.store_accounts((bank.slot(), &ref_accounts[..]));
//     });
// }

// #[bench]
// fn bench_store_stake_load_and_store(bencher: &mut Bencher) {
//     let (genesis_config, _) = create_genesis_config(10_000);
//     let bank = Bank::new_with_paths_for_benches(
//         &genesis_config,
//         vec![PathBuf::from("bench_store_stake_load_and_store")],
//     );
//     let accounts = populate_bank(&bank, NUM_ACCOUNTS, sol_to_lamports(1.0));
//     bencher.iter(|| {
//         for (pubkey, _account) in &accounts {
//             let mut account = bank.get_account_with_fixed_root(&pubkey).unwrap();
//             let StakeStateV2::Stake(meta, mut stake, flags) = account.state().unwrap() else {
//                 panic!();
//             };
//             stake.credits_observed += 10;
//             account
//                 .set_state(&StakeStateV2::Stake(meta, stake, flags))
//                 .unwrap();
//             bank.store_account(&pubkey, &account);
//         }
//     });
// }

// #[bench]
// fn bench_store_stake_load_and_store_bulk(bencher: &mut Bencher) {
//     let (genesis_config, _) = create_genesis_config(10_000);
//     let bank = Bank::new_with_paths_for_benches(
//         &genesis_config,
//         vec![PathBuf::from("bench_store_stake_load_and_store_bulk")],
//     );
//     let mut accounts = populate_bank(&bank, NUM_ACCOUNTS, sol_to_lamports(1.0));
//     bencher.iter(|| {
//         for (pubkey, old_account) in accounts.iter_mut() {
//             let account = bank.get_account_with_fixed_root(&pubkey).unwrap();
//             let StakeStateV2::Stake(_meta, mut stake, _flags) = account.state().unwrap() else {
//                 panic!();
//             };
//             stake.credits_observed += 10;
//             *old_account = account;
//         }
//         let ref_accounts: Vec<_> = accounts.iter().collect();
//         bank.store_accounts((bank.slot(), &ref_accounts[..]));
//     });
// }
