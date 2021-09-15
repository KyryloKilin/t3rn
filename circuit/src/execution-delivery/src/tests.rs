// This file is part of Substrate.

// Copyright (C) 2019-2021 Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: Apache-2.0

// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
// 	http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! Test utilities
use crate::ExecComposer;
use crate::{self as pallet_execution_delivery};
use codec::Encode;

use frame_support::assert_err;

use crate::Xtx;
use sp_io;

use sp_core::{crypto::Pair, sr25519, Hasher};
use sp_runtime::traits::Zero;

use crate::exec_composer::tests::insert_default_xdns_record;
use sp_io::TestExternalities;
use sp_keystore::testing::KeyStore;
use sp_keystore::{KeystoreExt, SyncCryptoStore};

use pallet_execution_delivery::Compose;

use t3rn_primitives::{ExecPhase, ExecStep, InterExecSchedule};

use crate::mock::*;

use crate::mock::AccountId;

#[test]
fn it_submits_empty_composable_exec_request() {
    sp_io::TestExternalities::default().execute_with(|| {
        assert_err!(
            ExecDelivery::submit_composable_exec_order(
                Origin::signed(Default::default()),
                vec![],
                vec![]
            ),
            "empty parameters submitted for execution order"
        );
    });
}

#[test]
fn it_should_correctly_parse_a_minimal_valid_io_schedule() {
    let expected = InterExecSchedule {
        phases: vec![ExecPhase {
            steps: vec![ExecStep {
                compose: Compose {
                    name: b"component1".to_vec(),
                    code_txt: r#""#.as_bytes().to_vec(),
                    exec_type: b"exec_escrow".to_vec(),
                    dest: AccountId::new([1 as u8; 32]),
                    value: 0,
                    bytes: vec![],
                    input_data: vec![],
                },
            }],
        }],
    };

    let io_schedule = b"component1;".to_vec();
    let components = vec![Compose {
        name: b"component1".to_vec(),
        code_txt: r#""#.as_bytes().to_vec(),
        exec_type: b"exec_escrow".to_vec(),
        dest: AccountId::new([1 as u8; 32]),
        value: 0,
        bytes: vec![],
        input_data: vec![],
    }];

    assert_eq!(
        ExecDelivery::decompose_io_schedule(components, io_schedule).unwrap(),
        expected
    )
}

#[test]
fn it_should_correctly_parse_a_valid_io_schedule_with_2_phases() {
    let expected = InterExecSchedule {
        phases: vec![
            ExecPhase {
                steps: vec![ExecStep {
                    compose: Compose {
                        name: b"component1".to_vec(),
                        code_txt: r#""#.as_bytes().to_vec(),
                        exec_type: b"exec_escrow".to_vec(),
                        dest: AccountId::new([1 as u8; 32]),
                        value: 0,
                        bytes: vec![],
                        input_data: vec![],
                    },
                }],
            },
            ExecPhase {
                steps: vec![ExecStep {
                    compose: Compose {
                        name: b"component2".to_vec(),
                        code_txt: r#""#.as_bytes().to_vec(),
                        exec_type: b"exec_escrow".to_vec(),
                        dest: AccountId::new([1 as u8; 32]),
                        value: 0,
                        bytes: vec![],
                        input_data: vec![],
                    },
                }],
            },
        ],
    };

    let io_schedule = b"component1 | component2;".to_vec();
    let components = vec![
        Compose {
            name: b"component1".to_vec(),
            code_txt: r#""#.as_bytes().to_vec(),

            exec_type: b"exec_escrow".to_vec(),
            dest: AccountId::new([1 as u8; 32]),
            value: 0,
            bytes: vec![],
            input_data: vec![],
        },
        Compose {
            name: b"component2".to_vec(),
            code_txt: r#""#.as_bytes().to_vec(),

            exec_type: b"exec_escrow".to_vec(),
            dest: AccountId::new([1 as u8; 32]),
            value: 0,
            bytes: vec![],
            input_data: vec![],
        },
    ];

    assert_eq!(
        ExecDelivery::decompose_io_schedule(components, io_schedule).unwrap(),
        expected
    )
}

#[test]
fn it_should_correctly_parse_a_valid_io_schedule_with_1_phase_and_2_steps() {
    let expected = InterExecSchedule {
        phases: vec![ExecPhase {
            steps: vec![
                ExecStep {
                    compose: Compose {
                        name: b"component1".to_vec(),
                        code_txt: r#""#.as_bytes().to_vec(),

                        exec_type: b"exec_escrow".to_vec(),
                        dest: AccountId::new([1 as u8; 32]),
                        value: 0,
                        bytes: vec![],
                        input_data: vec![],
                    },
                },
                ExecStep {
                    compose: Compose {
                        name: b"component2".to_vec(),
                        code_txt: r#""#.as_bytes().to_vec(),

                        exec_type: b"exec_escrow".to_vec(),
                        dest: AccountId::new([1 as u8; 32]),
                        value: 0,
                        bytes: vec![],
                        input_data: vec![],
                    },
                },
            ],
        }],
    };

    let io_schedule = b"component1 , component2;".to_vec();
    let components = vec![
        Compose {
            name: b"component1".to_vec(),
            code_txt: r#""#.as_bytes().to_vec(),

            exec_type: b"exec_escrow".to_vec(),
            dest: AccountId::new([1 as u8; 32]),
            value: 0,
            bytes: vec![],
            input_data: vec![],
        },
        Compose {
            name: b"component2".to_vec(),
            code_txt: r#""#.as_bytes().to_vec(),

            exec_type: b"exec_escrow".to_vec(),
            dest: AccountId::new([1 as u8; 32]),
            value: 0,
            bytes: vec![],
            input_data: vec![],
        },
    ];

    assert_eq!(
        ExecDelivery::decompose_io_schedule(components, io_schedule).unwrap(),
        expected
    )
}

#[test]
fn it_should_correctly_parse_a_valid_io_schedule_with_complex_structure() {
    let expected = InterExecSchedule {
        phases: vec![
            ExecPhase {
                steps: vec![
                    ExecStep {
                        compose: Compose {
                            name: b"component1".to_vec(),
                            code_txt: r#""#.as_bytes().to_vec(),

                            exec_type: b"exec_escrow".to_vec(),
                            dest: AccountId::new([1 as u8; 32]),
                            value: 0,
                            bytes: vec![],
                            input_data: vec![],
                        },
                    },
                    ExecStep {
                        compose: Compose {
                            name: b"component2".to_vec(),
                            code_txt: r#""#.as_bytes().to_vec(),

                            exec_type: b"exec_escrow".to_vec(),
                            dest: AccountId::new([1 as u8; 32]),
                            value: 0,
                            bytes: vec![],
                            input_data: vec![],
                        },
                    },
                ],
            },
            ExecPhase {
                steps: vec![ExecStep {
                    compose: Compose {
                        name: b"component2".to_vec(),
                        code_txt: r#""#.as_bytes().to_vec(),

                        exec_type: b"exec_escrow".to_vec(),
                        dest: AccountId::new([1 as u8; 32]),
                        value: 0,
                        bytes: vec![],
                        input_data: vec![],
                    },
                }],
            },
            ExecPhase {
                steps: vec![ExecStep {
                    compose: Compose {
                        name: b"component1".to_vec(),
                        code_txt: r#""#.as_bytes().to_vec(),

                        exec_type: b"exec_escrow".to_vec(),
                        dest: AccountId::new([1 as u8; 32]),
                        value: 0,
                        bytes: vec![],
                        input_data: vec![],
                    },
                }],
            },
            ExecPhase {
                steps: vec![
                    ExecStep {
                        compose: Compose {
                            name: b"component2".to_vec(),
                            code_txt: r#""#.as_bytes().to_vec(),

                            exec_type: b"exec_escrow".to_vec(),
                            dest: AccountId::new([1 as u8; 32]),
                            value: 0,
                            bytes: vec![],
                            input_data: vec![],
                        },
                    },
                    ExecStep {
                        compose: Compose {
                            name: b"component2".to_vec(),
                            code_txt: r#""#.as_bytes().to_vec(),

                            exec_type: b"exec_escrow".to_vec(),
                            dest: AccountId::new([1 as u8; 32]),
                            value: 0,
                            bytes: vec![],
                            input_data: vec![],
                        },
                    },
                    ExecStep {
                        compose: Compose {
                            name: b"component1".to_vec(),
                            code_txt: r#""#.as_bytes().to_vec(),

                            exec_type: b"exec_escrow".to_vec(),
                            dest: AccountId::new([1 as u8; 32]),
                            value: 0,
                            bytes: vec![],
                            input_data: vec![],
                        },
                    },
                ],
            },
        ],
    };

    let io_schedule = b"     component1 , component2 | component2 |     component1| component2, component2, component1;   ".to_vec();
    let components = vec![
        Compose {
            name: b"component1".to_vec(),
            code_txt: r#""#.as_bytes().to_vec(),

            exec_type: b"exec_escrow".to_vec(),
            dest: AccountId::new([1 as u8; 32]),
            value: 0,
            bytes: vec![],
            input_data: vec![],
        },
        Compose {
            name: b"component2".to_vec(),
            code_txt: r#""#.as_bytes().to_vec(),

            exec_type: b"exec_escrow".to_vec(),
            dest: AccountId::new([1 as u8; 32]),
            value: 0,
            bytes: vec![],
            input_data: vec![],
        },
    ];

    assert_eq!(
        ExecDelivery::decompose_io_schedule(components, io_schedule).unwrap(),
        expected
    )
}

#[test]
fn it_should_throw_when_io_schedule_does_not_end_correctly() {
    let expected = "IOScheduleNoEndingSemicolon";

    let io_schedule = b"component1".to_vec();
    let components = vec![Compose {
        name: b"component1".to_vec(),
        code_txt: r#""#.as_bytes().to_vec(),

        exec_type: b"exec_escrow".to_vec(),
        dest: AccountId::new([1 as u8; 32]),
        value: 0,
        bytes: vec![],
        input_data: vec![],
    }];

    assert_err!(
        ExecDelivery::decompose_io_schedule(components, io_schedule),
        expected
    );
}

#[test]
fn it_should_throw_when_io_schedule_references_a_missing_component() {
    let expected = "IOScheduleUnknownCompose";

    let io_schedule = b"component1 | component2;".to_vec();
    let components = vec![Compose {
        name: b"component1".to_vec(),
        code_txt: r#""#.as_bytes().to_vec(),

        exec_type: b"exec_escrow".to_vec(),
        dest: AccountId::new([1 as u8; 32]),
        value: 0,
        bytes: vec![],
        input_data: vec![],
    }];

    assert_err!(
        ExecDelivery::decompose_io_schedule(components, io_schedule),
        expected
    );
}

#[test]
fn it_should_throw_with_empty_io_schedule() {
    let expected = "IOScheduleEmpty";

    let io_schedule = b"".to_vec();
    let components = vec![Compose {
        name: b"component1".to_vec(),
        code_txt: r#""#.as_bytes().to_vec(),

        exec_type: b"exec_escrow".to_vec(),
        dest: AccountId::new([1 as u8; 32]),
        value: 0,
        bytes: vec![],
        input_data: vec![],
    }];

    assert_err!(
        ExecDelivery::decompose_io_schedule(components, io_schedule),
        expected
    );
}

#[test]
fn test_authority_selection() {
    let keystore = KeyStore::new();

    // Insert Alice's keys
    const SURI_ALICE: &str = "//Alice";
    let key_pair_alice = sr25519::Pair::from_string(SURI_ALICE, None).expect("Generates key pair");
    SyncCryptoStore::insert_unknown(
        &keystore,
        KEY_TYPE,
        SURI_ALICE,
        key_pair_alice.public().as_ref(),
    )
    .expect("Inserts unknown key");

    // Insert Bob's keys
    const SURI_BOB: &str = "//Bob";
    let key_pair_bob = sr25519::Pair::from_string(SURI_BOB, None).expect("Generates key pair");
    SyncCryptoStore::insert_unknown(
        &keystore,
        KEY_TYPE,
        SURI_BOB,
        key_pair_bob.public().as_ref(),
    )
    .expect("Inserts unknown key");

    // Insert Charlie's keys
    const SURI_CHARLIE: &str = "//Charlie";
    let key_pair_charlie =
        sr25519::Pair::from_string(SURI_CHARLIE, None).expect("Generates key pair");
    SyncCryptoStore::insert_unknown(
        &keystore,
        KEY_TYPE,
        SURI_CHARLIE,
        key_pair_charlie.public().as_ref(),
    )
    .expect("Inserts unknown key");

    // Alice's account
    // let escrow: AccountId = hex_literal::hex!["d43593c715fdd31c61141abd04a99fd6822c8558854ccde39a5684e7a56da27d"].into();

    // Bob's account
    let escrow: AccountId =
        hex_literal::hex!["8eaf04151687736326c9fea17e25fc5287613693c912909cb226aa4794f26a48"]
            .into();
    let mut ext = TestExternalities::new_empty();
    ext.register_extension(KeystoreExt(keystore.into()));
    ext.execute_with(|| {
        let submitter = ExecDelivery::select_authority(escrow.clone());

        assert!(submitter.is_ok());
    });
}

#[test]
fn error_if_keystore_is_empty() {
    let keystore = KeyStore::new();

    // Alice's escrow account
    let escrow: AccountId =
        hex_literal::hex!["8eaf04151687736326c9fea17e25fc5287613693c912909cb226aa4794f26a48"]
            .into();

    let mut ext = TestExternalities::new_empty();
    ext.register_extension(KeystoreExt(keystore.into()));
    ext.execute_with(|| {
        let submitter = ExecDelivery::select_authority(escrow.clone());

        assert!(submitter.is_err());
    });
}

#[test]
fn error_if_incorrect_escrow_is_submitted() {
    let keystore = KeyStore::new();

    // Insert Alice's keys
    const SURI_ALICE: &str = "//Alice";
    let key_pair_alice = sr25519::Pair::from_string(SURI_ALICE, None).expect("Generates key pair");
    SyncCryptoStore::insert_unknown(
        &keystore,
        KEY_TYPE,
        SURI_ALICE,
        key_pair_alice.public().as_ref(),
    )
    .expect("Inserts unknown key");

    // Insert Bob's keys
    const SURI_BOB: &str = "//Bob";
    let key_pair_bob = sr25519::Pair::from_string(SURI_BOB, None).expect("Generates key pair");
    SyncCryptoStore::insert_unknown(
        &keystore,
        KEY_TYPE,
        SURI_BOB,
        key_pair_bob.public().as_ref(),
    )
    .expect("Inserts unknown key");

    // Insert Charlie's keys
    const SURI_CHARLIE: &str = "//Charlie";
    let key_pair_charlie =
        sr25519::Pair::from_string(SURI_CHARLIE, None).expect("Generates key pair");
    SyncCryptoStore::insert_unknown(
        &keystore,
        KEY_TYPE,
        SURI_CHARLIE,
        key_pair_charlie.public().as_ref(),
    )
    .expect("Inserts unknown key");

    // Alice's original account => d43593c715fdd31c61141abd04a99fd6822c8558854ccde39a5684e7a56da27d
    // Alice's tempered account => a51593c715fdd31c61141abd04a99fd6822c8558854ccde39a5684e7a56da27d
    // The first 3 bytes are changed, thus making the account invalid
    let escrow: AccountId =
        hex_literal::hex!["a51593c715fdd31c61141abd04a99fd6822c8558854ccde39a5684e7a56da27d"]
            .into();

    let mut ext = TestExternalities::new_empty();
    ext.register_extension(KeystoreExt(keystore.into()));
    ext.execute_with(|| {
        let submitter = ExecDelivery::select_authority(escrow.clone());

        assert!(submitter.is_err());
    });
}

const CODE_CALL: &str = r#"
(module
	;; seal_call(
	;;    callee_ptr: u32,
	;;    callee_len: u32,
	;;    gas: u64,
	;;    value_ptr: u32,
	;;    value_len: u32,
	;;    input_data_ptr: u32,
	;;    input_data_len: u32,
	;;    output_ptr: u32,
	;;    output_len_ptr: u32
	;;) -> u32
	(import "seal0" "seal_call" (func $seal_call (param i32 i32 i64 i32 i32 i32 i32 i32 i32) (result i32)))
	(import "env" "memory" (memory 1 1))
	(func (export "call")
		(drop
			(call $seal_call
				(i32.const 4)  ;; Pointer to "callee" address.
				(i32.const 32)  ;; Length of "callee" address.
				(i64.const 0)  ;; How much gas to devote for the execution. 0 = all.
				(i32.const 36) ;; Pointer to the buffer with value to transfer
				(i32.const 8)  ;; Length of the buffer with value to transfer.
				(i32.const 44) ;; Pointer to input data buffer address
				(i32.const 4)  ;; Length of input data buffer
				(i32.const 4294967295) ;; u32 max value is the sentinel value: do not copy output
				(i32.const 0) ;; Length is ignored in this case
			)
		)
	)
	(func (export "deploy"))
	;; Destination AccountId (ALICE)
	(data (i32.const 4)
		"\01\01\01\01\01\01\01\01\01\01\01\01\01\01\01\01"
		"\01\01\01\01\01\01\01\01\01\01\01\01\01\01\01\01"
	)
	;; Amount of value to transfer.
	;; Represented by u64 (8 bytes long) in little endian.
	(data (i32.const 36) "\06\00\00\00\00\00\00\00")
	(data (i32.const 44) "\01\02\03\04")
)
"#;

#[test]
fn dry_run_whole_xtx_unseen_contract_one_phase_and_one_step_success() {
    let mut ext = TestExternalities::new_empty();
    ext.execute_with(|| {
        let mut contracts = vec![];
        let mut action_descriptions = vec![];
        let mut unseen_contracts = vec![];
        let mut contract_ids = vec![];

        let inter_schedule = InterExecSchedule {
            phases: vec![ExecPhase {
                steps: vec![ExecStep {
                    compose: Compose {
                        name: b"component1".to_vec(),
                        code_txt: CODE_CALL.as_bytes().to_vec(),
                        exec_type: b"exec_escrow".to_vec(),
                        dest: AccountId::new([1 as u8; 32]),
                        value: 0,
                        bytes: vec![
                            0, 97, 115, 109, 1, 0, 0, 0, 1, 17, 2, 96, 9, 127, 127, 126, 127, 127,
                            127, 127, 127, 127, 1, 127, 96, 0, 0, 2, 34, 2, 5, 115, 101, 97, 108,
                            48, 9, 115, 101, 97, 108, 95, 99, 97, 108, 108, 0, 0, 3, 101, 110, 118,
                            6, 109, 101, 109, 111, 114, 121, 2, 1, 1, 1, 3, 3, 2, 1, 1, 7, 17, 2,
                            4, 99, 97, 108, 108, 0, 1, 6, 100, 101, 112, 108, 111, 121, 0, 2, 10,
                            28, 2, 23, 0, 65, 4, 65, 32, 66, 0, 65, 36, 65, 8, 65, 44, 65, 4, 65,
                            127, 65, 0, 16, 0, 26, 11, 2, 0, 11, 11, 60, 3, 0, 65, 4, 11, 32, 1, 1,
                            1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
                            1, 1, 1, 1, 1, 1, 0, 65, 36, 11, 8, 6, 0, 0, 0, 0, 0, 0, 0, 0, 65, 44,
                            11, 4, 1, 2, 3, 4,
                        ],
                        input_data: vec![],
                    },
                }],
            }],
        };

        let escrow_account: AccountId =
            hex_literal::hex!["8eaf04151687736326c9fea17e25fc5287613693c912909cb226aa4794f26a48"]
                .into();

        let requester: AccountId =
            hex_literal::hex!["d43593c715fdd31c61141abd04a99fd6822c8558854ccde39a5684e7a56da27d"]
                .into();

        let first_phase = inter_schedule
            .phases
            .get(0)
            .expect("At least one phase should be in inter schedule");

        let step = first_phase
            .steps
            .get(0)
            .expect("At least one step in a phase");

        insert_default_xdns_record();

        let unseen_contract =
            ExecComposer::dry_run_single_contract::<Test>(step.compose.clone()).unwrap();

        unseen_contracts.push(unseen_contract.clone());
        contracts.extend(unseen_contracts);

        action_descriptions.extend(unseen_contract.action_descriptions.clone());

        let mut protocol_part_of_contract = step.compose.code_txt.clone();
        protocol_part_of_contract.extend(step.compose.bytes.clone());

        let key = <Test as frame_system::Config>::Hashing::hash(
            Encode::encode(&mut protocol_part_of_contract).as_ref(),
        );

        contract_ids.push(key);

        let max_steps = contracts.len() as u32;

        let (current_block_no, block_zero) = (
            <frame_system::Pallet<Test>>::block_number(),
            <Test as frame_system::Config>::BlockNumber::zero(),
        );

        let expected_xtx = Xtx {
            estimated_worth: Default::default(),
            current_worth: Default::default(),
            requester: requester.clone(),
            escrow_account: escrow_account.clone(),
            payload: vec![],
            current_step: 0,
            steps_no: max_steps,
            current_phase: 0,
            current_round: 0,
            result_status: vec![],
            phases_blockstamps: (current_block_no, block_zero),
            schedule: Default::default(),
        };

        assert_eq!(
            ExecDelivery::dry_run_whole_xtx(inter_schedule, escrow_account, requester),
            Ok((expected_xtx, contracts, contract_ids, action_descriptions))
        );
    });
}
