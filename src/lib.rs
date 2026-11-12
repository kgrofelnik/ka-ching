//! A toy payments engine.
//!
//! Reads transactions from CSV, applies them to client accounts and writes the
//! final balances as CSV. The README describes the rules and the assumptions.
//!
//! [`run`] is the only entry point. The modules are private, so a transaction
//! can only be created from a CSV row, where its amount is validated.

mod account;
mod amount;
mod engine;
mod transaction;

use std::io::{self, Read, Write};

use thiserror::Error;

use crate::{engine::Engine, transaction::Transaction};

/// An error that stops processing.
///
/// A problem with a single row is not an error of this kind. The row is
/// reported on the diagnostics stream and skipped.
#[derive(Debug, Error)]
pub enum Error {
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
    #[error("CSV error: {0}")]
    Csv(#[from] csv::Error),
}

/// Processes all transactions in `input` and writes the final account
/// balances to `output`.
///
/// The input is streamed: only one row is in memory at a time. One line is
/// written to `diagnostics` for every row that is skipped or rejected.
///
/// # Errors
/// Returns an error if reading `input` fails, or if writing to `output` or
/// `diagnostics` fails. Malformed and rejected rows are not errors.
pub fn run<R, W, D>(input: R, output: W, mut diagnostics: D) -> Result<(), Error>
where
    R: Read,
    W: Write,
    D: Write,
{
    let mut engine = Engine::new();
    process_transactions(input, &mut engine, &mut diagnostics)?;
    // Flush here so that a failed write of the diagnostics is reported.
    diagnostics.flush()?;
    write_accounts(&engine, output)
}

fn process_transactions<R: Read, D: Write>(
    input: R,
    engine: &mut Engine,
    diagnostics: &mut D,
) -> Result<(), Error> {
    let mut reader = csv::ReaderBuilder::new()
        .trim(csv::Trim::All)
        // Rows without an amount can omit the last comma.
        .flexible(true)
        .from_reader(input);

    for (index, row) in reader.deserialize::<Transaction>().enumerate() {
        // Row 1 is the header.
        let row_number = index + 2;

        let transaction = match row {
            Ok(transaction) => transaction,
            Err(error) if matches!(error.kind(), csv::ErrorKind::Io(_)) => return Err(error.into()),
            Err(error) => {
                writeln!(diagnostics, "row {row_number}: skipped: {error}")?;
                continue;
            },
        };

        if let Err(rejection) = engine.apply(transaction) {
            writeln!(diagnostics, "row {row_number}: rejected: {rejection}")?;
        }
    }
    Ok(())
}

fn write_accounts<W: Write>(engine: &Engine, output: W) -> Result<(), Error> {
    // Sort by client so that the same input always gives the same output.
    let mut accounts: Vec<_> = engine.accounts().collect();
    accounts.sort_unstable_by_key(|(client, _)| *client);

    let mut writer = csv::Writer::from_writer(output);
    writer.write_record(["client", "available", "held", "total", "locked"])?;
    for (client, account) in accounts {
        writer.write_record([
            client.to_string(),
            account.available().to_string(),
            account.held().to_string(),
            account.total().to_string(),
            account.is_locked().to_string(),
        ])?;
    }
    writer.flush()?;
    Ok(())
}
