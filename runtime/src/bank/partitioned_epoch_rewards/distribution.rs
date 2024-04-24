use {
    super::{
        Bank, EpochRewardStatus, PartitionedStakeReward, PartitionedStakeRewards, StakeRewards,
    },
    crate::{
        bank::metrics::{report_partitioned_reward_metrics, RewardsStoreMetrics},
        stake_account::StakeAccount,
    },
    log::error,
    solana_accounts_db::stake_rewards::StakeReward,
    solana_measure::measure_us,
    solana_sdk::{
        account::{AccountSharedData, ReadableAccount, WritableAccount},
        account_utils::StateMut,
        pubkey::Pubkey,
        stake::state::{Delegation, StakeStateV2},
    },
    std::sync::atomic::Ordering::Relaxed,
    thiserror::Error,
};

#[derive(Serialize, Deserialize, Debug, Error, PartialEq, Eq, Clone)]
enum DistributionError {
    #[error("stake account not found")]
    AccountNotFound,

    #[error("rewards arithmetic overflowed")]
    ArithmeticOverflow,

    #[error("stake account set_state failed")]
    UnableToSetState,
}

struct DistributionStorageResults {
    lamports_distributed: u64,
    lamports_burned: u64,
    updated_stake_rewards: StakeRewards,
}

impl Bank {
    /// Process reward distribution for the block if it is inside reward interval.
    pub(in crate::bank) fn distribute_partitioned_epoch_rewards(&mut self) {
        let EpochRewardStatus::Active(status) = &self.epoch_reward_status else {
            return;
        };

        let height = self.block_height();
        let start_block_height = status.start_block_height;
        let credit_start = start_block_height + self.get_reward_calculation_num_blocks();
        let credit_end_exclusive = credit_start + status.stake_rewards_by_partition.len() as u64;
        assert!(
            self.epoch_schedule.get_slots_in_epoch(self.epoch)
                > credit_end_exclusive.saturating_sub(credit_start)
        );

        if height >= credit_start && height < credit_end_exclusive {
            let partition_index = height - credit_start;
            self.distribute_epoch_rewards_in_partition(
                &status.stake_rewards_by_partition,
                partition_index,
            );
        }

        if height.saturating_add(1) >= credit_end_exclusive {
            datapoint_info!(
                "epoch-rewards-status-update",
                ("slot", self.slot(), i64),
                ("block_height", height, i64),
                ("active", 0, i64),
                ("start_block_height", start_block_height, i64),
            );

            assert!(matches!(
                self.epoch_reward_status,
                EpochRewardStatus::Active(_)
            ));
            self.epoch_reward_status = EpochRewardStatus::Inactive;
            self.set_epoch_rewards_sysvar_to_inactive();
        }
    }

    /// Process reward credits for a partition of rewards
    /// Store the rewards to AccountsDB, update reward history record and total capitalization.
    pub fn distribute_epoch_rewards_in_partition(
        &self,
        all_stake_rewards: &[StakeRewards],
        partition_index: u64,
    ) {
        let pre_capitalization = self.capitalization();
        let this_partition_stake_rewards = &all_stake_rewards[partition_index as usize];
        let converted_rewards: Vec<_> = this_partition_stake_rewards
            .iter()
            .map(|stake_reward| PartitionedStakeReward::maybe_from(stake_reward)
                .expect("StakeRewards should only be deserializable accounts with state StakeStateV2::Stake"))
            .collect();

        let (
            DistributionStorageResults {
                lamports_distributed,
                lamports_burned,
                updated_stake_rewards,
            },
            store_stake_accounts_us,
        ) = measure_us!(self.store_stake_accounts_in_partition(converted_rewards));

        // increase total capitalization by the distributed rewards
        self.capitalization.fetch_add(lamports_distributed, Relaxed);

        // decrease distributed capital from epoch rewards sysvar
        self.update_epoch_rewards_sysvar(lamports_distributed + lamports_burned);

        // update reward history for this partitioned distribution
        self.update_reward_history_in_partition(&updated_stake_rewards);

        let metrics = RewardsStoreMetrics {
            pre_capitalization,
            post_capitalization: self.capitalization(),
            total_stake_accounts_count: all_stake_rewards.len(),
            partition_index,
            store_stake_accounts_us,
            store_stake_accounts_count: this_partition_stake_rewards.len(),
            distributed_rewards: lamports_distributed,
            burned_rewards: lamports_burned,
        };

        report_partitioned_reward_metrics(self, metrics);
    }

    /// insert non-zero stake rewards to self.rewards
    /// Return the number of rewards inserted
    fn update_reward_history_in_partition(&self, stake_rewards: &[StakeReward]) -> usize {
        let mut rewards = self.rewards.write().unwrap();
        rewards.reserve(stake_rewards.len());
        let initial_len = rewards.len();
        stake_rewards
            .iter()
            .filter(|x| x.get_stake_reward() > 0)
            .for_each(|x| rewards.push((x.stake_pubkey, x.stake_reward_info)));
        rewards.len().saturating_sub(initial_len)
    }

    fn build_updated_stake_reward(
        stakes_cache_accounts: &im::HashMap<Pubkey, StakeAccount<Delegation>>,
        partitioned_stake_reward: PartitionedStakeReward,
    ) -> Result<StakeReward, DistributionError> {
        let stake_account = stakes_cache_accounts
            .get(&partitioned_stake_reward.stake_pubkey)
            .ok_or(DistributionError::AccountNotFound)?
            .clone();

        let (mut account, stake_state): (AccountSharedData, StakeStateV2) = stake_account.into();
        let StakeStateV2::Stake(meta, _stake, flags) = stake_state else {
            // StakesCache only stores accounts where StakeStateV2::delegation().is_some()
            unreachable!()
        };
        account
            .checked_add_lamports(partitioned_stake_reward.stake_reward_info.lamports as u64)
            .map_err(|_| DistributionError::ArithmeticOverflow)?;
        account
            .set_state(&StakeStateV2::Stake(
                meta,
                partitioned_stake_reward.stake,
                flags,
            ))
            .map_err(|_| DistributionError::UnableToSetState)?;
        let mut stake_reward_info = partitioned_stake_reward.stake_reward_info;
        stake_reward_info.post_balance = account.lamports();
        Ok(StakeReward {
            stake_pubkey: partitioned_stake_reward.stake_pubkey,
            stake_reward_info,
            stake_account: account,
        })
    }

    /// Store stake rewards in partition
    /// Returns DistributionStorageResults containing the sum of all the rewards
    /// stored, the sum of all rewards burned, and the updated StakeRewards.
    /// Because stake accounts are checked in calculation, and further state
    /// mutation prevents by stake-program restrictions, there should never be
    /// rewards burned.
    ///
    /// Note: even if staker's reward is 0, the stake account still needs to be
    /// stored because credits observed has changed
    fn store_stake_accounts_in_partition(
        &self,
        stake_rewards: PartitionedStakeRewards,
    ) -> DistributionStorageResults {
        let mut lamports_distributed = 0;
        let mut lamports_burned = 0;
        let mut updated_stake_rewards = vec![];
        let stakes_cache = self.stakes_cache.stakes();
        let stakes_cache_accounts = stakes_cache.stake_delegations();
        for partitioned_stake_reward in stake_rewards.into_iter() {
            let stake_pubkey = partitioned_stake_reward.stake_pubkey;
            let reward_amount = partitioned_stake_reward.stake_reward_info.lamports as u64;
            match Self::build_updated_stake_reward(stakes_cache_accounts, partitioned_stake_reward)
            {
                Ok(stake_reward) => {
                    lamports_distributed += reward_amount;
                    updated_stake_rewards.push(stake_reward);
                }
                Err(err) => {
                    error!(
                        "bank::distribution::store_stake_accounts_in_partition() failed for {}: {:?}",
                        stake_pubkey, err
                    );
                    lamports_burned += reward_amount;
                }
            }
        }
        drop(stakes_cache);
        self.store_accounts((self.slot(), &updated_stake_rewards[..]));
        DistributionStorageResults {
            lamports_distributed,
            lamports_burned,
            updated_stake_rewards,
        }
    }
}

#[cfg(test)]
mod tests {
    use {
        super::*,
        crate::bank::{
            partitioned_epoch_rewards::epoch_rewards_hasher::hash_rewards_into_partitions,
            tests::create_genesis_config,
        },
        rand::Rng,
        solana_accounts_db::stake_rewards::StakeReward,
        solana_sdk::{
            account::from_account,
            epoch_schedule::EpochSchedule,
            feature_set,
            hash::Hash,
            native_token::LAMPORTS_PER_SOL,
            rent::Rent,
            reward_info::RewardInfo,
            reward_type::RewardType,
            stake::{
                stake_flags::StakeFlags,
                state::{Meta, Stake},
            },
            sysvar,
        },
        solana_stake_program::stake_state,
        solana_vote_program::vote_state,
    };

    #[test]
    fn test_distribute_partitioned_epoch_rewards() {
        let (genesis_config, _mint_keypair) = create_genesis_config(1_000_000 * LAMPORTS_PER_SOL);
        let mut bank = Bank::new_for_tests(&genesis_config);

        let expected_num = 100;

        let stake_rewards = (0..expected_num)
            .map(|_| StakeReward::new_random())
            .collect::<Vec<_>>();

        let stake_rewards = hash_rewards_into_partitions(stake_rewards, &Hash::new(&[1; 32]), 2);

        bank.set_epoch_reward_status_active(stake_rewards);

        bank.distribute_partitioned_epoch_rewards();
    }

    #[test]
    #[should_panic(expected = "self.epoch_schedule.get_slots_in_epoch")]
    fn test_distribute_partitioned_epoch_rewards_too_many_partitions() {
        let (genesis_config, _mint_keypair) = create_genesis_config(1_000_000 * LAMPORTS_PER_SOL);
        let mut bank = Bank::new_for_tests(&genesis_config);

        let expected_num = 1;

        let stake_rewards = (0..expected_num)
            .map(|_| StakeReward::new_random())
            .collect::<Vec<_>>();

        let stake_rewards = hash_rewards_into_partitions(
            stake_rewards,
            &Hash::new(&[1; 32]),
            bank.epoch_schedule().slots_per_epoch as usize + 1,
        );

        bank.set_epoch_reward_status_active(stake_rewards);

        bank.distribute_partitioned_epoch_rewards();
    }

    #[test]
    fn test_distribute_partitioned_epoch_rewards_empty() {
        let (genesis_config, _mint_keypair) = create_genesis_config(1_000_000 * LAMPORTS_PER_SOL);
        let mut bank = Bank::new_for_tests(&genesis_config);

        bank.set_epoch_reward_status_active(vec![]);

        bank.distribute_partitioned_epoch_rewards();
    }

    fn populate_starting_stake_accounts_from_stake_rewards(bank: &Bank, rewards: &[StakeReward]) {
        let rent = Rent::free();
        let validator_pubkey = Pubkey::new_unique();
        let validator_vote_pubkey = Pubkey::new_unique();
        let starting_lamports = 20;

        let validator_vote_account = vote_state::create_account(
            &validator_vote_pubkey,
            &validator_pubkey,
            10,
            starting_lamports,
        );

        for stake_reward in rewards.iter() {
            // store account in Bank, since distribution now checks for account existence
            let validator_stake_account = stake_state::create_account(
                &stake_reward.stake_pubkey,
                &validator_vote_pubkey,
                &validator_vote_account,
                &rent,
                starting_lamports,
            );
            bank.store_account(&stake_reward.stake_pubkey, &validator_stake_account);
        }
    }

    /// Test distribute partitioned epoch rewards
    #[test]
    fn test_distribute_partitioned_epoch_rewards_bank_capital_and_sysvar_balance() {
        let (mut genesis_config, _mint_keypair) =
            create_genesis_config(1_000_000 * LAMPORTS_PER_SOL);
        genesis_config.epoch_schedule = EpochSchedule::custom(432000, 432000, false);
        let mut bank = Bank::new_for_tests(&genesis_config);
        bank.activate_feature(&feature_set::enable_partitioned_epoch_reward::id());

        // Set up epoch_rewards sysvar with rewards with 1e9 lamports to distribute.
        let total_rewards = 1_000_000_000;
        let num_partitions = 2; // num_partitions is arbitrary and unimportant for this test
        let total_points = (total_rewards * 42) as u128; // total_points is arbitrary for the purposes of this test
        bank.create_epoch_rewards_sysvar(total_rewards, 0, 42, num_partitions, total_points);
        let pre_epoch_rewards_account = bank.get_account(&sysvar::epoch_rewards::id()).unwrap();
        let expected_balance =
            bank.get_minimum_balance_for_rent_exemption(pre_epoch_rewards_account.data().len());
        // Expected balance is the sysvar rent-exempt balance
        assert_eq!(pre_epoch_rewards_account.lamports(), expected_balance);

        // Set up a partition of rewards to distribute
        let expected_num = 100;
        let mut stake_rewards = (0..expected_num)
            .map(|_| StakeReward::new_random())
            .collect::<Vec<_>>();
        populate_starting_stake_accounts_from_stake_rewards(&bank, &stake_rewards);
        let mut rewards_to_distribute = 0;
        for stake_reward in &mut stake_rewards {
            stake_reward.credit(100);
            rewards_to_distribute += 100;
        }
        let all_rewards = vec![stake_rewards];

        // Distribute rewards
        let pre_cap = bank.capitalization();
        bank.distribute_epoch_rewards_in_partition(&all_rewards, 0);
        let post_cap = bank.capitalization();
        let post_epoch_rewards_account = bank.get_account(&sysvar::epoch_rewards::id()).unwrap();

        // Assert that epoch rewards sysvar lamports balance does not change
        assert_eq!(post_epoch_rewards_account.lamports(), expected_balance);

        let epoch_rewards: sysvar::epoch_rewards::EpochRewards =
            from_account(&post_epoch_rewards_account).unwrap();
        assert_eq!(epoch_rewards.total_rewards, total_rewards);
        assert_eq!(epoch_rewards.distributed_rewards, rewards_to_distribute,);

        // Assert that the bank total capital changed by the amount of rewards
        // distributed
        assert_eq!(pre_cap + rewards_to_distribute, post_cap);
    }

    /// Test partitioned credits and reward history updates of epoch rewards do cover all the rewards
    /// slice.
    #[test]
    fn test_epoch_credit_rewards_and_history_update() {
        let (mut genesis_config, _mint_keypair) =
            create_genesis_config(1_000_000 * LAMPORTS_PER_SOL);
        genesis_config.epoch_schedule = EpochSchedule::custom(432000, 432000, false);
        let mut bank = Bank::new_for_tests(&genesis_config);

        // setup the expected number of stake rewards
        let expected_num = 12345;

        let mut stake_rewards = (0..expected_num)
            .map(|_| StakeReward::new_random())
            .collect::<Vec<_>>();

        bank.store_accounts((bank.slot(), &stake_rewards[..]));

        // Simulate rewards
        let mut expected_rewards = 0;
        for stake_reward in &mut stake_rewards {
            stake_reward.credit(1);
            expected_rewards += 1;
        }

        // Push extra StakeReward to simulate non-existent account
        stake_rewards.push(StakeReward::new_random());

        let stake_rewards_bucket =
            hash_rewards_into_partitions(stake_rewards, &Hash::new(&[1; 32]), 100);
        bank.set_epoch_reward_status_active(stake_rewards_bucket.clone());

        // Test partitioned stores
        let mut total_rewards = 0;
        let mut total_num_updates = 0;

        let pre_update_history_len = bank.rewards.read().unwrap().len();

        for stake_rewards in stake_rewards_bucket {
            let converted_rewards: Vec<_> = stake_rewards
                .iter()
                .map(|stake_reward| PartitionedStakeReward::maybe_from(stake_reward).unwrap())
                .collect();
            let DistributionStorageResults {
                lamports_distributed,
                updated_stake_rewards,
                ..
            } = bank.store_stake_accounts_in_partition(converted_rewards);
            let num_history_updates =
                bank.update_reward_history_in_partition(&updated_stake_rewards);
            assert_eq!(updated_stake_rewards.len(), num_history_updates);
            total_rewards += lamports_distributed;
            total_num_updates += num_history_updates;
        }

        let post_update_history_len = bank.rewards.read().unwrap().len();

        // assert that all rewards are credited
        assert_eq!(total_rewards, expected_rewards);
        assert_eq!(total_num_updates, expected_num);
        assert_eq!(
            total_num_updates,
            post_update_history_len - pre_update_history_len
        );
    }

    #[test]
    fn test_update_reward_history_in_partition() {
        for zero_reward in [false, true] {
            let (genesis_config, _mint_keypair) =
                create_genesis_config(1_000_000 * LAMPORTS_PER_SOL);
            let bank = Bank::new_for_tests(&genesis_config);

            let mut expected_num = 100;

            let mut stake_rewards = (0..expected_num)
                .map(|_| StakeReward::new_random())
                .collect::<Vec<_>>();

            let mut rng = rand::thread_rng();
            let i_zero = rng.gen_range(0..expected_num);
            if zero_reward {
                // pick one entry to have zero rewards so it gets ignored
                stake_rewards[i_zero].stake_reward_info.lamports = 0;
            }

            let num_in_history = bank.update_reward_history_in_partition(&stake_rewards);

            if zero_reward {
                stake_rewards.remove(i_zero);
                // -1 because one of them had zero rewards and was ignored
                expected_num -= 1;
            }

            bank.rewards
                .read()
                .unwrap()
                .iter()
                .zip(stake_rewards.iter())
                .for_each(|((k, reward_info), expected_stake_reward)| {
                    assert_eq!(
                        (
                            &expected_stake_reward.stake_pubkey,
                            &expected_stake_reward.stake_reward_info
                        ),
                        (k, reward_info)
                    );
                });

            assert_eq!(num_in_history, expected_num);
        }
    }

    #[test]
    fn test_build_updated_stake_reward() {
        let (genesis_config, _mint_keypair) = create_genesis_config(1_000_000 * LAMPORTS_PER_SOL);
        let bank = Bank::new_for_tests(&genesis_config);

        let voter_pubkey = Pubkey::new_unique();
        let new_stake = Stake {
            delegation: Delegation {
                voter_pubkey,
                stake: 55_555,
                ..Delegation::default()
            },
            credits_observed: 42,
        };
        let stake_reward_info = RewardInfo {
            reward_type: RewardType::Staking,
            lamports: 100,
            post_balance: 0,
            commission: None,
        };

        let nonexistent_account = Pubkey::new_unique();
        let partitioned_stake_reward = PartitionedStakeReward {
            stake_pubkey: nonexistent_account,
            stake: new_stake,
            stake_reward_info,
        };
        let stakes_cache = bank.stakes_cache.stakes();
        let stakes_cache_accounts = stakes_cache.stake_delegations();
        assert_eq!(
            Bank::build_updated_stake_reward(stakes_cache_accounts, partitioned_stake_reward)
                .unwrap_err(),
            DistributionError::AccountNotFound
        );
        drop(stakes_cache);

        let rent_exempt_reserve = 2_282_880;

        let overflowing_account = Pubkey::new_unique();
        let mut stake_account = AccountSharedData::new(
            u64::MAX - 99,
            StakeStateV2::size_of(),
            &solana_sdk::stake::program::id(),
        );
        stake_account
            .set_state(&StakeStateV2::Stake(
                Meta::default(),
                new_stake,
                StakeFlags::default(),
            ))
            .unwrap();
        bank.store_account(&overflowing_account, &stake_account);
        let partitioned_stake_reward = PartitionedStakeReward {
            stake_pubkey: overflowing_account,
            stake: new_stake,
            stake_reward_info,
        };
        let stakes_cache = bank.stakes_cache.stakes();
        let stakes_cache_accounts = stakes_cache.stake_delegations();
        assert_eq!(
            Bank::build_updated_stake_reward(stakes_cache_accounts, partitioned_stake_reward)
                .unwrap_err(),
            DistributionError::ArithmeticOverflow
        );
        drop(stakes_cache);

        let successful_account = Pubkey::new_unique();
        let mut stake_account = AccountSharedData::new(
            rent_exempt_reserve,
            StakeStateV2::size_of(),
            &solana_sdk::stake::program::id(),
        );
        let other_stake = Stake {
            delegation: Delegation {
                voter_pubkey,
                stake: 11_111,
                ..Delegation::default()
            },
            credits_observed: 11,
        };
        stake_account
            .set_state(&StakeStateV2::Stake(
                Meta::default(),
                other_stake,
                StakeFlags::default(),
            ))
            .unwrap();
        bank.store_account(&successful_account, &stake_account);
        let partitioned_stake_reward = PartitionedStakeReward {
            stake_pubkey: successful_account,
            stake: new_stake,
            stake_reward_info,
        };
        let stakes_cache = bank.stakes_cache.stakes();
        let stakes_cache_accounts = stakes_cache.stake_delegations();
        let expected_lamports = rent_exempt_reserve + stake_reward_info.lamports as u64;
        let mut expected_stake_account = AccountSharedData::new(
            expected_lamports,
            StakeStateV2::size_of(),
            &solana_sdk::stake::program::id(),
        );
        expected_stake_account
            .set_state(&StakeStateV2::Stake(
                Meta::default(),
                new_stake,
                StakeFlags::default(),
            ))
            .unwrap();
        let mut expected_reward_info = stake_reward_info;
        expected_reward_info.post_balance = expected_lamports;
        let expected_stake_reward = StakeReward {
            stake_pubkey: successful_account,
            stake_account: expected_stake_account,
            stake_reward_info: expected_reward_info,
        };
        assert_eq!(
            Bank::build_updated_stake_reward(stakes_cache_accounts, partitioned_stake_reward)
                .unwrap(),
            expected_stake_reward
        );
        drop(stakes_cache);
    }

    #[test]
    fn test_update_reward_history_in_partition_empty() {
        let (genesis_config, _mint_keypair) = create_genesis_config(1_000_000 * LAMPORTS_PER_SOL);
        let bank = Bank::new_for_tests(&genesis_config);

        let stake_rewards = vec![];

        let num_in_history = bank.update_reward_history_in_partition(&stake_rewards);
        assert_eq!(num_in_history, 0);
    }

    /// Test rewards computation and partitioned rewards distribution at the epoch boundary
    #[test]
    fn test_store_stake_accounts_in_partition() {
        let (genesis_config, _mint_keypair) = create_genesis_config(1_000_000 * LAMPORTS_PER_SOL);
        let bank = Bank::new_for_tests(&genesis_config);

        let expected_num = 100;

        let stake_rewards = (0..expected_num)
            .map(|_| StakeReward::new_random())
            .collect::<Vec<_>>();
        populate_starting_stake_accounts_from_stake_rewards(&bank, &stake_rewards);
        let converted_rewards: Vec<_> = stake_rewards
            .iter()
            .map(|stake_reward| PartitionedStakeReward::maybe_from(stake_reward).unwrap())
            .collect();

        let expected_total = converted_rewards
            .iter()
            .map(|stake_reward| stake_reward.stake_reward_info.lamports)
            .sum::<i64>() as u64;

        let DistributionStorageResults {
            lamports_distributed,
            ..
        } = bank.store_stake_accounts_in_partition(converted_rewards);
        assert_eq!(expected_total, lamports_distributed);
    }

    #[test]
    fn test_store_stake_accounts_in_partition_empty() {
        let (genesis_config, _mint_keypair) = create_genesis_config(1_000_000 * LAMPORTS_PER_SOL);
        let bank = Bank::new_for_tests(&genesis_config);

        let stake_rewards = vec![];

        let expected_total = 0;

        let DistributionStorageResults {
            lamports_distributed,
            ..
        } = bank.store_stake_accounts_in_partition(stake_rewards);
        assert_eq!(expected_total, lamports_distributed);
    }
}
