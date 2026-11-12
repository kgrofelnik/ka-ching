//! The ledger: applies transactions to client accounts.

use std::collections::{hash_map::Entry, HashMap};

use thiserror::Error;

use crate::{
    account::{Account, AccountError},
    amount::Amount,
    transaction::{ClientId, Transaction, TransactionId},
};

/// The reason a valid transaction was not applied.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum Rejection {
    #[error("transaction {tx}: {source}")]
    Account { tx: TransactionId, source: AccountError },
    #[error("transaction {0} was already processed")]
    DuplicateTransaction(TransactionId),
    #[error("transaction {0} does not refer to a known deposit")]
    UnknownTransaction(TransactionId),
    #[error("transaction {tx} does not belong to client {client}")]
    ClientMismatch { tx: TransactionId, client: ClientId },
    #[error("transaction {0} is already disputed")]
    AlreadyDisputed(TransactionId),
    #[error("transaction {0} is not under dispute")]
    NotDisputed(TransactionId),
    #[error("transaction {0} was charged back and is final")]
    ChargedBack(TransactionId),
}

/// The dispute state of a deposit.
///
/// ```text
/// Settled --dispute--> Disputed --resolve--> Settled
///                               --chargeback--> ChargedBack (final)
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DepositState {
    Settled,
    Disputed,
    ChargedBack,
}

/// The data kept for each deposit so that it can be disputed later.
#[derive(Debug)]
struct Deposit {
    client: ClientId,
    amount: Amount,
    state: DepositState,
}

/// Stores all client accounts and all deposits that can still be disputed.
///
/// Memory use depends on the number of clients (at most 65,536) and the number
/// of accepted deposits. Withdrawals are not stored because they cannot be
/// disputed (the README explains why).
#[derive(Debug, Default)]
pub struct Engine {
    accounts: HashMap<ClientId, Account>,
    deposits: HashMap<TransactionId, Deposit>,
}

impl Engine {
    pub fn new() -> Self {
        Self::default()
    }

    /// Applies one transaction. If the transaction is rejected, no balance is
    /// changed.
    pub fn apply(&mut self, transaction: Transaction) -> Result<(), Rejection> {
        match transaction {
            Transaction::Deposit { client, tx, amount } => self.deposit(client, tx, amount),
            Transaction::Withdrawal { client, tx, amount } => self.withdraw(client, tx, amount),
            Transaction::Dispute { client, tx } => self.dispute(client, tx),
            Transaction::Resolve { client, tx } => self.resolve(client, tx),
            Transaction::Chargeback { client, tx } => self.charge_back(client, tx),
        }
    }

    /// Returns all accounts. The order is not defined.
    pub fn accounts(&self) -> impl Iterator<Item = (ClientId, &Account)> {
        self.accounts.iter().map(|(client, account)| (*client, account))
    }

    fn deposit(
        &mut self,
        client: ClientId,
        tx: TransactionId,
        amount: Amount,
    ) -> Result<(), Rejection> {
        // The client record is created even if the deposit is then rejected.
        let account = self.accounts.entry(client).or_default();
        let Entry::Vacant(slot) = self.deposits.entry(tx) else {
            return Err(Rejection::DuplicateTransaction(tx));
        };
        account.deposit(amount).map_err(|source| Rejection::Account { tx, source })?;
        slot.insert(Deposit {
            client,
            amount,
            state: DepositState::Settled,
        });
        Ok(())
    }

    fn withdraw(
        &mut self,
        client: ClientId,
        tx: TransactionId,
        amount: Amount,
    ) -> Result<(), Rejection> {
        self.accounts
            .entry(client)
            .or_default()
            .withdraw(amount)
            .map_err(|source| Rejection::Account { tx, source })
    }

    fn dispute(&mut self, client: ClientId, tx: TransactionId) -> Result<(), Rejection> {
        let (deposit, account) = self.deposit_and_account(client, tx)?;
        match deposit.state {
            DepositState::Settled => {},
            DepositState::Disputed => return Err(Rejection::AlreadyDisputed(tx)),
            DepositState::ChargedBack => return Err(Rejection::ChargedBack(tx)),
        }
        account
            .hold(deposit.amount)
            .map_err(|source| Rejection::Account { tx, source })?;
        deposit.state = DepositState::Disputed;
        Ok(())
    }

    fn resolve(&mut self, client: ClientId, tx: TransactionId) -> Result<(), Rejection> {
        let (deposit, account) = self.deposit_and_account(client, tx)?;
        if deposit.state != DepositState::Disputed {
            return Err(Rejection::NotDisputed(tx));
        }
        account
            .release(deposit.amount)
            .map_err(|source| Rejection::Account { tx, source })?;
        deposit.state = DepositState::Settled;
        Ok(())
    }

    fn charge_back(&mut self, client: ClientId, tx: TransactionId) -> Result<(), Rejection> {
        let (deposit, account) = self.deposit_and_account(client, tx)?;
        if deposit.state != DepositState::Disputed {
            return Err(Rejection::NotDisputed(tx));
        }
        account
            .charge_back(deposit.amount)
            .map_err(|source| Rejection::Account { tx, source })?;
        deposit.state = DepositState::ChargedBack;
        Ok(())
    }

    /// Finds the deposit that a dispute, resolve or chargeback refers to, and the
    /// account of the client who made it.
    fn deposit_and_account(
        &mut self,
        client: ClientId,
        tx: TransactionId,
    ) -> Result<(&mut Deposit, &mut Account), Rejection> {
        let deposit = self.deposits.get_mut(&tx).ok_or(Rejection::UnknownTransaction(tx))?;
        if deposit.client != client {
            return Err(Rejection::ClientMismatch { tx, client });
        }
        // A stored deposit always has an account, so this lookup always succeeds.
        let account = self.accounts.get_mut(&client).ok_or(Rejection::UnknownTransaction(tx))?;
        Ok((deposit, account))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn amount(units: i64) -> Amount {
        Amount::from_minor_units(units)
    }

    fn deposit(client: ClientId, tx: TransactionId, units: i64) -> Transaction {
        Transaction::Deposit { client, tx, amount: amount(units) }
    }

    fn withdrawal(client: ClientId, tx: TransactionId, units: i64) -> Transaction {
        Transaction::Withdrawal { client, tx, amount: amount(units) }
    }

    fn dispute(client: ClientId, tx: TransactionId) -> Transaction {
        Transaction::Dispute { client, tx }
    }

    fn resolve(client: ClientId, tx: TransactionId) -> Transaction {
        Transaction::Resolve { client, tx }
    }

    fn chargeback(client: ClientId, tx: TransactionId) -> Transaction {
        Transaction::Chargeback { client, tx }
    }

    /// Applies every transaction and ignores rejections, like the binary does.
    fn engine_after(transactions: &[Transaction]) -> Engine {
        let mut engine = Engine::new();
        for transaction in transactions {
            let _ = engine.apply(*transaction);
        }
        engine
    }

    /// (available, held, total, locked) in ten-thousands.
    fn state(engine: &Engine, client: ClientId) -> (i64, i64, i64, bool) {
        let account = &engine.accounts[&client];
        (
            account.available().minor_units(),
            account.held().minor_units(),
            account.total().minor_units(),
            account.is_locked(),
        )
    }

    #[test]
    fn deposits_and_withdrawals_update_each_client_separately() {
        let engine = engine_after(&[
            deposit(1, 1, 10_000),
            deposit(2, 2, 20_000),
            deposit(1, 3, 20_000),
            withdrawal(1, 4, 15_000),
            withdrawal(2, 5, 30_000),
        ]);
        assert_eq!(state(&engine, 1), (15_000, 0, 15_000, false));
        assert_eq!(state(&engine, 2), (20_000, 0, 20_000, false));
    }

    #[test]
    fn withdrawal_without_funds_is_rejected() {
        let mut engine = Engine::new();
        assert_eq!(
            engine.apply(withdrawal(1, 1, 1)),
            Err(Rejection::Account {
                tx: 1,
                source: AccountError::InsufficientFunds
            })
        );
        assert_eq!(state(&engine, 1), (0, 0, 0, false));
    }

    #[test]
    fn duplicate_deposit_id_is_rejected() {
        let mut engine = engine_after(&[deposit(1, 1, 10_000)]);
        assert_eq!(engine.apply(deposit(2, 1, 50_000)), Err(Rejection::DuplicateTransaction(1)));
        assert_eq!(state(&engine, 1), (10_000, 0, 10_000, false));
        assert_eq!(state(&engine, 2), (0, 0, 0, false));
    }

    #[test]
    fn dispute_holds_the_deposited_amount() {
        let engine = engine_after(&[deposit(1, 1, 10_000), deposit(1, 2, 5_000), dispute(1, 1)]);
        assert_eq!(state(&engine, 1), (5_000, 10_000, 15_000, false));
    }

    #[test]
    fn resolve_releases_the_held_amount() {
        let engine = engine_after(&[deposit(1, 1, 10_000), dispute(1, 1), resolve(1, 1)]);
        assert_eq!(state(&engine, 1), (10_000, 0, 10_000, false));
    }

    #[test]
    fn chargeback_removes_the_funds_and_locks_the_account() {
        let engine = engine_after(&[
            deposit(1, 1, 10_000),
            deposit(1, 2, 5_000),
            dispute(1, 1),
            chargeback(1, 1),
        ]);
        assert_eq!(state(&engine, 1), (5_000, 0, 5_000, true));
    }

    #[test]
    fn locked_account_rejects_withdrawals() {
        let mut engine = engine_after(&[
            deposit(1, 1, 10_000),
            deposit(1, 2, 5_000),
            dispute(1, 1),
            chargeback(1, 1),
        ]);
        assert_eq!(
            engine.apply(withdrawal(1, 3, 1)),
            Err(Rejection::Account { tx: 3, source: AccountError::Locked })
        );
        assert_eq!(state(&engine, 1), (5_000, 0, 5_000, true));
    }

    #[test]
    fn locked_account_accepts_deposits_and_stays_locked() {
        let engine = engine_after(&[
            deposit(1, 1, 10_000),
            deposit(1, 2, 5_000),
            dispute(1, 1),
            chargeback(1, 1),
            deposit(1, 3, 2_000),
        ]);
        assert_eq!(state(&engine, 1), (7_000, 0, 7_000, true));
    }

    #[test]
    fn deposit_on_a_locked_account_repays_a_negative_balance() {
        let engine = engine_after(&[
            deposit(1, 1, 10_000),
            withdrawal(1, 2, 8_000),
            dispute(1, 1),
            chargeback(1, 1),
            deposit(1, 3, 8_000),
        ]);
        assert_eq!(state(&engine, 1), (0, 0, 0, true));
    }

    #[test]
    fn deposit_on_a_locked_account_can_be_disputed() {
        let engine = engine_after(&[
            deposit(1, 1, 10_000),
            dispute(1, 1),
            chargeback(1, 1),
            deposit(1, 2, 5_000),
            dispute(1, 2),
        ]);
        assert_eq!(state(&engine, 1), (0, 5_000, 5_000, true));
    }

    #[test]
    fn disputes_on_a_locked_account_are_still_processed() {
        let engine = engine_after(&[
            deposit(1, 1, 10_000),
            deposit(1, 2, 5_000),
            dispute(1, 1),
            chargeback(1, 1),
            dispute(1, 2),
            chargeback(1, 2),
        ]);
        assert_eq!(state(&engine, 1), (0, 0, 0, true));
    }

    #[test]
    fn dispute_of_unknown_transaction_is_rejected() {
        let mut engine = engine_after(&[deposit(1, 1, 10_000)]);
        assert_eq!(engine.apply(dispute(1, 99)), Err(Rejection::UnknownTransaction(99)));
        assert_eq!(engine.apply(resolve(1, 99)), Err(Rejection::UnknownTransaction(99)));
        assert_eq!(engine.apply(chargeback(1, 99)), Err(Rejection::UnknownTransaction(99)));
        assert_eq!(state(&engine, 1), (10_000, 0, 10_000, false));
    }

    #[test]
    fn dispute_for_unknown_transaction_does_not_create_an_account() {
        let engine = engine_after(&[dispute(7, 1)]);
        assert_eq!(engine.accounts().count(), 0);
    }

    #[test]
    fn withdrawals_cannot_be_disputed() {
        let mut engine = engine_after(&[deposit(1, 1, 10_000), withdrawal(1, 2, 4_000)]);
        assert_eq!(engine.apply(dispute(1, 2)), Err(Rejection::UnknownTransaction(2)));
        assert_eq!(state(&engine, 1), (6_000, 0, 6_000, false));
    }

    #[test]
    fn client_cannot_dispute_another_clients_deposit() {
        let mut engine = engine_after(&[deposit(1, 1, 10_000), deposit(2, 2, 10_000)]);
        assert_eq!(
            engine.apply(dispute(2, 1)),
            Err(Rejection::ClientMismatch { tx: 1, client: 2 })
        );
        assert_eq!(state(&engine, 1), (10_000, 0, 10_000, false));
        assert_eq!(state(&engine, 2), (10_000, 0, 10_000, false));
    }

    #[test]
    fn resolve_and_chargeback_require_an_open_dispute() {
        let mut engine = engine_after(&[deposit(1, 1, 10_000)]);
        assert_eq!(engine.apply(resolve(1, 1)), Err(Rejection::NotDisputed(1)));
        assert_eq!(engine.apply(chargeback(1, 1)), Err(Rejection::NotDisputed(1)));
        assert_eq!(state(&engine, 1), (10_000, 0, 10_000, false));
    }

    #[test]
    fn deposit_cannot_be_disputed_twice_at_once() {
        let mut engine = engine_after(&[deposit(1, 1, 10_000), dispute(1, 1)]);
        assert_eq!(engine.apply(dispute(1, 1)), Err(Rejection::AlreadyDisputed(1)));
        assert_eq!(state(&engine, 1), (0, 10_000, 10_000, false));
    }

    #[test]
    fn resolved_deposit_can_be_disputed_again() {
        let engine = engine_after(&[
            deposit(1, 1, 10_000),
            dispute(1, 1),
            resolve(1, 1),
            dispute(1, 1),
            chargeback(1, 1),
        ]);
        assert_eq!(state(&engine, 1), (0, 0, 0, true));
    }

    #[test]
    fn chargeback_is_final() {
        let mut engine = engine_after(&[deposit(1, 1, 10_000), dispute(1, 1), chargeback(1, 1)]);
        assert_eq!(engine.apply(dispute(1, 1)), Err(Rejection::ChargedBack(1)));
        assert_eq!(engine.apply(resolve(1, 1)), Err(Rejection::NotDisputed(1)));
        assert_eq!(engine.apply(chargeback(1, 1)), Err(Rejection::NotDisputed(1)));
        assert_eq!(state(&engine, 1), (0, 0, 0, true));
    }

    #[test]
    fn disputing_spent_funds_leaves_a_negative_available_balance() {
        let engine = engine_after(&[deposit(1, 1, 10_000), withdrawal(1, 2, 8_000), dispute(1, 1)]);
        assert_eq!(state(&engine, 1), (-8_000, 10_000, 2_000, false));
    }

    #[test]
    fn disputing_spent_funds_can_be_charged_back() {
        let engine = engine_after(&[
            deposit(1, 1, 10_000),
            withdrawal(1, 2, 8_000),
            dispute(1, 1),
            chargeback(1, 1),
        ]);
        assert_eq!(state(&engine, 1), (-8_000, 0, -8_000, true));
    }

    #[test]
    fn disputing_spent_funds_can_be_resolved() {
        let engine = engine_after(&[
            deposit(1, 1, 10_000),
            withdrawal(1, 2, 8_000),
            dispute(1, 1),
            resolve(1, 1),
        ]);
        assert_eq!(state(&engine, 1), (2_000, 0, 2_000, false));
    }
}
