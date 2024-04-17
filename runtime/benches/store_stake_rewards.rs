#![feature(test)]

extern crate test;

use {
    solana_accounts_db::stake_rewards::StakeReward,
    solana_runtime::bank::Bank,
    solana_sdk::{
        account::{AccountSharedData, WritableAccount},
        account_utils::StateMut,
        genesis_config::create_genesis_config,
        native_token::sol_to_lamports,
        pubkey::Pubkey,
        reward_info::RewardInfo,
        reward_type::RewardType,
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

struct PartitionedStakeReward {
    stake_pubkey: Pubkey,
    stake: Stake,
    stake_reward_info: RewardInfo,
}

impl PartitionedStakeReward {
    fn get_stake_reward(&self) -> i64 {
        self.stake_reward_info.lamports
    }

    fn maybe_from(stake_reward: &StakeReward) -> Option<Self> {
        if let Ok(StakeStateV2::Stake(_meta, stake, _flags)) = stake_reward.stake_account.state() {
            Some(Self {
                stake_pubkey: stake_reward.stake_pubkey,
                stake,
                stake_reward_info: stake_reward.stake_reward_info,
            })
        } else {
            None
        }
    }
}

fn populate_bank(bank: &Bank, num_accounts: usize, starting_stake: u64) -> Vec<StakeReward> {
    let mut stake_rewards = vec![];
    let rent_exempt_reserve = bank.get_minimum_balance_for_rent_exemption(StakeStateV2::size_of());
    let balance = rent_exempt_reserve + starting_stake;
    let voter_pubkey = Pubkey::new_unique();

    let reward_info = RewardInfo {
        reward_type: RewardType::Staking,
        lamports: 42,
        post_balance: balance + 42,
        commission: None,
    };

    for _ in 0..num_accounts {
        let stake_pubkey = Pubkey::new_unique();
        let mut stake_account =
            AccountSharedData::new(balance, StakeStateV2::size_of(), &stake::program::id());
        let stake = Stake {
            delegation: Delegation {
                voter_pubkey,
                stake: starting_stake,
                ..Delegation::default()
            },
            credits_observed: 11,
        };
        stake_account
            .set_state(&StakeStateV2::Stake(
                Meta::default(),
                stake,
                StakeFlags::default(),
            ))
            .unwrap();
        bank.store_account(&stake_pubkey, &stake_account);
        stake_rewards.push(StakeReward {
            stake_pubkey,
            stake_reward_info: reward_info.clone(),
            stake_account,
        });
    }

    stake_rewards
}

#[bench]
fn bench_store_rewards_bulk(bencher: &mut Bencher) {
    let (genesis_config, _) = create_genesis_config(10_000);
    let bank = Bank::new_with_paths_for_benches(
        &genesis_config,
        vec![PathBuf::from("bench_store_stake_bulk")],
    );
    let stake_rewards = populate_bank(&bank, NUM_ACCOUNTS, sol_to_lamports(1.0));
    bencher.iter(|| {
        bank.store_accounts((bank.slot(), &stake_rewards[..]));
    });
}

#[bench]
fn bench_store_rewards_load_and_store(bencher: &mut Bencher) {
    let (genesis_config, _) = create_genesis_config(10_000);
    let bank = Bank::new_with_paths_for_benches(
        &genesis_config,
        vec![PathBuf::from("bench_store_stake_load_and_store")],
    );
    let stake_rewards = populate_bank(&bank, NUM_ACCOUNTS, sol_to_lamports(1.0));
    let partitioned_stake_rewards: Vec<_> = stake_rewards
        .into_iter()
        .map(|stake_reward| PartitionedStakeReward::maybe_from(&stake_reward).unwrap())
        .collect();
    bencher.iter(|| {
        for reward in &partitioned_stake_rewards {
            let pubkey = reward.stake_pubkey;
            let reward_amount = reward.get_stake_reward() as u64;
            let mut account = bank.get_account_with_fixed_root(&pubkey).unwrap();
            let StakeStateV2::Stake(meta, _stake, flags) = account.state().unwrap() else {
                panic!();
            };
            account.checked_add_lamports(reward_amount).unwrap();
            account
                .set_state(&StakeStateV2::Stake(meta, reward.stake, flags))
                .unwrap();
            bank.store_account(&pubkey, &account);
        }
    });
}

#[bench]
fn bench_store_rewards_load_and_store_bulk(bencher: &mut Bencher) {
    let (genesis_config, _) = create_genesis_config(10_000);
    let bank = Bank::new_with_paths_for_benches(
        &genesis_config,
        vec![PathBuf::from("bench_store_stake_load_and_store_bulk")],
    );
    let stake_rewards = populate_bank(&bank, NUM_ACCOUNTS, sol_to_lamports(1.0));
    let partitioned_stake_rewards: Vec<_> = stake_rewards
        .into_iter()
        .map(|stake_reward| PartitionedStakeReward::maybe_from(&stake_reward).unwrap())
        .collect();
    bencher.iter(|| {
        let mut new_stake_rewards = vec![];
        for reward in &partitioned_stake_rewards {
            let pubkey = reward.stake_pubkey;
            let reward_amount = reward.get_stake_reward() as u64;
            let mut account = bank.get_account_with_fixed_root(&pubkey).unwrap();
            let StakeStateV2::Stake(meta, _stake, flags) = account.state().unwrap() else {
                panic!();
            };
            account.checked_add_lamports(reward_amount).unwrap();
            account
                .set_state(&StakeStateV2::Stake(meta, reward.stake, flags))
                .unwrap();
            new_stake_rewards.push(StakeReward {
                stake_pubkey: pubkey,
                stake_reward_info: reward.stake_reward_info,
                stake_account: account,
            });
        }
        bank.store_accounts((bank.slot(), &new_stake_rewards[..]));
    });
}
