//! The asset account of a single client.

use thiserror::Error;

use crate::amount::Amount;

/// The reason an operation on an [`Account`] was rejected.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum AccountError {
    #[error("account is locked")]
    Locked,
    #[error("insufficient available funds")]
    InsufficientFunds,
    #[error("balance would overflow")]
    Overflow,
}

/// The balances of one client.
///
/// Only `available` and `held` are stored. `total` is calculated from them, so
/// the three values are always consistent. If an operation fails, the account
/// is not changed.
///
/// The engine releases or charges back only what it has held before, so `held`
/// is never negative. `available` can be negative. This happens when a deposit
/// is disputed after its funds were already withdrawn. The client then owes
/// money.
///
/// A chargeback locks the account. A locked account rejects withdrawals. It
/// still accepts deposits, so the client can repay a negative balance. There is
/// no operation that unlocks an account.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Account {
    available: Amount,
    held: Amount,
    locked: bool,
}

impl Account {
    /// Returns the funds the client can withdraw. Can be negative.
    pub fn available(&self) -> Amount {
        self.available
    }

    pub fn held(&self) -> Amount {
        self.held
    }

    pub fn total(&self) -> Amount {
        // This never saturates in practice. Only `deposit` increases the total,
        // and it rejects a deposit that would overflow it.
        self.available.saturating_add(self.held)
    }

    pub fn is_locked(&self) -> bool {
        self.locked
    }

    /// Credits `amount` to the available funds. This is allowed on a locked
    /// account.
    pub fn deposit(&mut self, amount: Amount) -> Result<(), AccountError> {
        let available = self.available.checked_add(amount).ok_or(AccountError::Overflow)?;
        // The total must not overflow either.
        available.checked_add(self.held).ok_or(AccountError::Overflow)?;
        self.available = available;
        Ok(())
    }

    /// Debits `amount` from the available funds.
    pub fn withdraw(&mut self, amount: Amount) -> Result<(), AccountError> {
        if self.locked {
            return Err(AccountError::Locked);
        }
        if amount > self.available {
            return Err(AccountError::InsufficientFunds);
        }
        self.available = self.available.checked_sub(amount).ok_or(AccountError::Overflow)?;
        Ok(())
    }

    /// Moves `amount` from available to held when a deposit is disputed.
    pub fn hold(&mut self, amount: Amount) -> Result<(), AccountError> {
        let available = self.available.checked_sub(amount).ok_or(AccountError::Overflow)?;
        let held = self.held.checked_add(amount).ok_or(AccountError::Overflow)?;
        self.available = available;
        self.held = held;
        Ok(())
    }

    /// Moves `amount` from held back to available when a dispute is resolved.
    pub fn release(&mut self, amount: Amount) -> Result<(), AccountError> {
        let held = self.held.checked_sub(amount).ok_or(AccountError::Overflow)?;
        let available = self.available.checked_add(amount).ok_or(AccountError::Overflow)?;
        self.available = available;
        self.held = held;
        Ok(())
    }

    /// Removes `amount` of held funds and locks the account.
    pub fn charge_back(&mut self, amount: Amount) -> Result<(), AccountError> {
        self.held = self.held.checked_sub(amount).ok_or(AccountError::Overflow)?;
        self.locked = true;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn amount(units: i64) -> Amount {
        Amount::from_minor_units(units)
    }

    fn balances(account: &Account) -> (Amount, Amount, Amount) {
        (account.available(), account.held(), account.total())
    }

    #[test]
    fn deposit_increases_available_and_total() {
        let mut account = Account::default();
        account.deposit(amount(15_000)).unwrap();
        assert_eq!(balances(&account), (amount(15_000), amount(0), amount(15_000)));
    }

    #[test]
    fn withdrawal_decreases_available_and_total() {
        let mut account = Account::default();
        account.deposit(amount(15_000)).unwrap();
        account.withdraw(amount(5_000)).unwrap();
        assert_eq!(balances(&account), (amount(10_000), amount(0), amount(10_000)));
    }

    #[test]
    fn withdrawal_of_the_whole_balance_is_allowed() {
        let mut account = Account::default();
        account.deposit(amount(15_000)).unwrap();
        account.withdraw(amount(15_000)).unwrap();
        assert_eq!(account.total(), amount(0));
    }

    #[test]
    fn withdrawal_beyond_available_funds_changes_nothing() {
        let mut account = Account::default();
        account.deposit(amount(15_000)).unwrap();
        assert_eq!(account.withdraw(amount(15_001)), Err(AccountError::InsufficientFunds));
        assert_eq!(account.total(), amount(15_000));
    }

    #[test]
    fn held_funds_cannot_be_withdrawn() {
        let mut account = Account::default();
        account.deposit(amount(15_000)).unwrap();
        account.hold(amount(10_000)).unwrap();
        assert_eq!(account.withdraw(amount(5_001)), Err(AccountError::InsufficientFunds));
        account.withdraw(amount(5_000)).unwrap();
    }

    #[test]
    fn hold_and_release_move_funds_without_changing_the_total() {
        let mut account = Account::default();
        account.deposit(amount(15_000)).unwrap();

        account.hold(amount(10_000)).unwrap();
        assert_eq!(balances(&account), (amount(5_000), amount(10_000), amount(15_000)));

        account.release(amount(10_000)).unwrap();
        assert_eq!(balances(&account), (amount(15_000), amount(0), amount(15_000)));
    }

    #[test]
    fn hold_can_drive_available_negative() {
        let mut account = Account::default();
        account.deposit(amount(10_000)).unwrap();
        account.withdraw(amount(8_000)).unwrap();
        account.hold(amount(10_000)).unwrap();
        assert_eq!(balances(&account), (amount(-8_000), amount(10_000), amount(2_000)));
    }

    #[test]
    fn charge_back_removes_held_funds_and_locks() {
        let mut account = Account::default();
        account.deposit(amount(15_000)).unwrap();
        account.hold(amount(10_000)).unwrap();
        account.charge_back(amount(10_000)).unwrap();
        assert_eq!(balances(&account), (amount(5_000), amount(0), amount(5_000)));
        assert!(account.is_locked());
    }

    #[test]
    fn locked_account_rejects_withdrawals() {
        let mut account = Account::default();
        account.deposit(amount(15_000)).unwrap();
        account.hold(amount(10_000)).unwrap();
        account.charge_back(amount(10_000)).unwrap();

        assert_eq!(account.withdraw(amount(1)), Err(AccountError::Locked));
        assert_eq!(account.total(), amount(5_000));
    }

    #[test]
    fn locked_account_accepts_deposits_and_stays_locked() {
        let mut account = Account::default();
        account.deposit(amount(15_000)).unwrap();
        account.hold(amount(10_000)).unwrap();
        account.charge_back(amount(10_000)).unwrap();

        account.deposit(amount(2_000)).unwrap();
        assert_eq!(balances(&account), (amount(7_000), amount(0), amount(7_000)));
        assert!(account.is_locked());
    }

    #[test]
    fn deposit_that_would_overflow_is_rejected() {
        let mut account = Account::default();
        account.deposit(amount(i64::MAX)).unwrap();
        assert_eq!(account.deposit(amount(1)), Err(AccountError::Overflow));
        assert_eq!(account.available(), amount(i64::MAX));
    }

    #[test]
    fn deposit_that_would_overflow_the_total_is_rejected() {
        let mut account = Account::default();
        account.deposit(amount(i64::MAX)).unwrap();
        account.hold(amount(i64::MAX)).unwrap();
        assert_eq!(account.deposit(amount(1)), Err(AccountError::Overflow));
        assert_eq!(account.total(), amount(i64::MAX));
    }
}
