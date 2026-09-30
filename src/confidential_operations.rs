use std::num::NonZeroI8;

use anyhow::{Context, Result, ensure};
use sha2::{Digest, Sha256};
use solana_instruction::Instruction;
use solana_pubkey::Pubkey;
use solana_rpc_client::nonblocking::rpc_client::RpcClient;
use solana_signer::Signer;
use solana_zk_sdk::{
    encryption::{
        auth_encryption::{AeCiphertext, AeKey},
        derivation::{confidential_derivation_message, derive_confidential_keys_from_ikm},
        elgamal::{ElGamalCiphertext, ElGamalKeypair, ElGamalPubkey},
    },
    zk_elgamal_proof_program::build_pubkey_validity_proof_data,
};
use spl_token_2022_interface::{
    extension::{
        BaseStateWithExtensions, ExtensionType, StateWithExtensions,
        confidential_transfer::{
            ConfidentialTransferAccount, ConfidentialTransferMint, MAXIMUM_DEPOSIT_TRANSFER_AMOUNT,
            instruction as ct,
        },
    },
    state::{Account, AccountState, Mint},
};
use spl_token_confidential_transfer_proof_extraction::instruction::ProofLocation;
use spl_token_confidential_transfer_proof_generation::{
    transfer::transfer_split_proof_data, withdraw::withdraw_proof_data,
};
use zeroize::{Zeroize, Zeroizing};

use crate::{
    config::RpcProfile, operations::PreparedTransfer, tokens::TokenAccount, transaction::Format,
    wallet::Wallet,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operation {
    Configure,
    Approve,
    Reveal,
    Deposit,
    Apply,
    Transfer,
    Withdraw,
    Lock,
}
impl Operation {
    pub fn label(self) -> &'static str {
        match self {
            Self::Configure => "Configure account",
            Self::Approve => "Approve account",
            Self::Reveal => "Reveal balances",
            Self::Deposit => "Deposit public tokens",
            Self::Apply => "Apply pending balance",
            Self::Transfer => "Send confidentially",
            Self::Withdraw => "Withdraw to public",
            Self::Lock => "Hide balances",
        }
    }
}

#[derive(Zeroize)]
pub struct Balances {
    pub public: u64,
    pub available: u64,
    pub pending: u64,
}

struct Keys {
    elgamal: Zeroizing<ElGamalKeypair>,
    aes: Zeroizing<AeKey>,
}
fn keys(wallet: &Wallet) -> Result<Keys> {
    let signer = wallet.signer()?;
    let bytes = Zeroizing::new(<[u8; 64]>::from(
        signer.try_sign_message(&confidential_derivation_message(&[]))?,
    ));
    drop(signer);
    let (elgamal, aes) = derive_confidential_keys_from_ikm(bytes.as_ref())
        .map_err(|_| anyhow::anyhow!("Cannot derive confidential keys"))?;
    Ok(Keys {
        elgamal: Zeroizing::new(elgamal),
        aes: Zeroizing::new(aes),
    })
}

fn check_keys(state: &ConfidentialTransferAccount, keys: &Keys) -> Result<()> {
    ensure!(
        state.elgamal_pubkey == keys.elgamal.pubkey().to_owned().into(),
        "This account uses different confidential keys. Solte uses the standard wallet-bound derivation; the account was not changed."
    );
    Ok(())
}
fn available(state: &ConfidentialTransferAccount, keys: &Keys) -> Result<u64> {
    check_keys(state, keys)?;
    if state.actual_pending_balance_credit_counter != state.expected_pending_balance_credit_counter
    {
        let cipher: ElGamalCiphertext = state.available_balance.try_into()?;
        return cipher.decrypt_u32(keys.elgamal.secret()).context("Credits arrived while applying. Solte cannot recover this large available balance; use a compatible confidential client to resynchronize it.");
    }
    let cipher: AeCiphertext = state
        .decryptable_available_balance
        .try_into()
        .context("Invalid encrypted available balance")?;
    cipher
        .decrypt(&keys.aes)
        .context("Cannot decrypt available balance with this wallet")
}
fn pending(state: &ConfidentialTransferAccount, keys: &Keys) -> Result<u64> {
    let decrypt = |pod: spl_token_2022_interface::extension::confidential_transfer::EncryptedBalance| -> Result<u64> {
        let cipher: ElGamalCiphertext = pod.try_into().context("Invalid pending ciphertext")?;
        cipher.decrypt_u32(keys.elgamal.secret()).context("Cannot decrypt pending balance")
    };
    let lo = Zeroizing::new(decrypt(state.pending_balance_lo)?);
    let hi = Zeroizing::new(decrypt(state.pending_balance_hi)?);
    hi.checked_mul(1 << 16)
        .and_then(|v| v.checked_add(*lo))
        .context("Pending balance overflow")
}

pub(crate) struct Snapshot {
    address: Pubkey,
    mint: Pubkey,
    base: Account,
    decimals: u8,
    state: Option<ConfidentialTransferAccount>,
    mint_state: ConfidentialTransferMint,
    guards: Vec<(Pubkey, [u8; 32])>,
    configure_size: usize,
    lamports: u64,
}
fn supported<S: spl_token_2022_interface::extension::BaseState>(
    state: &impl BaseStateWithExtensions<S>,
) -> Result<()> {
    for extension in state.get_extension_types()? {
        ensure!(
            matches!(
                extension,
                ExtensionType::ConfidentialTransferMint
                    | ExtensionType::ConfidentialTransferAccount
                    | ExtensionType::ImmutableOwner
                    | ExtensionType::MintCloseAuthority
                    | ExtensionType::MetadataPointer
                    | ExtensionType::TokenMetadata
                    | ExtensionType::GroupPointer
                    | ExtensionType::TokenGroup
                    | ExtensionType::GroupMemberPointer
                    | ExtensionType::TokenGroupMember
            ),
            "{extension:?} is not supported by the confidential workflow"
        );
    }
    Ok(())
}
async fn snapshot(rpc: &RpcClient, address: Pubkey, selected_mint: &str) -> Result<Snapshot> {
    let raw = rpc
        .get_account(&address)
        .await
        .context("Cannot fetch token account")?;
    ensure!(
        raw.owner == spl_token_2022_interface::id(),
        "Confidential balances require Token-2022"
    );
    let account = StateWithExtensions::<Account>::unpack(&raw.data)?;
    supported(&account)?;
    ensure!(
        account.base.state == AccountState::Initialized,
        "Token account is frozen or uninitialized"
    );
    ensure!(
        account.base.mint.to_string() == selected_mint,
        "Token account belongs to a different mint"
    );
    let mint_raw = rpc.get_account(&account.base.mint).await?;
    ensure!(mint_raw.owner == raw.owner, "Mint program changed");
    let mint = StateWithExtensions::<Mint>::unpack(&mint_raw.data)?;
    supported(&mint)?;
    let mint_state = *mint
        .get_extension::<ConfidentialTransferMint>()
        .context("Mint does not support confidential balances")?;
    let mut extensions = account.get_extension_types()?;
    if !extensions.contains(&ExtensionType::ConfidentialTransferAccount) {
        extensions.push(ExtensionType::ConfidentialTransferAccount);
    }
    Ok(Snapshot {
        configure_size: ExtensionType::try_calculate_account_len::<Account>(&extensions)?,
        lamports: raw.lamports,
        address,
        mint: account.base.mint,
        base: account.base,
        decimals: mint.base.decimals,
        state: account
            .get_extension::<ConfidentialTransferAccount>()
            .ok()
            .copied(),
        mint_state,
        guards: vec![
            (address, account_guard(&raw.owner, &raw.data)),
            (
                account.base.mint,
                account_guard(&mint_raw.owner, &mint_raw.data),
            ),
        ],
    })
}

pub async fn reveal(
    profile: &RpcProfile,
    wallet: &Wallet,
    selected: &TokenAccount,
) -> Result<Zeroizing<Balances>> {
    let rpc = crate::network::client(profile);
    let before = rpc.get_genesis_hash().await?;
    let snapshot = snapshot(&rpc, selected.address.parse()?, &selected.mint).await?;
    ensure!(
        snapshot.base.owner.to_string() == wallet.address,
        "Select the account owner wallet to reveal balances"
    );
    let state = snapshot.state.context("Account has not been configured")?;
    let wallet = wallet.clone();
    let balances = tokio::task::spawn_blocking(move || {
        let keys = keys(&wallet)?;
        Ok::<_, anyhow::Error>(Zeroizing::new(Balances {
            public: snapshot.base.amount,
            available: available(&state, &keys)?,
            pending: pending(&state, &keys)?,
        }))
    })
    .await??;
    for (address, expected) in &snapshot.guards {
        let account = rpc.get_account(address).await?;
        ensure!(
            account_guard(&account.owner, &account.data) == *expected,
            "Account changed while revealing; try again"
        );
    }
    ensure!(
        rpc.get_genesis_hash().await? == before,
        "Network changed; reveal again"
    );
    Ok(balances)
}

pub struct Request {
    pub operation: Operation,
    pub value: String,
    pub recipient: String,
    pub recipient_is_account: bool,
}

pub async fn prepare(
    profile: &RpcProfile,
    payer: &Wallet,
    wallets: &[Wallet],
    selected: &TokenAccount,
    request: Request,
) -> Result<PreparedTransfer> {
    ensure!(!payer.program, "Program identities cannot sign");
    let rpc = crate::network::client(profile);
    let genesis = crate::network::verify_development_network(&rpc).await?;
    let address = if request.operation == Operation::Approve {
        request
            .recipient
            .parse()
            .context("Enter a valid token account address")?
    } else {
        selected.address.parse()?
    };
    let source = snapshot(&rpc, address, &selected.mint).await?;
    let mut guards = source.guards.clone();
    let authority = if request.operation == Operation::Approve {
        let address: Option<Pubkey> = source.mint_state.authority.into();
        let address = address.context("This mint has no approval authority")?;
        wallets
            .iter()
            .find(|w| !w.program && w.address == address.to_string())
            .context("Load the mint's confidential approval authority wallet")?
            .clone()
    } else {
        ensure!(
            source.base.owner.to_string() == payer.address,
            "The active wallet must own this token account"
        );
        payer.clone()
    };
    let recipient = if request.operation == Operation::Transfer {
        let destination: Pubkey = request
            .recipient
            .parse()
            .context("Enter a valid recipient address")?;
        let destination = if request.recipient_is_account {
            destination
        } else {
            crate::tokens::associated_address(
                &destination,
                &source.mint,
                &spl_token_2022_interface::id(),
            )
        };
        ensure!(
            destination != source.address,
            "Choose a different destination account"
        );
        let destination = snapshot(&rpc, destination, &selected.mint)
            .await
            .context("Recipient needs an existing configured confidential token account")?;
        receive_ready(&destination.state.context("Recipient is not configured")?)?;
        guards.extend(destination.guards.clone());
        Some(destination)
    } else {
        None
    };
    let amount = if matches!(
        request.operation,
        Operation::Deposit | Operation::Withdraw | Operation::Transfer
    ) {
        crate::tokens::parse_amount(&request.value, source.decimals)?
    } else {
        0
    };
    if matches!(request.operation, Operation::Deposit | Operation::Transfer) {
        ensure!(
            amount <= MAXIMUM_DEPOSIT_TRANSFER_AMOUNT,
            "A confidential deposit or transfer must be less than 2^48 raw units"
        );
    }
    let payer_address = payer.address.parse()?;
    let owner = authority.address.parse()?;
    let operation = request.operation;
    let extra_wallet = if owner != payer_address {
        Some(authority.clone())
    } else {
        None
    };
    let rent = if operation == Operation::Configure {
        rpc.get_minimum_balance_for_rent_exemption(source.configure_size)
            .await?
            .saturating_sub(source.lamports)
    } else {
        0
    };
    let format = if matches!(operation, Operation::Transfer | Operation::Withdraw) {
        Format::V1
    } else {
        Format::Legacy
    };
    let instructions = tokio::task::spawn_blocking(move || {
        build(
            &source,
            &authority,
            &payer_address,
            &owner,
            recipient.as_ref(),
            &request,
            amount,
        )
    })
    .await??;
    let mut prepared = crate::token_operations::prepare(profile, payer, &rpc, &genesis, &instructions, format).await.context("Confidential operation simulation failed. The endpoint must support v1 and the ZK ElGamal Proof Program")?;
    prepared.state_guards = guards;
    if let Some(wallet) = extra_wallet {
        prepared.extra_wallets.push(wallet);
    }
    prepared.confidential = true;
    prepared.summary = vec![
        operation.label().into(),
        format!("Mint {}", selected.mint),
        format!("Account {address}"),
        "One atomic transaction · no temporary proof accounts".into(),
    ];
    if rent > 0 {
        prepared.summary.push(format!(
            "Additional account rent estimate {} SOL",
            crate::amount::format_sol(rent)
        ));
    }
    if amount != 0 {
        prepared.summary.push(format!(
            "Amount {} · raw {amount}",
            crate::tokens::format_amount(amount, selected.decimals)
        ));
    }
    if operation == Operation::Transfer {
        prepared.summary.extend([
            "Recipient receives pending tokens; apply before spending.".into(),
            "Transfer amounts are encrypted. Account addresses remain public.".into(),
        ]);
    }
    if matches!(operation, Operation::Deposit | Operation::Withdraw) {
        prepared
            .summary
            .push("This operation reveals its amount on chain.".into());
    }
    Ok(prepared)
}

fn receive_ready(state: &ConfidentialTransferAccount) -> Result<()> {
    ensure!(
        bool::from(state.approved),
        "Recipient account needs confidential approval"
    );
    ensure!(
        bool::from(state.allow_confidential_credits),
        "Account does not accept confidential credits"
    );
    ensure!(
        u64::from(state.pending_balance_credit_counter)
            < u64::from(state.maximum_pending_balance_credit_counter),
        "Pending credit limit reached; the owner must apply pending balances before receiving more tokens"
    );
    Ok(())
}

fn build(
    source: &Snapshot,
    wallet: &Wallet,
    payer: &Pubkey,
    owner: &Pubkey,
    destination: Option<&Snapshot>,
    request: &Request,
    amount: u64,
) -> Result<Vec<Instruction>> {
    let program = spl_token_2022_interface::id();
    if request.operation == Operation::Approve {
        ensure!(
            !bool::from(
                source
                    .state
                    .context("Configure the recipient account first")?
                    .approved
            ),
            "Account is already approved"
        );
        return Ok(vec![ct::approve_account(
            &program,
            &source.address,
            &source.mint,
            owner,
            &[],
        )?]);
    }
    let keys = keys(wallet)?;
    if request.operation == Operation::Configure {
        ensure!(
            source.state.is_none_or(|s| s.elgamal_pubkey.0 == [0; 32]),
            "Account is already configured; Solte will not replace its keys"
        );
        let max: u64 = request
            .value
            .parse()
            .context("Pending credit limit must be an integer")?;
        ensure!(
            max > 0 && max <= 65536,
            "Pending credit limit must be between 1 and 65536"
        );
        let proof = build_pubkey_validity_proof_data(&keys.elgamal)?;
        let mut instructions = vec![spl_token_2022_interface::instruction::reallocate(
            &program,
            &source.address,
            payer,
            owner,
            &[],
            &[ExtensionType::ConfidentialTransferAccount],
        )?];
        instructions.extend(ct::configure_account(
            &program,
            &source.address,
            &source.mint,
            &keys.aes.encrypt(0).into(),
            max,
            owner,
            &[],
            ProofLocation::InstructionOffset(NonZeroI8::new(1).unwrap(), &proof),
        )?);
        return Ok(instructions);
    }
    let state = source.state.context("Configure this account first")?;
    check_keys(&state, &keys)?;
    ensure!(
        bool::from(state.approved),
        "This account needs confidential approval before it can be used"
    );
    match request.operation {
        Operation::Deposit => {
            ensure!(
                amount <= source.base.amount,
                "Insufficient public token balance"
            );
            receive_ready(&state)?;
            Ok(vec![ct::deposit(
                &program,
                &source.address,
                &source.mint,
                amount,
                source.decimals,
                owner,
                &[],
            )?])
        }
        Operation::Apply => {
            let balance = Zeroizing::new(available(&state, &keys)?);
            let pending = Zeroizing::new(pending(&state, &keys)?);
            let total = Zeroizing::new(balance.checked_add(*pending).context("Balance overflow")?);
            ensure!(
                u64::from(state.pending_balance_credit_counter) > 0,
                "No pending credits to apply"
            );
            Ok(vec![ct::apply_pending_balance(
                &program,
                &source.address,
                u64::from(state.pending_balance_credit_counter),
                &keys.aes.encrypt(*total).into(),
                owner,
                &[],
            )?])
        }
        Operation::Withdraw | Operation::Transfer => {
            let balance = Zeroizing::new(available(&state, &keys)?);
            let remaining = Zeroizing::new(balance.checked_sub(amount).context(
                "Insufficient confidential available balance; apply pending credits first",
            )?);
            let cipher: ElGamalCiphertext = state.available_balance.try_into()?;
            let next = keys.aes.encrypt(*remaining).into();
            if request.operation == Operation::Withdraw {
                let proof = withdraw_proof_data(&cipher, *balance, amount, &keys.elgamal)?;
                ct::withdraw(
                    &program,
                    &source.address,
                    &source.mint,
                    amount,
                    source.decimals,
                    &next,
                    owner,
                    &[],
                    ProofLocation::InstructionOffset(
                        NonZeroI8::new(1).unwrap(),
                        &proof.equality_proof_data,
                    ),
                    ProofLocation::InstructionOffset(
                        NonZeroI8::new(2).unwrap(),
                        &proof.range_proof_data,
                    ),
                )
                .map_err(Into::into)
            } else {
                let destination = destination.context("Recipient account missing")?;
                let destination_key: ElGamalPubkey = destination
                    .state
                    .context("Recipient is not configured")?
                    .elgamal_pubkey
                    .try_into()?;
                let auditor: Option<solana_zk_sdk_pod::encryption::elgamal::PodElGamalPubkey> =
                    source.mint_state.auditor_elgamal_pubkey.into();
                let auditor = auditor.map(ElGamalPubkey::try_from).transpose()?;
                let proof = transfer_split_proof_data(
                    &cipher,
                    &state.decryptable_available_balance.try_into()?,
                    amount,
                    &keys.elgamal,
                    &keys.aes,
                    &destination_key,
                    auditor.as_ref(),
                )?;
                let validity = proof.ciphertext_validity_proof_data_with_ciphertext;
                ct::transfer(
                    &program,
                    &source.address,
                    &source.mint,
                    &destination.address,
                    &next,
                    &validity.ciphertext_lo,
                    &validity.ciphertext_hi,
                    owner,
                    &[],
                    ProofLocation::InstructionOffset(
                        NonZeroI8::new(1).unwrap(),
                        &proof.equality_proof_data,
                    ),
                    ProofLocation::InstructionOffset(
                        NonZeroI8::new(2).unwrap(),
                        &validity.proof_data,
                    ),
                    ProofLocation::InstructionOffset(
                        NonZeroI8::new(3).unwrap(),
                        &proof.range_proof_data,
                    ),
                )
                .map_err(Into::into)
            }
        }
        _ => anyhow::bail!("Choose a confidential transaction operation"),
    }
}

pub(crate) fn account_guard(owner: &Pubkey, data: &[u8]) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(owner.as_ref());
    hash.update(data);
    hash.finalize().into()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recipient_eligibility_errors_explain_the_required_action() {
        let mut state = ConfidentialTransferAccount {
            approved: true.into(),
            allow_confidential_credits: true.into(),
            maximum_pending_balance_credit_counter: 10.into(),
            ..Default::default()
        };
        assert!(receive_ready(&state).is_ok());
        state.approved = false.into();
        assert!(
            receive_ready(&state)
                .unwrap_err()
                .to_string()
                .contains("approval")
        );
        state.approved = true.into();
        state.allow_confidential_credits = false.into();
        assert!(
            receive_ready(&state)
                .unwrap_err()
                .to_string()
                .contains("confidential credits")
        );
        state.allow_confidential_credits = true.into();
        state.pending_balance_credit_counter = 10.into();
        assert!(
            receive_ready(&state)
                .unwrap_err()
                .to_string()
                .contains("apply pending")
        );
    }

    #[test]
    fn confidential_keys_are_reproducible_and_wrong_owners_cannot_decrypt() {
        let directory = tempfile::tempdir().unwrap();
        let alice = crate::wallet::create(directory.path(), "alice").unwrap();
        let bob = crate::wallet::create(directory.path(), "bob").unwrap();
        let first = keys(&alice).unwrap();
        let second = keys(&alice).unwrap();
        assert_eq!(first.elgamal.pubkey(), second.elgamal.pubkey());
        let mut state = ConfidentialTransferAccount {
            elgamal_pubkey: first.elgamal.pubkey().to_owned().into(),
            decryptable_available_balance: first.aes.encrypt(10).into(),
            pending_balance_lo: first.elgamal.pubkey().encrypt(5u64).into(),
            pending_balance_hi: first.elgamal.pubkey().encrypt(0u64).into(),
            ..Default::default()
        };
        assert_eq!(available(&state, &second).unwrap(), 10);
        assert_eq!(pending(&state, &second).unwrap(), 5);
        assert!(available(&state, &keys(&bob).unwrap()).is_err());
        state.decryptable_available_balance.0[0] ^= 1;
        assert!(available(&state, &second).is_err());
    }
}
