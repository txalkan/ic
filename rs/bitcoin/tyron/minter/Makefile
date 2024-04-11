.PHONY: all
all: clean reinstall minter_info

# ----
# export PRINCIPAL=$(dfx identity get-principal)
# ----

.PHONY: ledgers
.SILENT: ledgers
ledgers:
	dfx deploy --ic icrc1_ledger_syron_btc --argument '(variant { Init = record { token_symbol = "BTC"; token_name = "BTC Syron Ledger"; decimals = opt 8; minting_account = record { owner = principal "ehubr-iyaaa-aaaap-ab3sq-cai" }; transfer_fee = 0; metadata = vec {}; feature_flags = opt record { icrc2 = true }; initial_balances = vec { record { record { owner = principal "ehubr-iyaaa-aaaap-ab3sq-cai"; }; 2_100_000_000_000_000; }; }; archive_options = record { num_blocks_to_archive = 1000; trigger_threshold = 2000; controller_id = principal "$(PRINCIPAL)"; cycles_for_archive_creation = opt 10000000000000 }}})'\
	&& dfx deploy --ic icrc1_ledger_syron_susd --argument '(variant { Init = record { token_symbol = "SU$D"; token_name = "Syron US Dollar"; decimals = opt 8; minting_account = record { owner = principal "ehubr-iyaaa-aaaap-ab3sq-cai" }; transfer_fee = 0; metadata = vec {}; feature_flags = opt record { icrc2 = true }; initial_balances = vec { record { record { owner = principal "ehubr-iyaaa-aaaap-ab3sq-cai"; }; 10_000_000_000_000_000_000; }; }; archive_options = record { num_blocks_to_archive = 1000; trigger_threshold = 2000; controller_id = principal "$(PRINCIPAL)"; cycles_for_archive_creation = opt 10000000000000 }}})'

.PHONY: reinstall_ledgers
.SILENT: reinstall_ledgers
reinstall_ledgers:
	#make clean
	dfx canister --ic install --mode=reinstall \
	icrc1_ledger_syron_btc --argument '(variant { Init = record { token_symbol = "BTC"; token_name = "BTC Syron Ledger"; decimals = opt 8; minting_account = record { owner = principal "$(PRINCIPAL)" }; transfer_fee = 0; metadata = vec {}; feature_flags = opt record { icrc2 = true }; initial_balances = vec { record { record { owner = principal "ehubr-iyaaa-aaaap-ab3sq-cai"; }; 2_100_000_000_000_000; }; }; archive_options = record { num_blocks_to_archive = 1000; trigger_threshold = 2000; controller_id = principal "$(PRINCIPAL)"; cycles_for_archive_creation = opt 10000000000000 }}})'\
	&& dfx canister --ic install --mode=reinstall \
	icrc1_ledger_syron_susd --argument '(variant { Init = record { token_symbol = "SU$D"; token_name = "Syron US Dollar"; decimals = opt 8; minting_account = record { owner = principal "$(PRINCIPAL)" }; transfer_fee = 0; metadata = vec {}; feature_flags = opt record { icrc2 = true }; initial_balances = vec { record { record { owner = principal "ehubr-iyaaa-aaaap-ab3sq-cai"; }; 10_000_000_000_000_000_000; }; }; archive_options = record { num_blocks_to_archive = 1000; trigger_threshold = 2000; controller_id = principal "$(PRINCIPAL)"; cycles_for_archive_creation = opt 10000000000000 }}})'

# ----
# export BTC_LEDGER=$(dfx canister id --ic icrc1_ledger_syron_btc) SUSD_LEDGER=$(dfx canister id --ic icrc1_ledger_syron_susd)
# ----

.PHONY: deploy
.SILENT: deploy
deploy:
	dfx deploy --ic ic_ckbtc_minter_tyron --argument '(variant { Init = record { mode = variant { GeneralAvailability }; btc_network = variant { Testnet }; ledger_id = principal "$(BTC_LEDGER)"; susd_id = principal "$(SUSD_LEDGER)"; xrc_id = principal "uf6dk-hyaaa-aaaaq-qaaaq-cai"; ecdsa_key_name = "test_key_1"; min_confirmations = opt 1; retrieve_btc_min_amount = 600; max_time_in_queue_nanos = 600_000_000_000 } })'

.PHONY: vault
.SILENT: vault
vault:
	dfx canister --ic call ic_ckbtc_minter_tyron get_btc_address "(record { ssi=\"$(SSI)\";})"
	
.PHONY: susd
.SILENT: susd
susd:
	dfx canister --ic call ic_ckbtc_minter_tyron update_balance "(record { ssi=\"$(SSI)\";})"

.PHONY: withdrawal_account
.SILENT: withdrawal_account
withdrawal_account:
	dfx canister --ic call ic_ckbtc_minter_tyron get_withdrawal_account
# "(record { ssi=\"$(SSI)\";})"

.PHONY: fees
.SILENT: fees
fees : # estimate_withdrawal_fee : (record { amount : opt nat64 }) -> (record { bitcoin_fee : nat64; minter_fee : nat64 }) query;
	dfx canister --ic call ic_ckbtc_minter_tyron estimate_withdrawal_fee "(record { amount=null;})"

.PHONY: info
.SILENT: info
info:
	dfx canister --ic call ic_ckbtc_minter_tyron get_minter_info

.PHONY: status
.SILENT: status
status:
	dfx canister --ic call ic_ckbtc_minter_tyron get_canister_status

.PHONY: reinstall
.SILENT: reinstall
reinstall:
	dfx canister --ic install --mode=reinstall ic_ckbtc_minter_tyron --argument '(variant { Init = record { mode = variant { GeneralAvailability }; btc_network = variant { Testnet }; ledger_id = principal "$(BTC_LEDGER)"; susd_id = principal "$(SUSD_LEDGER)"; xrc_id = principal "uf6dk-hyaaa-aaaaq-qaaaq-cai"; ecdsa_key_name = "test_key_1"; min_confirmations = opt 1; retrieve_btc_min_amount = 1_000; max_time_in_queue_nanos = 600_000_000_000 } })'
# ;kyt_principal = opt principal "$(PRINCIPAL)"

.PHONY: clean
.SILENT: clean
clean:
	rm -rf .dfx
	cargo clean
	dfx build --ic

.PHONY: bal_btc_minter
.SILENT: bal_btc_minter
bal_btc_minter:
	dfx canister --ic call icrc1_ledger_syron_btc icrc1_balance_of "(record { owner = principal \"ehubr-iyaaa-aaaap-ab3sq-cai\" })"

.PHONY: bal_susd_minter
.SILENT: bal_susd_minter
bal_susd_minter:
	dfx canister --ic call icrc1_ledger_syron_susd icrc1_balance_of "(record { owner = principal \"ehubr-iyaaa-aaaap-ab3sq-cai\" })"

.PHONY: bal_susd
.SILENT: bal_susd
bal_susd:
	dfx canister --ic call icrc1_ledger_syron_susd icrc1_balance_of "(record { owner = principal \
	\"ehubr-iyaaa-aaaap-ab3sq-cai\"; \
	subaccount = opt blob \"\6d\60\70\a6\83\8e\5a\d4\0c\12\e5\ee\d2\52\93\dd\3e\29\aa\66\75\0a\72\b1\f6\77\3a\94\67\38\57\0a\" })"
#\"$(PRINCIPAL)\"; \

.PHONY: bal_btc
.SILENT: bal_btc
bal_btc:
	dfx canister --ic call icrc1_ledger_syron_btc icrc1_balance_of "(record { owner = principal \
	\"ehubr-iyaaa-aaaap-ab3sq-cai\"; \
	subaccount = opt blob \"\6d\60\70\a6\83\8e\5a\d4\0c\12\e5\ee\d2\52\93\dd\3e\29\aa\66\75\0a\72\b1\f6\77\3a\94\67\38\57\0a\" })"

.PHONY: cy
.SILENT: cy
cy:
	dfx canister --ic call gyjkd-saaaa-aaaap-abxra-cai wallet_balance

.PHONY: subaccount
.SILENT: subaccount
subaccount:
	dfx canister --ic call ic_ckbtc_minter_tyron get_subaccount "( \"$(SSI)\" )"

.PHONY: transfer_btc
.SILENT: transfer_btc
transfer_btc:
	dfx canister --ic call icrc1_ledger_syron_btc icrc1_transfer "(record { \
	to = record { owner = principal \"ehubr-iyaaa-aaaap-ab3sq-cai\"; subaccount = opt blob \"\6d\60\70\a6\83\8e\5a\d4\0c\12\e5\ee\d2\52\93\dd\3e\29\aa\66\75\0a\72\b1\f6\77\3a\94\67\38\57\0a\" }; \
	amount = 1_000 })"

#from_subaccount = opt blob \"\6d\60\70\a6\83\8e\5a\d4\0c\12\e5\ee\d2\52\93\dd\3e\29\aa\66\75\0a\72\b1\f6\77\3a\94\67\38\57\0a\"; \

.PHONY: minter_info
.SILENT: minter_info
minter_info:
	dfx canister --ic call ic_ckbtc_minter_tyron get_minter_info