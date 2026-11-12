//! Input rows and validated transactions.
//!
//! A CSV row is deserialized into a `Record` and then converted into a
//! [`Transaction`]. The conversion checks the amount and is part of
//! deserialization, so a `Transaction` always has a valid amount: every deposit
//! and withdrawal has an amount greater than zero.

use serde::Deserialize;
use thiserror::Error;

use crate::amount::Amount;

pub type ClientId = u16;
pub type TransactionId = u32;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
enum RecordKind {
    Deposit,
    Withdrawal,
    Dispute,
    Resolve,
    Chargeback,
}

/// One input row, as it appears in the file.
#[derive(Debug, Deserialize)]
struct Record {
    #[serde(rename = "type")]
    kind: RecordKind,
    client: ClientId,
    tx: TransactionId,
    /// Empty or missing for disputes, resolves and chargebacks.
    #[serde(default)]
    amount: Option<Amount>,
}

/// A valid transaction that can be applied to the ledger.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(try_from = "Record")]
pub enum Transaction {
    Deposit {
        client: ClientId,
        tx: TransactionId,
        amount: Amount,
    },
    Withdrawal {
        client: ClientId,
        tx: TransactionId,
        amount: Amount,
    },
    Dispute {
        client: ClientId,
        tx: TransactionId,
    },
    Resolve {
        client: ClientId,
        tx: TransactionId,
    },
    Chargeback {
        client: ClientId,
        tx: TransactionId,
    },
}

/// The reason a parsed row cannot be converted into a [`Transaction`].
#[derive(Debug, Error, PartialEq, Eq)]
enum InvalidRecord {
    #[error("transaction {0} has no amount")]
    MissingAmount(TransactionId),
    #[error("transaction {0} has a non-positive amount")]
    NonPositiveAmount(TransactionId),
}

impl TryFrom<Record> for Transaction {
    type Error = InvalidRecord;

    fn try_from(record: Record) -> Result<Self, Self::Error> {
        let Record { kind, client, tx, amount } = record;

        let positive_amount = || match amount {
            None => Err(InvalidRecord::MissingAmount(tx)),
            Some(amount) if !amount.is_positive() => Err(InvalidRecord::NonPositiveAmount(tx)),
            Some(amount) => Ok(amount),
        };

        // Disputes, resolves and chargebacks refer to an earlier transaction.
        // If the row has an amount, it is ignored.
        Ok(match kind {
            RecordKind::Deposit => Transaction::Deposit { client, tx, amount: positive_amount()? },
            RecordKind::Withdrawal => {
                Transaction::Withdrawal { client, tx, amount: positive_amount()? }
            },
            RecordKind::Dispute => Transaction::Dispute { client, tx },
            RecordKind::Resolve => Transaction::Resolve { client, tx },
            RecordKind::Chargeback => Transaction::Chargeback { client, tx },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(kind: RecordKind, amount: Option<i64>) -> Record {
        Record {
            kind,
            client: 1,
            tx: 7,
            amount: amount.map(Amount::from_minor_units),
        }
    }

    #[test]
    fn deposits_and_withdrawals_need_a_positive_amount() {
        for kind in [RecordKind::Deposit, RecordKind::Withdrawal] {
            assert_eq!(
                Transaction::try_from(record(kind, None)),
                Err(InvalidRecord::MissingAmount(7))
            );
            assert_eq!(
                Transaction::try_from(record(kind, Some(0))),
                Err(InvalidRecord::NonPositiveAmount(7))
            );
            assert_eq!(
                Transaction::try_from(record(kind, Some(-1))),
                Err(InvalidRecord::NonPositiveAmount(7))
            );
            assert!(Transaction::try_from(record(kind, Some(1))).is_ok());
        }
    }

    #[test]
    fn dispute_lifecycle_records_ignore_any_amount() {
        assert_eq!(
            Transaction::try_from(record(RecordKind::Dispute, None)),
            Ok(Transaction::Dispute { client: 1, tx: 7 })
        );
        assert_eq!(
            Transaction::try_from(record(RecordKind::Resolve, Some(-5))),
            Ok(Transaction::Resolve { client: 1, tx: 7 })
        );
        assert_eq!(
            Transaction::try_from(record(RecordKind::Chargeback, Some(5))),
            Ok(Transaction::Chargeback { client: 1, tx: 7 })
        );
    }
}
