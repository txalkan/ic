```markdown
# ckBTC Canister Upgrade Protocol Summary

The ckBTC canisters (Minter and Ledger) are upgraded via the standard NNS `ExecuteNnsFunction` proposal mechanism, targeting `NnsFunction::NnsCanisterUpgrade`. This triggers the Lifeline canister to perform the upgrade.

## Key Protocol Steps & Characteristics:

1.  **Proposal:**
    *   A standard NNS proposal (`ExecuteNnsFunction`) is prepared.
    *   **Payload:** An encoded `ic_management_canister_types::ChangeCanisterRequest` containing:
        *   `target_canister_id`: The ID of the ckBTC canister (Minter or Ledger) to be upgraded.
        *   `module_wasm`: The new Wasm bytecode for the canister.
        *   `arg`: Candid-encoded arguments for the `post_upgrade` hook. For the ckBTC Minter, this is `opt UpgradeArgs` wrapped in a `MinterArg::Upgrade` variant, allowing for optional configuration updates during the upgrade.
        *   `mode`: `CanisterInstallMode::Upgrade`.
    *   Proposal generation often utilizes scripts like `prepare-nns-upgrade-proposal-text.sh` and `ic-admin` for submission.

2.  **Pre-Upgrade Hook (`pre_upgrade`):**
    *   The ckBTC Minter canister notably **does not** define an explicit `pre_upgrade` hook for full state serialization to stable memory. Persistence relies on its event log being durably stored.
    *   The ckBTC Ledger, if utilizing `StableBTreeMap` or similar stable structures, may also have minimal or no explicit state serialization in `pre_upgrade`.

3.  **Post-Upgrade Hook (`post_upgrade`):**
    *   **ckBTC Minter:**
        *   Accepts optional `UpgradeArgs` for configuration parameter updates.
        *   Reconstructs its entire runtime state by replaying its event log, which is expected to be persisted in stable memory. This event sourcing approach is central to its upgrade integrity.
        *   Validates the reconstructed state's configuration.
    *   **ckBTC Ledger:** The `post_upgrade` hook would primarily ensure stable data structures are correctly re-initialized or accessible.

4.  **State Management & Integrity:**
    *   **Minter:** State integrity hinges on the correctness and completeness of the event log and the replay logic's idempotency and backward compatibility.
    *   **Ledger:** Typically relies on stable memory data structures (e.g., `StableBTreeMap`) for persistent state, simplifying state management across upgrades.

5.  **Tooling:**
    *   Standard IC development tools: `git`, build systems (e.g., Bazel via `build-ic.sh`), `sha256sum`.
    *   Candid interaction: `didc` for encoding `post_upgrade` arguments.
    *   NNS interaction: `ic-admin` for proposal submission, often orchestrated by helper scripts in `testnet/tools/nns-tools/`.

## Specific Considerations for Protocol Engineers:

*   **Event Replay (Minter):** The efficiency and correctness of the event replay in `post_upgrade` are critical. Potential for long replay times with growing event logs must be managed, possibly through future optimizations or implicit snapshotting if event log reads become chunked/paged.
*   **Upgrade Arguments (Minter):** The `UpgradeArgs` provide a mechanism for controlled evolution of canister parameters without requiring a separate proposal. The encoding and handling of these arguments must be precise.
*   **Interface Stability:** Adherence to ICRC standards is crucial for interoperability, especially between the Minter and Ledger.
*   **Resource Limits:** `post_upgrade` execution, especially event replay, must operate within canister instruction and cycle limits.
*   **Verification:** Post-upgrade verification involves checking the module hash, canister status, and performing functional tests to ensure state integrity and continued service health. Past upgrade proposals (e.g., markdown files in `rs/bitcoin/ckbtc/mainnet/`) serve as valuable templates and checklists.

This protocol leverages generic NNS upgrade primitives while incorporating specific state management strategies (event sourcing for the Minter) tailored to the canisters' roles.
```
