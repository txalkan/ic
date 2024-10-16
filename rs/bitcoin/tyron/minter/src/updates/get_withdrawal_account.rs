use ic_base_types::PrincipalId;
use ic_crypto_sha2::Sha256;
use icrc_ledger_types::icrc1::account::{Account, Subaccount, DEFAULT_SUBACCOUNT};

use super::get_btc_address::init_ecdsa_public_key;

/// Deterministically computes a ckBTC Ledger account ID based on the ckBTC Minter’s principal ID and the caller’s principal ID.
pub async fn get_withdrawal_account() -> Account {
    init_ecdsa_public_key().await;
    let minter = ic_cdk::id();
    let subaccount: Subaccount = compute_subaccount(0, ""); // @review (burn)
    // Check that the computed subaccount doesn't collide with minting account.
    if &subaccount == DEFAULT_SUBACCOUNT {
        panic!(
            "Subaccount collision with principal {}. Please contact DFINITY support.",
            minter
        );
    }
    Account {
        owner: minter,
        subaccount: Some(subaccount),
    }
}

/// Compute the subaccount of the minter based on a given nonce and SSI
pub fn compute_subaccount(nonce: u64, ssi: &str) -> Subaccount {
    let minter = PrincipalId(ic_cdk::id());
    const DOMAIN: &[u8] = b"syron";
    const DOMAIN_LENGTH: [u8; 1] = [0x05];

    let mut hasher = Sha256::new();
    hasher.write(&DOMAIN_LENGTH);
    hasher.write(DOMAIN);
    hasher.write(minter.as_slice());
    hasher.write(&nonce.to_be_bytes());
    hasher.write(ssi.to_string().into_bytes().as_slice());
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use crate::updates::get_withdrawal_account::compute_subaccount;
    use ic_base_types::PrincipalId;
    use std::str::FromStr;

    #[test]
    fn test_compute_subaccount() {
        let pid: PrincipalId = PrincipalId::from_str("2chl6-4hpzw-vqaaa-aaaaa-c").unwrap();
        let expected: [u8; 32] = [
            211, 145, 143, 138, 238, 246, 17, 130, 84, 217, 3, 153, 163, 32, 123, 31, 160, 98, 150,
            15, 94, 27, 22, 100, 63, 46, 142, 251, 144, 173, 213, 69,
        ];
        assert_eq!(expected, compute_subaccount(0, "")); //@review (burn)
    }
}
