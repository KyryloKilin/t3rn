//! <!-- markdown-link-check-disable -->
//! # X-DNS Pallet
//! </pre></p></details>

// Ensure we're `no_std` when compiling for Wasm.
#![cfg_attr(not(feature = "std"), no_std)]
#![allow(clippy::type_complexity)]
#![allow(clippy::too_many_arguments)]
use codec::Encode;

use frame_support::sp_runtime::traits::Zero;
use sp_std::{collections::btree_set::BTreeSet, prelude::*};

pub use t3rn_types::{
    gateway::GatewayABIConfig,
    sfx::{EventSignature, SideEffectId, SideEffectName},
};

use frame_support::sp_runtime::traits::Saturating;
use t3rn_primitives::reexport_currency_types;
pub use t3rn_primitives::{ChainId, GatewayGenesisConfig, GatewayType, GatewayVendor};

// Re-export pallet items so that they can be accessed from the crate namespace.
pub use crate::pallet::*;

#[cfg(test)]
mod tests;

#[cfg(feature = "runtime-benchmarks")]
mod benchmarking;

pub mod weights;

use weights::WeightInfo;
reexport_currency_types!();

// Definition of the pallet logic, to be aggregated at runtime definition through
// `construct_runtime`.
#[frame_support::pallet]
pub mod pallet {
    // Import various types used to declare pallet in scope.
    use super::*;
    use crate::WeightInfo;
    use circuit_runtime_types::AssetId;
    use frame_support::{
        pallet_prelude::*,
        traits::{
            fungible::{Inspect, Mutate},
            Currency, Time,
        },
    };
    use frame_system::pallet_prelude::*;
    use sp_core::H256;
    use sp_runtime::{traits::CheckedDiv, SaturatedConversion};
    use sp_std::convert::TryInto;
    use t3rn_abi::{sfx_abi::SFXAbi, Codec};
    use t3rn_primitives::{
        attesters::AttestersReadApi,
        circuit::{AdaptiveTimeout, CircuitDLQ},
        light_client::{LightClientAsyncAPI, LightClientHeartbeat},
        portal::Portal,
        xdns::{
            EpochEstimate, FullGatewayRecord, GatewayRecord, PalletAssetsOverlay, TokenRecord, Xdns,
        },
        Bytes, ChainId, ExecutionVendor, FinalityVerifierActivity, GatewayActivity, GatewayType,
        GatewayVendor, SpeedMode, TokenInfo, TreasuryAccount, TreasuryAccountProvider,
        XDNSTopology,
    };
    use t3rn_types::{fsx::TargetId, sfx::Sfx4bId};

    use t3rn_types::{fsx::FullSideEffect, sfx::SecurityLvl};

    pub const MAX_GATEWAY_OVERVIEW_RECORDS: u32 = 1000;

    #[pallet::config]
    pub trait Config: frame_system::Config {
        /// The overarching event type.
        type RuntimeEvent: From<Event<Self>> + IsType<<Self as frame_system::Config>::RuntimeEvent>;

        /// Type representing the weight of this pallet
        type WeightInfo: weights::WeightInfo;

        /// A type that provides inspection and mutation to some fungible assets
        type Balances: Inspect<Self::AccountId> + Mutate<Self::AccountId>;

        type Currency: Currency<Self::AccountId>;

        type AssetsOverlay: PalletAssetsOverlay<Self, BalanceOf<Self>>;

        type Portal: Portal<Self>;

        type CircuitDLQ: CircuitDLQ<Self>;

        type AttestersRead: AttestersReadApi<Self::AccountId, BalanceOf<Self>, BlockNumberFor<Self>>;

        type TreasuryAccounts: TreasuryAccountProvider<Self::AccountId>;

        type SelfTokenId: Get<AssetId>;

        type SelfGatewayId: Get<ChainId>;

        type Time: Time;
    }

    // Simple declaration of the `Pallet` type. It is placeholder we use to implement traits and
    // method.
    #[pallet::pallet]
    #[pallet::without_storage_info]
    pub struct Pallet<T>(_);

    // Pallet implements [`Hooks`] trait to define some logic to execute in some context.
    #[pallet::hooks]
    impl<T: Config> Hooks<BlockNumberFor<T>> for Pallet<T> {
        // `on_initialize` is executed at the beginning of the block before any extrinsic are
        // dispatched.
        //
        // This function must return the weight consumed by `on_initialize` and `on_finalize`.
        fn on_initialize(_n: frame_system::pallet_prelude::BlockNumberFor<T>) -> Weight {
            // Anything that needs to be done at the start of the block.
            // We don't do anything here.
            Zero::zero()
        }

        // `on_finalize` is executed at the end of block after all extrinsic are dispatched.
        fn on_finalize(_n: frame_system::pallet_prelude::BlockNumberFor<T>) {
            // Perform necessary data/state clean up here.
        }

        // A runtime code run after every block and have access to extended set of APIs.
        //
        // For instance you can generate extrinsics for the upcoming produced block.
        fn offchain_worker(_n: frame_system::pallet_prelude::BlockNumberFor<T>) {
            // We don't do anything here.
            // but we could dispatch extrinsic (transaction/unsigned/inherent) using
            // sp_io::submit_extrinsic.
            // To see example on offchain worker, please refer to example-offchain-worker pallet
            // accompanied in this repository.
        }

        fn on_runtime_upgrade() -> Weight {
            // Define the maximum weight of this migration.
            let max_weight = T::DbWeight::get().reads_writes(10, 10);
            // Define the current storage migration version.
            const CURRENT_STORAGE_VERSION: u32 = 2;
            // Migrate the storage entries.
            StorageMigrations::<T>::try_mutate(|current_version| {
                match *current_version {
                    0 => {
                        // Storage Migration: StandardSideEffects -> StandardSFXABIs
                        // Storage Migration Details: 16-03-2023; v1.4.0-rc -> v1.5.0-rc
                        // Iterate through the old storage entries and migrate them.
                        for (key, _value) in StandardSideEffects::<T>::drain() {
                            let sfx4b_id = key;
                            match SFXAbi::get_standard_interface(sfx4b_id) {
                                Some(sfx_abi) => {
                                    StandardSFXABIs::<T>::insert(sfx4b_id, sfx_abi);
                                }
                                None => {
                                    log::error!(
                                "Failed to migrate StandardSideEffects to StandardSFXABIs for sfx4b_id: {:?}",
                                sfx4b_id
                            );
                                }
                            }
                        }

                        // Set migrations_done to true
                        *current_version = CURRENT_STORAGE_VERSION;

                        // Return the weight consumed by the migration.
                        Ok::<Weight, DispatchError>(max_weight)
                    }
                    // Storage Migration: Raw XDNS storage entry kill
                    // Storage Migration Details: 27-07-2023; v1.4.43-rc -> v1.4.44-rc
                    //     Many Collators on t0rn hit: frame_support::storage: (key, value) failed to decode at [225, 205, 72, 162, 242, 43, 101, 142, 192, 157, 178, 168, 200, 143, 21, 13, 175, 239, 182, 147, 135, 79, 226, 105, 210, 52, 22, 179, 228, 93, 185, 249, 114, 111, 99, 111]
                    1 => {
                        // Manually kill the old XDNS storage entry (XDNSRegistry is now replaced by Gateways)
                        frame_support::storage::unhashed::kill(&[225, 205, 72, 162, 242, 43, 101, 142, 192, 157, 178, 168, 200, 143, 21, 13, 175, 239, 182, 147, 135, 79, 226, 105, 210, 52, 22, 179, 228, 93, 185, 249, 114, 111, 99, 111]);
                        // Set migrations_done to true
                        *current_version = CURRENT_STORAGE_VERSION;
                        // Return the weight consumed by the migration.
                        Ok::<Weight, DispatchError>(T::DbWeight::get().writes(1))
                    }
                    // Storage Migration: Another Raw XDNS storage entry kill
                    // Storage Migration Details: 27-07-2023; v1.4.44-rc -> v1.4.45-rc
                    //     Many Collators on t0rn hit: frame_support::storage: (key, value) failed to decode at [84, 10, 79, 135, 84, 170, 82, 152, 163, 214, 233, 170, 9, 233, 63, 151, 78, 11,
                    //      18, 119, 80, 58, 19, 112, 111, 133, 165, 20, 116, 96, 124, 88, 24, 172, 250, 191, 195, 140, 91, 41, 106, 32, 177, 28, 37, 248, 177, 35, 27, 230, 169, 204, 8, 192, 121, 163, 226, 24, 100, 166, 207, 36, 66, 173, 219, 150, 184, 250, 101, 171, 135, 85,]
                    2 => {
                        // Manually kill the old XDNS storage entry (XDNSRegistry is now replaced by Gateways)
                        frame_support::storage::unhashed::kill(&[84, 10, 79, 135, 84, 170, 82, 152, 163, 214, 233, 170, 9, 233, 63, 151, 78, 11,
                            18, 119, 80, 58, 19, 112, 111, 133, 165, 20, 116, 96, 124, 88, 24, 172, 250,
                            191, 195, 140, 91, 41, 106, 32, 177, 28, 37, 248, 177, 35, 27, 230, 169, 204,
                            8, 192, 121, 163, 226, 24, 100, 166, 207, 36, 66, 173, 219, 150, 184, 250, 101,
                            171, 135, 85,]);
                        // Set migrations_done to true
                        *current_version = CURRENT_STORAGE_VERSION;
                        // Return the weight consumed by the migration.
                        Ok::<Weight, DispatchError>(T::DbWeight::get().writes(1))
                    }
                    // Add more migration cases here, if needed in the future
                    _ => {
                        // No migration needed.
                        Ok::<Weight, DispatchError>(Default::default())
                    }
                }
            })
                .unwrap_or_default()
        }
    }

    impl<T: Config> Pallet<T> {
        pub fn check_for_manual_verifier_overview_process(
            n: frame_system::pallet_prelude::BlockNumberFor<T>,
        ) -> Weight {
            let mut total_weight: Weight = Zero::zero();

            let latest_overview = <VerifierOverviewStore<T>>::get();
            total_weight = total_weight.saturating_add(T::DbWeight::get().reads(1));

            GatewayVendor::iterator().for_each(|verifier| {
                let verifier = verifier.clone();
                let latest_vendor_overview = latest_overview
                    .iter()
                    .find(|x| x.verifier == verifier)
                    .cloned()
                    .unwrap_or_default();

                let estimated_epoch_length = EpochHistory::<T>::get(&verifier)
                    .and_then(|epochs| epochs.last().cloned())
                    .map(|epoch| epoch.local)
                    .unwrap_or_else(|| {
                        frame_system::pallet_prelude::BlockNumberFor::<T>::from(50u8)
                    });

                if latest_vendor_overview.reported_at + estimated_epoch_length < n {
                    let mut latest_heartbeat =
                        T::Portal::get_latest_heartbeat_by_vendor(verifier.clone());
                    let epoch = latest_heartbeat.last_finalized_height;
                    if verifier == GatewayVendor::XBI {
                        let current_block_here = frame_system::Pallet::<T>::block_number();
                        latest_heartbeat.last_finalized_height = current_block_here;
                        latest_heartbeat.last_rational_height = current_block_here;
                        latest_heartbeat.last_fast_height = current_block_here;
                        latest_heartbeat.last_heartbeat = current_block_here;
                        latest_heartbeat.ever_initialized = true;
                    }
                    let weight = Self::process_single_verifier_overview(
                        n,
                        verifier,
                        epoch,
                        latest_heartbeat,
                    );
                    total_weight = total_weight.saturating_add(weight);
                }
            });

            total_weight
        }

        pub fn process_all_verifier_overviews(
            n: frame_system::pallet_prelude::BlockNumberFor<T>,
        ) -> Weight {
            Self::check_for_manual_verifier_overview_process(n)
        }

        pub fn process_single_verifier_overview(
            n: frame_system::pallet_prelude::BlockNumberFor<T>,
            verifier: GatewayVendor,
            new_epoch: frame_system::pallet_prelude::BlockNumberFor<T>,
            latest_heartbeat: LightClientHeartbeat<T>,
        ) -> Weight {
            let mut total_weight: Weight = Zero::zero();

            let (justified_height, finalized_height, updated_height, is_active) = (
                latest_heartbeat.last_rational_height,
                latest_heartbeat.last_finalized_height,
                latest_heartbeat.last_fast_height,
                !latest_heartbeat.is_halted,
            );

            let historic_overview = VerifierOverviewStoreHistory::<T>::get(&verifier);
            let mut last_record = historic_overview.last().cloned().unwrap_or_default();
            total_weight = total_weight.saturating_add(T::DbWeight::get().reads(1));

            let new_activity =
                FinalityVerifierActivity::new_for_finalized_compare(n, finalized_height);

            let mut is_moving = false;
            let (mut local_height_increase, mut target_finalized_height_increase) =
                (Zero::zero(), Zero::zero());

            if let Some(last_record) = historic_overview.last() {
                if let Some((target_increase, local_increase)) =
                    FinalityVerifierActivity::determine_finalized_reports_increase(&[
                        last_record.clone(),
                        new_activity.clone(),
                    ])
                {
                    if local_increase > Zero::zero() {
                        local_height_increase = local_increase;
                    }
                    if target_increase > Zero::zero() {
                        is_moving = true;
                        target_finalized_height_increase = target_increase;
                    }
                }
            }

            if is_moving {
                Self::update_epoch_history(
                    &verifier,
                    target_finalized_height_increase,
                    local_height_increase,
                );
            }

            if !is_moving {
                if let Some(previous) = historic_overview.iter().rev().nth(1).cloned() {
                    if let Some((target_finality_height_increase, _)) =
                        FinalityVerifierActivity::determine_finalized_reports_increase(&[
                            previous,
                            last_record.clone(),
                            new_activity,
                        ])
                    {
                        if target_finality_height_increase > Zero::zero() {
                            is_moving = true;
                        }
                    }
                }
            }
            let was_active = last_record.is_active;
            let activity = if is_active {
                FinalityVerifierActivity {
                    verifier: verifier.clone(),
                    reported_at: n,
                    justified_height,
                    finalized_height,
                    updated_height,
                    epoch: new_epoch,
                    is_active: is_active && is_moving,
                }
            } else {
                last_record.reported_at = n;
                last_record.is_active = false;
                last_record
            };

            Self::update_historic_overview(verifier.clone(), activity.clone());
            Self::update_overview_store(verifier.clone(), activity.clone());
            total_weight = total_weight.saturating_add(T::DbWeight::get().reads_writes(3, 2));

            if !was_active && activity.is_active {
                let weight = T::CircuitDLQ::process_dlq(n);
                total_weight = total_weight.saturating_add(weight);
            }

            let weight = T::CircuitDLQ::process_adaptive_xtx_timeout_queue(n, &verifier);
            total_weight = total_weight.saturating_add(weight);

            total_weight
        }

        pub fn update_historic_overview(
            verifier: GatewayVendor,
            activity: FinalityVerifierActivity<BlockNumberFor<T>>,
        ) {
            let mut historic_overview = VerifierOverviewStoreHistory::<T>::get(&verifier);
            if historic_overview.len() == MAX_GATEWAY_OVERVIEW_RECORDS as usize {
                let _ = historic_overview.remove(0);
            }
            historic_overview.push(activity);
            VerifierOverviewStoreHistory::<T>::insert(&verifier, historic_overview);
        }

        pub fn update_overview_store(
            verifier: GatewayVendor,
            activity: FinalityVerifierActivity<BlockNumberFor<T>>,
        ) {
            VerifierOverviewStore::<T>::mutate(|all_overviews| {
                if let Some(overview) = all_overviews.iter_mut().find(|o| o.verifier == verifier) {
                    *overview = activity.clone();
                } else {
                    all_overviews.push(activity.clone());
                }
            });
        }

        pub fn process_overview(n: frame_system::pallet_prelude::BlockNumberFor<T>) {
            let mut all_overviews: Vec<GatewayActivity<BlockNumberFor<T>>> = Vec::new();

            for gateway in Self::fetch_full_gateway_records() {
                let gateway_id = gateway.gateway_record.gateway_id;
                let last_finality_verifier_update = VerifierOverviewStoreHistory::<T>::get(
                    &gateway.gateway_record.verification_vendor,
                )
                .last()
                .cloned()
                .unwrap_or_else(|| FinalityVerifierActivity {
                    verifier: gateway.gateway_record.verification_vendor.clone(),
                    reported_at: Zero::zero(),
                    justified_height: Zero::zero(),
                    finalized_height: Zero::zero(),
                    updated_height: Zero::zero(),
                    epoch: Zero::zero(),
                    is_active: false,
                });

                let security_lvl = match gateway.gateway_record.escrow_account {
                    Some(_) => SecurityLvl::Escrow,
                    None => SecurityLvl::Optimistic,
                };

                let (justified_height, finalized_height, updated_height, is_active) = (
                    last_finality_verifier_update.justified_height,
                    last_finality_verifier_update.finalized_height,
                    last_finality_verifier_update.updated_height,
                    last_finality_verifier_update.is_active,
                );

                let attestation_latency = T::AttestersRead::read_attestation_latency(&gateway_id);

                let activity = GatewayActivity {
                    gateway_id,
                    reported_at: n,
                    justified_height,
                    finalized_height,
                    updated_height,
                    attestation_latency,
                    security_lvl,
                    is_active,
                };

                // Add the new activity to the historic overview of the gateway
                let mut historic_overview = GatewaysOverviewStoreHistory::<T>::get(gateway_id);
                if historic_overview.len() == MAX_GATEWAY_OVERVIEW_RECORDS as usize {
                    let _ = historic_overview.remove(0);
                }
                historic_overview.push(activity.clone());
                GatewaysOverviewStoreHistory::<T>::insert(gateway_id, historic_overview);

                // Add the new activity to the general overview
                all_overviews.push(activity);
            }

            // Update the general overview
            GatewaysOverviewStore::<T>::put(all_overviews);
        }

        // XDNS Topology Zip / Unzip
        pub fn do_zip_topology() -> XDNSTopology<T::AccountId> {
            let gateways = Self::fetch_full_gateway_records();

            // Collect all asset information
            let assets = AllTokenIds::<T>::get()
                .iter()
                .filter_map(|asset_id| Tokens::<T>::get(asset_id, T::SelfGatewayId::get()))
                .collect::<Vec<TokenRecord>>();

            // Assemble the topology structure
            XDNSTopology { gateways, assets }
        }

        pub fn do_unzip_topology(
            origin: &OriginFor<T>,
            topology: XDNSTopology<T::AccountId>,
        ) -> DispatchResult {
            // Update the gateways
            for gateway in topology.gateways {
                // Register Gateway
                let (
                    gateway_id,
                    verification_vendor,
                    execution_vendor,
                    codec,
                    registrant,
                    escrow_account,
                    allowed_side_effects,
                ) = (
                    gateway.gateway_record.gateway_id,
                    gateway.gateway_record.verification_vendor,
                    gateway.gateway_record.execution_vendor,
                    gateway.gateway_record.codec,
                    gateway.gateway_record.registrant,
                    gateway.gateway_record.escrow_account,
                    gateway.gateway_record.allowed_side_effects,
                );
                log::info!("topology unzip -- gateway_id: {:?}", gateway_id);
                Self::override_gateway(
                    gateway_id,
                    verification_vendor,
                    execution_vendor,
                    codec,
                    registrant,
                    escrow_account,
                    allowed_side_effects,
                )?;
            }

            // Update the assets
            for asset in topology.assets {
                log::info!("topology unzip -- asset_id: {:?}", asset.token_id);
                // Register Asset if not present
                if !AllTokenIds::<T>::get().contains(&asset.token_id) {
                    Self::register_new_token(origin, asset.token_id, asset.token_props)?;
                } else {
                    // Link the asset to the gateway
                    Self::link_token_to_gateway(
                        asset.token_id,
                        asset.gateway_id,
                        asset.token_props,
                    )?;
                }
            }

            Ok(())
        }
    }

    #[pallet::call]
    impl<T: Config> Pallet<T> {
        /// Re-adds the self-gateway if was present before. Inserts if wasn't. Root only access.
        #[pallet::weight(< T as Config >::WeightInfo::reboot_self_gateway())]
        pub fn reboot_self_gateway(
            origin: OriginFor<T>,
            vendor: GatewayVendor,
        ) -> DispatchResultWithPostInfo {
            Self::do_reboot_self_gateway(origin, vendor)?;

            Ok(().into())
        }

        /// Re-adds the self-gateway if was present before. Inserts if wasn't. Root only access.
        #[pallet::weight(< T as Config >::WeightInfo::reboot_self_gateway())]
        pub fn add_supported_bridging_asset(
            origin: OriginFor<T>,
            asset_id: AssetId,
            target_id: TargetId,
        ) -> DispatchResultWithPostInfo {
            ensure_root(origin)?;

            if !<AuthorizedMintAssets<T>>::get().contains(&(asset_id, target_id)) {
                <AuthorizedMintAssets<T>>::append((&asset_id, &target_id));
            }

            Ok(().into())
        }

        #[pallet::weight(< T as Config >::WeightInfo::reboot_self_gateway())]
        pub fn enroll_bridge_asset(
            origin: OriginFor<T>,
            asset_id: AssetId,
            target_id: TargetId,
            token_info: TokenInfo,
        ) -> DispatchResultWithPostInfo {
            ensure_root(origin.clone())?;

            assert!(!Self::check_asset_is_mintable(target_id, asset_id));

            if !<AllTokenIds<T>>::get().contains(&asset_id) {
                Self::register_new_token(&origin, asset_id, token_info.clone())?;
            }
            // Check that the asset is not already added to the gateway
            if !<Tokens<T>>::contains_key(&asset_id, &target_id) {
                Self::link_token_to_gateway(asset_id, target_id, token_info)?;
            }

            <AuthorizedMintAssets<T>>::append((&asset_id, &target_id));

            log::info!(
                "Enrolled asset {:?} for bridging to {:?}",
                asset_id,
                target_id
            );
            // Check that the asset is mintable
            assert!(Self::check_asset_is_mintable(target_id, asset_id));

            Ok(().into())
        }

        #[pallet::weight(< T as Config >::WeightInfo::reboot_self_gateway())]
        pub fn enroll_new_abi_to_selected_gateway(
            origin: OriginFor<T>,
            target_id: ChainId,
            sfx_4b_id: Sfx4bId,
            sfx_expected_abi: Option<SFXAbi>,
            maybe_pallet_id: Option<u8>,
        ) -> DispatchResultWithPostInfo {
            ensure_root(origin.clone())?;

            if let Some(abi) = sfx_expected_abi {
                <SFXABIRegistry<T>>::insert(target_id, sfx_4b_id, abi);
            } else {
                let mut assume_known_abi = <StandardSFXABIs<T>>::get(sfx_4b_id)
                    .ok_or(Error::<T>::SideEffectABINotFound)?;
                assume_known_abi.maybe_prefix_memo = maybe_pallet_id;
                <SFXABIRegistry<T>>::insert(target_id, sfx_4b_id, assume_known_abi);
            }

            let updated_abi_list = <SFXABIRegistry<T>>::iter_prefix(target_id).collect();

            Self::override_sfx_abi(target_id, updated_abi_list)?;

            Ok(().into())
        }

        #[pallet::weight(< T as Config >::WeightInfo::reboot_self_gateway())]
        pub fn unroll_abi_of_selected_gateway(
            origin: OriginFor<T>,
            target_id: ChainId,
            sfx_4b_id: Sfx4bId,
        ) -> DispatchResultWithPostInfo {
            ensure_root(origin.clone())?;

            <SFXABIRegistry<T>>::remove(target_id, sfx_4b_id);

            let updated_abi_list = <SFXABIRegistry<T>>::iter_prefix(target_id).collect();

            Self::override_sfx_abi(target_id, updated_abi_list)?;

            Ok(().into())
        }

        #[pallet::weight(< T as Config >::WeightInfo::reboot_self_gateway())]
        pub fn add_remote_order_address(
            origin: OriginFor<T>,
            target_id: TargetId,
            remote_address: H256,
        ) -> DispatchResultWithPostInfo {
            ensure_root(origin.clone())?;
            <RemoteOrderAddresses<T>>::insert(target_id, remote_address);
            Ok(().into())
        }

        /// Re-adds the self-gateway if was present before. Inserts if wasn't. Root only access.
        #[pallet::weight(< T as Config >::WeightInfo::reboot_self_gateway())]
        pub fn purge_supported_bridging_asset(
            origin: OriginFor<T>,
            asset_id: AssetId,
            target_id: TargetId,
        ) -> DispatchResultWithPostInfo {
            ensure_root(origin)?;

            if <AuthorizedMintAssets<T>>::get().contains(&(asset_id, target_id)) {
                <AuthorizedMintAssets<T>>::mutate(|all_token_ids| {
                    all_token_ids.retain(|&(a_id, t_id)| a_id != asset_id && t_id != target_id);
                });
            }

            Ok(().into())
        }

        /// Removes a gateway from the onchain registry. Root only access.
        #[pallet::weight(< T as Config >::WeightInfo::purge_gateway())]
        pub fn purge_gateway_record(
            origin: OriginFor<T>,
            requester: T::AccountId,
            gateway_id: TargetId,
        ) -> DispatchResultWithPostInfo {
            ensure_root(origin)?;
            if !<Gateways<T>>::contains_key(gateway_id) {
                Err(Error::<T>::XdnsRecordNotFound.into())
            } else {
                // Get associated finality verifier
                let verifier = <Gateways<T>>::get(gateway_id)
                    .expect("Ensured by Gateways::contains_key() above during purge_gateway_record")
                    .verification_vendor;

                <Gateways<T>>::remove(gateway_id);

                let token_ids = GatewayTokens::<T>::get(gateway_id);

                token_ids.iter().for_each(|token_id| {
                    <Tokens<T>>::remove(token_id, gateway_id);
                    if gateway_id == T::SelfGatewayId::get() {
                        <AllTokenIds<T>>::mutate(|all_token_ids| {
                            all_token_ids.retain(|id| id != token_id);
                        });
                    }
                });

                <GatewayTokens<T>>::remove(gateway_id);

                <AllGatewayIds<T>>::mutate(|all_gateway_ids| {
                    all_gateway_ids.retain(|&id| id != gateway_id);
                });

                let current_block = <frame_system::Pallet<T>>::block_number();

                let latest_heartbeat = T::Portal::get_latest_heartbeat_by_vendor(verifier.clone());
                let epoch = latest_heartbeat.last_finalized_height;
                let _weight = Self::process_single_verifier_overview(
                    current_block,
                    verifier,
                    epoch,
                    latest_heartbeat,
                );

                Self::deposit_event(Event::<T>::GatewayRecordPurged(requester, gateway_id));
                Ok(().into())
            }
        }

        #[pallet::weight(< T as Config >::WeightInfo::purge_gateway())]
        pub fn unlink_token(
            origin: OriginFor<T>,
            gateway_id: TargetId,
            token_id: AssetId,
        ) -> DispatchResultWithPostInfo {
            ensure_root(origin)?;

            <Tokens<T>>::remove(token_id, gateway_id);

            <GatewayTokens<T>>::mutate(gateway_id, |token_ids| {
                token_ids.retain(|&x_token_id| x_token_id != token_id);
            });

            Ok(().into())
        }

        #[pallet::weight(< T as Config >::WeightInfo::purge_gateway())]
        pub fn link_token(
            origin: OriginFor<T>,
            gateway_id: TargetId,
            token_id: AssetId,
            token_props: TokenInfo,
        ) -> DispatchResultWithPostInfo {
            ensure_root(origin)?;

            Self::link_token_to_gateway(token_id, gateway_id, token_props)?;

            Ok(().into())
        }

        /// Removes from all of the registered destinations + the onchain registry. Root only access.
        #[pallet::weight(< T as Config >::WeightInfo::purge_gateway())]
        pub fn purge_token_record(
            origin: OriginFor<T>,
            token_id: AssetId,
        ) -> DispatchResultWithPostInfo {
            // Try destroying assets with associated to sudo origin of Escrow or admin / ownership rights
            let is_root = ensure_signed_or_root(origin.clone())?.is_none();
            let origin_admin: T::RuntimeOrigin = match is_root {
                true => T::RuntimeOrigin::from(frame_system::RawOrigin::Signed(
                    T::TreasuryAccounts::get_treasury_account(TreasuryAccount::Escrow),
                )),
                false => origin,
            };

            // Try destroying assets with
            T::AssetsOverlay::destroy(origin_admin, &token_id)?;

            // Remove from all destinations
            let destinations = <Tokens<T>>::iter_prefix(token_id)
                .map(|(dest, _)| dest)
                .collect::<Vec<_>>();
            destinations.iter().for_each(|dest| {
                <Tokens<T>>::remove(token_id, *dest);
            });

            <AllTokenIds<T>>::mutate(|all_token_ids| {
                all_token_ids.retain(|&id| id != token_id);
            });

            Ok(().into())
        }

        #[pallet::weight(< T as Config >::WeightInfo::purge_gateway())]
        pub fn zip_topology(origin: OriginFor<T>) -> DispatchResult {
            let _ = ensure_signed(origin)?;
            let topology = Self::do_zip_topology();
            Self::deposit_event(Event::<T>::XDNSTopologyZip(topology.clone()));
            Ok(())
        }

        #[pallet::weight(< T as Config >::WeightInfo::purge_gateway())]
        pub fn unzip_topology(
            origin: OriginFor<T>,
            topology_decoded: Option<XDNSTopology<T::AccountId>>,
            topology_encoded: Option<Vec<u8>>,
        ) -> DispatchResult {
            let _ = ensure_root(origin.clone())?;
            let topology = if let Some(topology) = topology_decoded {
                topology
            } else if let Some(topology) = topology_encoded {
                let topology = XDNSTopology::<T::AccountId>::decode(&mut topology.as_slice())
                    .map_err(|_| Error::<T>::TopologyDecodeError)?;
                topology
            } else {
                return Err(Error::<T>::EmptyTopologySubmitted.into())
            };

            Self::do_unzip_topology(&origin, topology)?;

            Ok(())
        }
    }

    #[pallet::event]
    #[pallet::generate_deposit(pub (super) fn deposit_event)]
    pub enum Event<T: Config> {
        /// \[gateway_4b_id\]
        GatewayRecordStored(TargetId),
        /// \[asset_id, gateway_4b_id\]
        NewTokenLinkedToGateway(AssetId, TargetId),
        /// \[asset_id, gateway_4b_id\]
        NewTokenAssetRegistered(AssetId, TargetId),
        /// \[requester, gateway_record_id\]
        GatewayRecordPurged(T::AccountId, TargetId),
        /// \[requester, xdns_record_id\]
        XdnsRecordPurged(T::AccountId, TargetId),
        /// \[xdns_record_id\]
        XdnsRecordUpdated(TargetId),
        /// \[xdns_topology\]
        XDNSTopologyZip(XDNSTopology<T::AccountId>),
    }

    // Errors inform users that something went wrong.
    #[pallet::error]
    pub enum Error<T> {
        /// Stored gateway has already been added before
        GatewayRecordAlreadyExists,
        /// XDNS Record not found
        XdnsRecordNotFound,
        /// Escrow account not found
        EscrowAccountNotFound,
        /// Escrow account not found
        RemoteOrderAddressNotFound,
        /// Stored token has already been added before
        TokenRecordAlreadyExists,
        /// XDNS Token not found in assets overlay
        TokenRecordNotFoundInAssetsOverlay,
        /// XDNS Token not found in on that gateway
        TokenRecordNotFoundInGateway,
        /// Gateway Record not found
        GatewayRecordNotFound,
        /// SideEffectABI already exists
        SideEffectABIAlreadyExists,
        /// SideEffectABI not found
        SideEffectABINotFound,
        /// the xdns entry does not contain parachain information
        NoParachainInfoFound,
        /// A token is not compatible with the gateways execution layer
        TokenExecutionVendorMismatch,
        /// Gateway verified as inactive
        GatewayNotActive,
        /// Failed to decode XDNS topology at Unzip
        TopologyDecodeError,
        /// Empty topology submitted at Unzip
        EmptyTopologySubmitted,
    }

    // Deprecated storage entry -- StandardSideEffects
    // Storage Migration: StandardSideEffects -> StandardSFXABIs
    // Storage Migration Details: 16-03-2023; v1.4.0-rc -> v1.5.0-rc
    #[pallet::storage]
    pub type StandardSideEffects<T: Config> = StorageMap<_, Identity, TargetId, Vec<u8>>; // SideEffectInterface

    // Deprecated storage entry -- CustomSideEffects
    // Storage Migration: CustomSideEffects -> !dropped and replaced by SFXABIRegistry
    // Storage Migration Details: 16-03-2023; v1.4.0-rc -> v1.5.0-rc
    #[pallet::storage]
    pub type CustomSideEffects<T: Config> = StorageMap<_, Identity, SideEffectId<T>, Vec<u8>>;

    #[pallet::storage]
    #[pallet::getter(fn storage_migrations_done)]
    pub type StorageMigrations<T: Config> = StorageValue<_, u32, ValueQuery>;

    #[pallet::storage]
    pub type StandardSFXABIs<T: Config> = StorageMap<_, Identity, Sfx4bId, SFXAbi>;

    #[pallet::storage]
    pub type SFXABIRegistry<T: Config> =
        StorageDoubleMap<_, Identity, TargetId, Identity, Sfx4bId, SFXAbi>;

    #[pallet::storage]
    #[pallet::getter(fn gateways)]
    pub type Gateways<T: Config> =
        StorageMap<_, Identity, TargetId, GatewayRecord<T::AccountId>, OptionQuery>;

    // Token can be stored in multiple gateways and on each Gateway be mapped to a different TokenRecord (Substrate, Eth etc.)
    #[pallet::storage]
    #[pallet::getter(fn tokens)]
    pub type Tokens<T: Config> =
        StorageDoubleMap<_, Identity, AssetId, Identity, TargetId, TokenRecord, OptionQuery>;

    // Recover TokenRecords stored per gateway, to be able to iterate over all tokens stored on a gateway
    #[pallet::storage]
    #[pallet::getter(fn gateway_tokens)]
    pub type GatewayTokens<T: Config> = StorageMap<_, Identity, TargetId, Vec<AssetId>, ValueQuery>;

    // All known TokenIds to t3rn
    #[pallet::storage]
    #[pallet::getter(fn all_token_ids)]
    pub type AllTokenIds<T: Config> = StorageValue<_, Vec<AssetId>, ValueQuery>;

    // All known TokenIds to t3rn
    #[pallet::storage]
    #[pallet::getter(fn supported_bridging_assets)]
    pub type AuthorizedMintAssets<T: Config> =
        StorageValue<_, Vec<(AssetId, TargetId)>, ValueQuery>;

    #[pallet::storage]
    #[pallet::getter(fn all_gateway_ids)]
    pub type AllGatewayIds<T: Config> = StorageValue<_, Vec<TargetId>, ValueQuery>;

    #[pallet::storage]
    #[pallet::getter(fn remote_order_addresses)]
    pub type RemoteOrderAddresses<T: Config> = StorageMap<_, Identity, TargetId, H256, OptionQuery>;

    #[pallet::storage]
    #[pallet::getter(fn per_target_asset_estimates)]
    pub type PerTargetAssetEstimates<T: Config> = StorageDoubleMap<
        _,
        Identity,
        TargetId,
        Identity,
        (AssetId, AssetId),
        BalanceOf<T>,
        ValueQuery,
    >;

    #[pallet::storage]
    #[pallet::getter(fn asset_estimates)]
    pub type AssetEstimates<T: Config> =
        StorageMap<_, Identity, (AssetId, AssetId), BalanceOf<T>, ValueQuery>;

    #[pallet::storage]
    #[pallet::getter(fn asset_estimates_in_native)]
    pub type AssetEstimatesInNative<T: Config> =
        StorageMap<_, Identity, AssetId, BalanceOf<T>, ValueQuery>;

    #[pallet::storage]
    #[pallet::getter(fn asset_cost_estimates_in_native)]
    pub type AssetCostEstimatesInNative<T: Config> =
        StorageMap<_, Identity, AssetId, BalanceOf<T>, ValueQuery>;

    #[pallet::storage]
    #[pallet::getter(fn gateways_overview)]
    pub type GatewaysOverviewStore<T: Config> =
        StorageValue<_, Vec<GatewayActivity<BlockNumberFor<T>>>, ValueQuery>;

    #[pallet::storage]
    #[pallet::getter(fn verifier_overview)]
    pub type VerifierOverviewStore<T: Config> =
        StorageValue<_, Vec<FinalityVerifierActivity<BlockNumberFor<T>>>, ValueQuery>;

    #[pallet::storage]
    #[pallet::getter(fn gateways_overview_history)]
    pub type GatewaysOverviewStoreHistory<T: Config> = StorageMap<
        _,
        Twox64Concat,
        TargetId,                                // Gateway Id
        Vec<GatewayActivity<BlockNumberFor<T>>>, // Activity
        ValueQuery,
    >;

    #[pallet::storage]
    #[pallet::getter(fn verifier_overview_history)]
    pub type VerifierOverviewStoreHistory<T: Config> = StorageMap<
        _,
        Twox64Concat,
        GatewayVendor,                                    // Gateway Id
        Vec<FinalityVerifierActivity<BlockNumberFor<T>>>, // Activity
        ValueQuery,
    >;

    // Keep last 10 epoch estimates
    #[pallet::storage]
    #[pallet::getter(fn epoch_history)]
    pub type EpochHistory<T: Config> =
        StorageMap<_, Identity, GatewayVendor, Vec<EpochEstimate<BlockNumberFor<T>>>>;

    // The genesis config type.
    #[pallet::genesis_config]
    #[derive(frame_support::DefaultNoBound)]
    pub struct GenesisConfig<T: Config> {
        pub known_gateway_records: Vec<u8>,
        // Fixme: GatewayRecord is not serializable with DefaultNoBound with current serde settings. Debug what changed after v1.0.0 update.
        // pub known_gateway_records: Vec<GatewayRecord<T::AccountId>>,
        // pub standard_sfx_abi: Vec<(Sfx4bId, SFXAbi)>,
        pub standard_sfx_abi: Vec<u8>,
        #[serde(skip)]
        pub _marker: PhantomData<T>,
    }
    //
    // /// The default value for the genesis config type.
    // #[cfg(feature = "std")]
    // impl<T: Config> Default for GenesisConfig<T> {
    //     fn default() -> Self {
    //         Self {
    //             known_gateway_records: Default::default(),
    //             standard_sfx_abi: Default::default(),
    //         }
    //     }
    // }

    /// The build of genesis for the pallet.
    /// Populates storage with the known XDNS Records
    #[pallet::genesis_build]
    impl<T: Config> BuildGenesisConfig for GenesisConfig<T> {
        fn build(&self) {
            let known_gateway_records: Vec<GatewayRecord<T::AccountId>> =
                Decode::decode(&mut &self.known_gateway_records[..]).unwrap_or_default();

            let standard_sfx_abi: Vec<(Sfx4bId, SFXAbi)> =
                Decode::decode(&mut &self.standard_sfx_abi[..]).unwrap_or_default();

            for (sfx_4b_id, sfx_abi) in standard_sfx_abi {
                let _sfx_4b_str = sp_std::str::from_utf8(sfx_4b_id.as_slice())
                    .unwrap_or("invalid utf8 4b sfx id format");
                <StandardSFXABIs<T>>::insert(sfx_4b_id, sfx_abi);
            }

            for gateway_record in known_gateway_records {
                Pallet::<T>::override_gateway(
                    gateway_record.gateway_id,
                    gateway_record.verification_vendor,
                    gateway_record.execution_vendor,
                    gateway_record.codec,
                    gateway_record.registrant,
                    gateway_record.escrow_account,
                    gateway_record.allowed_side_effects,
                )
                .map_err(|e| {
                    log::error!(
                        "XDNS -- on-genesis: failed to add gateway via override_gateway: {:?}",
                        e
                    );
                })
                .ok();
            }
        }
    }

    impl<T: Config> Pallet<T> {
        pub fn do_reboot_self_gateway(
            origin: OriginFor<T>,
            vendor: GatewayVendor,
        ) -> DispatchResult {
            let admin: T::AccountId = ensure_signed_or_root(origin)?.unwrap_or(
                T::TreasuryAccounts::get_treasury_account(TreasuryAccount::Escrow),
            );

            // Refresh the list of StandardABI based on latest implementation
            // Purge all standards first
            if <StandardSFXABIs<T>>::iter().count() > 0 {
                <StandardSFXABIs<T>>::remove_all(None);
            }
            // Re-add all standards
            t3rn_abi::standard::standard_sfx_abi()
                .iter()
                .for_each(|(sfx_4b_id, sfx_abi)| {
                    <StandardSFXABIs<T>>::insert(sfx_4b_id, sfx_abi.clone());
                });

            let target_id = T::SelfGatewayId::get();

            const BALANCES_INDEX: u8 = 10;
            const ASSETS_INDEX: u8 = 12;
            const EVM_INDEX: u8 = 120;
            const WASM_INDEX: u8 = 121;

            let mut allowed_side_effects = vec![];

            if <StandardSFXABIs<T>>::contains_key(*b"tran") {
                allowed_side_effects.push((*b"tran", Some(BALANCES_INDEX)));
            }
            if <StandardSFXABIs<T>>::contains_key(*b"tddd") {
                allowed_side_effects.push((*b"tran", Some(ASSETS_INDEX)));
            }
            if <StandardSFXABIs<T>>::contains_key(*b"tass") {
                allowed_side_effects.push((*b"tran", Some(ASSETS_INDEX)));
            }
            if <StandardSFXABIs<T>>::contains_key(*b"cevm") {
                allowed_side_effects.push((*b"cevm", Some(EVM_INDEX)));
            }
            if <StandardSFXABIs<T>>::contains_key(*b"wasm") {
                allowed_side_effects.push((*b"wasm", Some(WASM_INDEX)));
            }

            Pallet::<T>::override_gateway(
                target_id,
                vendor,
                ExecutionVendor::Substrate,
                Codec::Scale,
                Some(admin.clone()),
                Some(admin),
                allowed_side_effects,
            )?;

            // If standards are linked to Gateways, refresh them all at once
            t3rn_abi::standard::standard_sfx_abi().iter_mut().for_each(
                |(ref sfx_4b_id, ref mut sfx_abi)| {
                    <SFXABIRegistry<T>>::iter_keys().for_each(|(gateway_id, on_dest_sfx_4b_id)| {
                        if *sfx_4b_id == on_dest_sfx_4b_id {
                            <SFXABIRegistry<T>>::mutate(gateway_id, sfx_4b_id, |target_sfx_abi| {
                                sfx_abi.maybe_prefix_memo = match target_sfx_abi {
                                    Some(target_sfx_abi) => target_sfx_abi.maybe_prefix_memo,
                                    None => None,
                                };
                                *target_sfx_abi = Some(sfx_abi.clone());
                            });
                        }
                    })
                },
            );

            Ok(())
        }

        pub fn update_epoch_history(
            verifier: &GatewayVendor,
            epoch_duration_in_remote_blocks: frame_system::pallet_prelude::BlockNumberFor<T>,
            epoch_duration_in_local_blocks: frame_system::pallet_prelude::BlockNumberFor<T>,
        ) {
            const N: usize = 10; // Number of epochs to consider for the moving average

            let mut history = EpochHistory::<T>::get(verifier).unwrap_or_default();

            // Remove the oldest elements until length is no longer than N
            while history.len() > N {
                history.remove(0);
            }

            // Calculate moving averages
            let moving_average_local = history
                .iter()
                .map(|e| e.local.saturated_into::<u32>())
                .sum::<u32>()
                .checked_div(history.len() as u32)
                .unwrap_or(Zero::zero())
                .into();

            let moving_average_remote = history
                .iter()
                .map(|e| e.remote.saturated_into::<u32>())
                .sum::<u32>()
                .checked_div(history.len() as u32)
                .unwrap_or(Zero::zero())
                .into();

            let epoch_duration_estimate_update = EpochEstimate::<BlockNumberFor<T>> {
                remote: epoch_duration_in_remote_blocks,
                local: epoch_duration_in_local_blocks,
                moving_average_local,
                moving_average_remote,
            };

            history.push(epoch_duration_estimate_update);

            EpochHistory::<T>::insert(verifier, history);
        }
    }

    impl<T: Config> LightClientAsyncAPI<T> for Pallet<T> {
        fn on_new_epoch(
            verifier: GatewayVendor,
            new_epoch: frame_system::pallet_prelude::BlockNumberFor<T>,
            current_hearbeat: LightClientHeartbeat<T>,
        ) {
            Self::process_single_verifier_overview(
                <frame_system::Pallet<T>>::block_number(),
                verifier,
                new_epoch,
                current_hearbeat,
            );
        }
    }

    impl<T: Config> Xdns<T, BalanceOf<T>> for Pallet<T> {
        /// Fetches all known Gateway records
        fn fetch_gateways() -> Vec<GatewayRecord<T::AccountId>> {
            Gateways::<T>::iter_values().collect()
        }

        /// Register new token assuming self::SelfGatewayIdOptimistic as a base chain
        fn register_new_token(
            origin: &OriginFor<T>,
            token_id: AssetId,
            token_props: TokenInfo,
        ) -> DispatchResult {
            if T::AssetsOverlay::contains_asset(&token_id) {
                return Err(Error::<T>::TokenRecordAlreadyExists.into())
            }

            if <AllTokenIds<T>>::get().contains(&token_id) {
                return Err(Error::<T>::TokenRecordAlreadyExists.into())
            }

            let admin: T::AccountId = ensure_signed_or_root(origin.clone())?.unwrap_or(
                T::TreasuryAccounts::get_treasury_account(TreasuryAccount::Escrow),
            );

            T::AssetsOverlay::force_create_asset(
                origin.clone(),
                token_id,
                admin,
                true,
                T::Currency::minimum_balance(),
            )?;

            let gateway_id = T::SelfGatewayId::get();

            Self::link_token_to_gateway(token_id, gateway_id, token_props)?;

            <AllTokenIds<T>>::append(token_id);

            Self::deposit_event(Event::<T>::NewTokenAssetRegistered(token_id, gateway_id));

            Ok(())
        }

        // Link existing token to a gateway. Assume that the token is already registered in the assets overlay via register_new_token
        fn link_token_to_gateway(
            token_id: AssetId,
            gateway_id: TargetId,
            token_props: TokenInfo,
        ) -> DispatchResult {
            // fetch record and ensure it exists
            let _record =
                <Gateways<T>>::get(gateway_id).ok_or(Error::<T>::GatewayRecordNotFound)?;

            // early exit if record already exists in storage
            if <Tokens<T>>::contains_key(token_id, gateway_id) {
                return Err(Error::<T>::TokenRecordAlreadyExists.into())
            }

            ensure!(
                T::AssetsOverlay::contains_asset(&token_id),
                Error::<T>::TokenRecordNotFoundInAssetsOverlay
            );

            Self::override_token(token_id, gateway_id, token_props)
        }

        fn override_token(
            token_id: AssetId,
            gateway_id: TargetId,
            token_props: TokenInfo,
        ) -> DispatchResult {
            <Tokens<T>>::insert(
                token_id,
                gateway_id,
                TokenRecord {
                    token_id,
                    gateway_id,
                    token_props,
                },
            );

            <GatewayTokens<T>>::mutate(gateway_id, |tokens| {
                if !tokens.contains(&token_id) {
                    tokens.push(token_id);
                }
            });

            // Make sure that the token is added to the list of all tokens
            if !<AllTokenIds<T>>::get().contains(&token_id) {
                <AllTokenIds<T>>::append(token_id);
            }

            Self::deposit_event(Event::<T>::NewTokenLinkedToGateway(token_id, gateway_id));
            Ok(())
        }

        fn list_available_mint_assets(gateway_id: TargetId) -> Vec<TokenRecord> {
            let mut available_assets = Vec::new();
            for (asset, target) in <AuthorizedMintAssets<T>>::get() {
                if let Some(linked_asset) = <Tokens<T>>::get(asset, gateway_id) {
                    if target == gateway_id {
                        available_assets.push(linked_asset);
                    }
                }
            }
            available_assets
        }

        fn check_asset_is_mintable(gateway_id: TargetId, asset_id: AssetId) -> bool {
            Self::list_available_mint_assets(gateway_id)
                .iter()
                .any(|token| token.token_id == asset_id)
        }

        fn get_token_by_eth_address(
            gateway_id: TargetId,
            eth_address: sp_core::H160,
        ) -> Result<TokenRecord, DispatchError> {
            for token in <GatewayTokens<T>>::get(gateway_id) {
                let token_record = <Tokens<T>>::get(token, gateway_id)
                    .ok_or(Error::<T>::TokenRecordNotFoundInGateway)?;
                match token_record.token_props {
                    TokenInfo::Ethereum(ref eth_token) =>
                        if eth_token.address == Some(eth_address.into()) {
                            return Ok(token_record.clone())
                        },
                    TokenInfo::Substrate(_) =>
                        return Err(Error::<T>::TokenRecordNotFoundInGateway.into()),
                }
            }
            Err(Error::<T>::TokenRecordNotFoundInGateway.into())
        }

        fn add_new_gateway(
            gateway_id: TargetId,
            verification_vendor: GatewayVendor,
            execution_vendor: ExecutionVendor,
            codec: Codec,
            registrant: Option<T::AccountId>,
            escrow_account: Option<T::AccountId>,
            allowed_side_effects: Vec<(TargetId, Option<u8>)>,
        ) -> DispatchResult {
            // early exit if record already exists in storage
            if <Gateways<T>>::contains_key(gateway_id) {
                return Err(Error::<T>::GatewayRecordAlreadyExists.into())
            }

            Self::override_gateway(
                gateway_id,
                verification_vendor,
                execution_vendor,
                codec,
                registrant,
                escrow_account,
                allowed_side_effects,
            )
        }

        fn override_gateway(
            gateway_id: TargetId,
            verification_vendor: GatewayVendor,
            execution_vendor: ExecutionVendor,
            codec: Codec,
            registrant: Option<T::AccountId>,
            escrow_account: Option<T::AccountId>,
            allowed_side_effects: Vec<(TargetId, Option<u8>)>,
        ) -> DispatchResult {
            // Populate standard side effect ABI registry
            for (sfx_4b_id, maybe_event_memo_prefix) in allowed_side_effects.iter() {
                match <StandardSFXABIs<T>>::get(sfx_4b_id) {
                    Some(mut abi) => {
                        abi.maybe_prefix_memo = *maybe_event_memo_prefix;
                        <SFXABIRegistry<T>>::insert(gateway_id, sfx_4b_id, abi)
                    },
                    None => {
                        let _sfx_4b_str = sp_std::str::from_utf8(sfx_4b_id.as_slice())
                            .unwrap_or("invalid utf8 4b sfx id format");
                        log::error!(
                            "ABI not found for {:?}; override_gateway failed.",
                            sfx_4b_id
                        );
                        return Err(Error::<T>::SideEffectABINotFound.into())
                    },
                }
            }
            <Gateways<T>>::insert(
                gateway_id,
                GatewayRecord {
                    gateway_id,
                    verification_vendor,
                    execution_vendor,
                    codec,
                    registrant,
                    escrow_account,
                    allowed_side_effects,
                },
            );
            <AllGatewayIds<T>>::mutate(|ids| {
                ids.iter()
                    .position(|&id| id == gateway_id)
                    .map(|i| ids.remove(i));
                ids.push(gateway_id);
            });
            Self::deposit_event(Event::<T>::GatewayRecordStored(gateway_id));

            Ok(())
        }

        fn extend_sfx_abi(
            origin: OriginFor<T>,
            gateway_id: ChainId,
            sfx_4b_id: Sfx4bId,
            sfx_expected_abi: SFXAbi,
        ) -> DispatchResult {
            ensure_root(origin)?;
            if !<Gateways<T>>::contains_key(gateway_id) {
                return Err(Error::<T>::XdnsRecordNotFound.into())
            }

            <SFXABIRegistry<T>>::mutate(gateway_id, sfx_4b_id, |sfx_abi| match sfx_abi {
                Some(_) => Err(Error::<T>::SideEffectABIAlreadyExists),
                None => {
                    *sfx_abi = Some(sfx_expected_abi);
                    Ok(())
                },
            })?;

            Ok(())
        }

        fn override_sfx_abi(
            gateway_id: ChainId,
            new_sfx_abis: Vec<(Sfx4bId, SFXAbi)>,
        ) -> DispatchResult {
            if !<Gateways<T>>::contains_key(gateway_id) {
                return Err(Error::<T>::XdnsRecordNotFound.into())
            }
            // mutate allowed side effects field in gateway record
            let mut gateway_record =
                <Gateways<T>>::get(gateway_id).ok_or(Error::<T>::XdnsRecordNotFound)?;

            gateway_record.allowed_side_effects = new_sfx_abis
                .iter()
                .map(|(sfx_4b_id, abi)| (*sfx_4b_id, abi.maybe_prefix_memo))
                .collect();

            <Gateways<T>>::insert(gateway_id, gateway_record);

            for (sfx_4b_id, sfx_expected_abi) in new_sfx_abis {
                <SFXABIRegistry<T>>::mutate(gateway_id, sfx_4b_id, |sfx_abi| {
                    *sfx_abi = Some(sfx_expected_abi);
                });
            }

            Ok(())
        }

        fn get_all_sfx_abi(gateway_id: &ChainId) -> Vec<(Sfx4bId, SFXAbi)> {
            <SFXABIRegistry<T>>::iter_prefix(gateway_id)
                .map(|(sfx_4b_id, sfx_abi)| (sfx_4b_id, sfx_abi))
                .collect()
        }

        fn get_sfx_abi(gateway_id: &ChainId, sfx_4b_id: Sfx4bId) -> Option<SFXAbi> {
            <SFXABIRegistry<T>>::get(gateway_id, sfx_4b_id)
        }

        fn add_escrow_account(
            origin: OriginFor<T>,
            gateway_id: ChainId,
            escrow_account: T::AccountId,
        ) -> DispatchResult {
            ensure_root(origin)?;

            Gateways::<T>::mutate(gateway_id, |gateway| match gateway {
                None => Err(Error::<T>::GatewayRecordNotFound),
                Some(record) => {
                    record.escrow_account = Some(escrow_account);
                    Ok(())
                },
            })?;

            Ok(())
        }

        /// returns a mapping of all allowed side_effects of a gateway.
        fn allowed_side_effects(gateway_id: &ChainId) -> Vec<(Sfx4bId, Option<u8>)> {
            match <Gateways<T>>::get(gateway_id) {
                Some(gateway) => gateway.allowed_side_effects,
                None => Vec::new(),
            }
        }

        // todo: this must be removed and functionality replaced
        fn get_gateway_max_security_lvl(chain_id: &ChainId) -> SecurityLvl {
            if chain_id == &[3u8; 4] {
                return SecurityLvl::Escrow
            }

            match Self::get_escrow_account(chain_id) {
                Ok(_) => SecurityLvl::Escrow,
                Err(_) => SecurityLvl::Optimistic,
            }
        }

        /// returns the gateway vendor of a gateway if its available
        fn get_verification_vendor(chain_id: &ChainId) -> Result<GatewayVendor, DispatchError> {
            match <Gateways<T>>::get(chain_id) {
                Some(rec) => Ok(rec.verification_vendor),
                None => Err(Error::<T>::XdnsRecordNotFound.into()),
            }
        }

        fn get_target_codec(chain_id: &ChainId) -> Result<Codec, DispatchError> {
            match <Gateways<T>>::get(chain_id) {
                Some(rec) => Ok(rec.codec),
                None => Err(Error::<T>::XdnsRecordNotFound.into()),
            }
        }

        fn get_escrow_account(chain_id: &ChainId) -> Result<Bytes, DispatchError> {
            match <Gateways<T>>::get(chain_id) {
                Some(rec) => match rec.escrow_account {
                    Some(account) => Ok(account.encode()),
                    None => Err(Error::<T>::EscrowAccountNotFound.into()),
                },
                None => Err(Error::<T>::XdnsRecordNotFound.into()),
            }
        }

        fn fetch_full_gateway_records() -> Vec<FullGatewayRecord<T::AccountId>> {
            Gateways::<T>::iter_values()
                .map(|gateway| {
                    let tokens = Tokens::<T>::iter_values()
                        .filter(|token| token.gateway_id == gateway.gateway_id)
                        .collect();
                    FullGatewayRecord {
                        gateway_record: gateway,
                        tokens,
                    }
                })
                .collect()
        }

        fn read_last_activity(gateway_id: ChainId) -> Option<GatewayActivity<BlockNumberFor<T>>> {
            Self::read_last_activity_overview()
                .into_iter()
                .find(|activity| activity.gateway_id == gateway_id)
        }

        fn read_last_activity_overview() -> Vec<GatewayActivity<BlockNumberFor<T>>> {
            let mut overview = <GatewaysOverviewStore<T>>::get();
            // get the latest update
            let latest_update = overview
                .iter()
                .max_by_key(|activity| activity.reported_at)
                .map(|activity| activity.reported_at)
                .unwrap_or_default();

            if latest_update != <frame_system::Pallet<T>>::block_number() {
                Self::process_overview(<frame_system::Pallet<T>>::block_number());
                overview = <GatewaysOverviewStore<T>>::get();
            }

            overview
        }

        fn is_target_active(gateway_id: TargetId, security_lvl: &SecurityLvl) -> bool {
            match Self::read_last_activity(gateway_id) {
                Some(activity) => activity.security_lvl >= *security_lvl && activity.is_active,
                None => false,
            }
        }

        fn get_remote_order_contract_address(gateway_id: TargetId) -> Result<H256, DispatchError> {
            <RemoteOrderAddresses<T>>::get(gateway_id)
                .ok_or(Error::<T>::RemoteOrderAddressNotFound.into())
        }

        fn mint(asset_id: AssetId, user: T::AccountId, amount: BalanceOf<T>) -> DispatchResult {
            assert!(
                Self::check_asset_is_mintable(T::SelfGatewayId::get(), asset_id),
                "Asset is not mintable"
            );
            log::debug!(
                "attempt of minting asset: {:?} for user: {:?} with amount: {:?} on chain: {:?}",
                asset_id,
                user,
                amount,
                T::SelfGatewayId::get()
            );
            T::AssetsOverlay::mint(
                T::RuntimeOrigin::from(frame_system::RawOrigin::Signed(
                    T::TreasuryAccounts::get_treasury_account(TreasuryAccount::Escrow),
                )),
                asset_id,
                user,
                amount,
            )
        }

        fn burn(asset_id: AssetId, user: T::AccountId, amount: BalanceOf<T>) -> DispatchResult {
            assert!(
                Self::check_asset_is_mintable(T::SelfGatewayId::get(), asset_id),
                "Asset is not mintable"
            );
            T::AssetsOverlay::burn(
                T::RuntimeOrigin::from(frame_system::RawOrigin::Signed(
                    T::TreasuryAccounts::get_treasury_account(TreasuryAccount::Escrow),
                )),
                asset_id,
                user,
                amount,
            )
        }

        fn verify_active(
            gateway_id: &ChainId,
            max_acceptable_heartbeat_offset: frame_system::pallet_prelude::BlockNumberFor<T>,
            security_lvl: &SecurityLvl,
        ) -> Result<LightClientHeartbeat<T>, DispatchError> {
            let heartbeat = T::Portal::get_latest_heartbeat(gateway_id)
                .map_err(|_| Error::<T>::GatewayNotActive)?;

            if heartbeat.is_halted
                || !heartbeat.ever_initialized
                || max_acceptable_heartbeat_offset
                    > frame_system::Pallet::<T>::block_number()
                        .saturating_sub(heartbeat.last_heartbeat)
            {
                return Err(Error::<T>::GatewayNotActive.into())
            }

            if security_lvl == &SecurityLvl::Escrow {
                T::AttestersRead::get_activated_targets()
                    .iter()
                    .find(|target| target == &gateway_id)
                    .ok_or(Error::<T>::GatewayNotActive)?;
            }

            Ok(heartbeat)
        }

        fn get_slowest_verifier_target(
            all_targets: Vec<TargetId>,
            speed_mode: &SpeedMode,
            emergency_offset: frame_system::pallet_prelude::BlockNumberFor<T>,
        ) -> Option<(
            GatewayVendor,
            TargetId,
            frame_system::pallet_prelude::BlockNumberFor<T>,
            frame_system::pallet_prelude::BlockNumberFor<T>,
        )> {
            // map all targets to their respective vendors, and collect them into a BTreeSet to eliminate duplicates
            let all_distinct_verifiers: BTreeSet<_> = all_targets
                .iter()
                .filter_map(|target| {
                    let vendor = Self::get_verification_vendor(target);
                    vendor.ok().map(|vendor| (vendor, *target))
                })
                .collect();

            all_distinct_verifiers
                .iter()
                .map(|(vendor, target)| {
                    let epoch_history = <EpochHistory<T>>::get(vendor);
                    let (local_offset, remote_offset) =
                        vendor.calculate_offsets(speed_mode, emergency_offset, epoch_history);
                    (vendor.clone(), *target, local_offset, remote_offset)
                })
                .max_by_key(|(_, _, submit_by_local_offset, _)| *submit_by_local_offset)
        }

        fn estimate_adaptive_timeout_on_slowest_target(
            all_targets: Vec<TargetId>,
            speed_mode: &SpeedMode,
            emergency_offset: frame_system::pallet_prelude::BlockNumberFor<T>,
        ) -> AdaptiveTimeout<frame_system::pallet_prelude::BlockNumberFor<T>, TargetId> {
            let current_block = <frame_system::Pallet<T>>::block_number();

            let (slowest_verifier, target, submit_by_local_offset, submit_by_remote_offset) =
                match Self::get_slowest_verifier_target(all_targets, speed_mode, emergency_offset) {
                    Some(values) => values,
                    None => return AdaptiveTimeout::default_401(),
                };

            let latest_overview_of_verifier =
                match <VerifierOverviewStoreHistory<T>>::get(slowest_verifier).last() {
                    Some(overview) => overview.clone(),
                    None => return AdaptiveTimeout::default_401(),
                };

            let submit_by_height_here = current_block.saturating_add(submit_by_local_offset);
            let submit_by_height_there = latest_overview_of_verifier
                .finalized_height
                .saturating_add(submit_by_remote_offset);
            let estimated_height_here =
                submit_by_height_here.saturating_add(submit_by_local_offset);
            let estimated_height_there =
                submit_by_height_there.saturating_add(submit_by_remote_offset);

            AdaptiveTimeout {
                estimated_height_here,
                estimated_height_there,
                submit_by_height_here,
                submit_by_height_there,
                there: target,
                emergency_timeout_here: emergency_offset.saturating_add(current_block),
                dlq: None,
            }
        }

        fn estimate_costs(
            _fsx: &Vec<
                FullSideEffect<
                    T::AccountId,
                    frame_system::pallet_prelude::BlockNumberFor<T>,
                    BalanceOf<T>,
                >,
            >,
        ) {
            todo!("estimate costs")
        }
    }
}
