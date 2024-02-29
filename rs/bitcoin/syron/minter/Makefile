.PHONY: all
all: ledgers deploy vault susd withdrawal_account info status reinstall

# ----
# export PRINCIPAL=$(dfx identity get-principal)
# ----
.PHONY: ledgers
.SILENT: ledgers
ledgers:
	dfx deploy --ic icrc1_ledger_syron_btc --argument '(variant { Init = record { token_symbol = "BTC"; token_name = "BTC Syron Ledger"; minting_account = record { owner = principal "$(PRINCIPAL)" }; transfer_fee = 0; metadata = vec {}; feature_flags = opt record { icrc2 = true }; initial_balances = vec { record { record { owner = principal "$(PRINCIPAL)"; }; 2_100_000_000_000_000; }; }; archive_options = record { num_blocks_to_archive = 1000; trigger_threshold = 2000; controller_id = principal "$(PRINCIPAL)"; cycles_for_archive_creation = opt 10000000000000 }}})' \
	&& dfx deploy --ic icrc1_ledger_syron_susd --argument '(variant { Init = record { token_symbol = "SU$D"; token_name = "Syron US Dollar"; minting_account = record { owner = principal "$(PRINCIPAL)" }; transfer_fee = 0; metadata = vec {}; feature_flags = opt record { icrc2 = true }; initial_balances = vec { record { record { owner = principal "$(PRINCIPAL)"; }; 10_000_000_000_000_000_000; }; }; archive_options = record { num_blocks_to_archive = 1000; trigger_threshold = 2000; controller_id = principal "$(PRINCIPAL)"; cycles_for_archive_creation = opt 10000000000000 }}})' \

# ----
# export BTC_LEDGER=$(dfx canister id --ic icrc1_ledger_syron_btc) SUSD_LEDGER=$(dfx canister id --ic icrc1_ledger_syron_susd) SSI=tb1pjnr5curwpqcxhyxjfcqmya3ms48s7ca7erd6uxwx6dp3svunq2wsq4me33
# ----

.PHONY: deploy
.SILENT: deploy
deploy:
	dfx deploy --ic ic_ckbtc_minter_syron --argument '(variant { Init = record { mode = variant { GeneralAvailability }; btc_network = variant { Testnet }; ledger_id = principal "$(BTC_LEDGER)"; susd_id = principal "$(SUSD_LEDGER)"; ecdsa_key_name = "test_key_1"; min_confirmations = opt 1; retrieve_btc_min_amount = 100_000; max_time_in_queue_nanos = 600_000_000_000 } })'

# ---- Use your own Bitcoin address
# export SSI=tb1pjnr5curwpqcxhyxjfcqmya3ms48s7ca7erd6uxwx6dp3svunq2wsq4me33
# ----

.PHONY: vault
.SILENT: vault
vault:
	dfx canister --ic call ic_ckbtc_minter_syron get_btc_address "(record { ssi=\"$(SSI)\";})"
	
.PHONY: susd
.SILENT: susd
susd:
	dfx canister --ic call ic_ckbtc_minter_syron update_balance "(record { ssi=\"$(SSI)\";})"

.PHONY: withdrawal_account
.SILENT: withdrawal_account
withdrawal_account:
	dfx canister --ic call ic_ckbtc_minter_syron get_withdrawal_account
	# "(record { ssi=\"$(SSI)\";})"

.PHONY: fees
.SILENT: fees
fees : # estimate_withdrawal_fee : (record { amount : opt nat64 }) -> (record { bitcoin_fee : nat64; minter_fee : nat64 }) query;
	dfx canister --ic call ic_ckbtc_minter_syron estimate_withdrawal_fee "(record { amount=null;})"

.PHONY: info
.SILENT: info
info:
	dfx canister --ic call ic_ckbtc_minter_syron get_minter_info

.PHONY: status
.SILENT: status
status:
	dfx canister --ic call ic_ckbtc_minter_syron get_canister_status

.PHONY: reinstall
.SILENT: reinstall
reinstall:
	dfx canister --ic install --mode=reinstall ic_ckbtc_minter_syron --argument '(variant { Init = record { mode = variant { GeneralAvailability }; btc_network = variant { Testnet }; ledger_id = principal "$(BTC_LEDGER)"; susd_id = principal "$(SUSD_LEDGER)"; ecdsa_key_name = "test_key_1"; min_confirmations = opt 1; retrieve_btc_min_amount = 100_000; max_time_in_queue_nanos = 600_000_000_000 } })'

.PHONY: clean
.SILENT: clean
clean:
	rm -rf .dfx
	cargo clean
