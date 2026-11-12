//! End-to-end tests. Each `tests/fixtures/<name>.csv` must produce exactly
//! the content of `tests/fixtures/<name>.expected.csv`.

use std::{fs, path::PathBuf, process::Command};

fn fixture(file_name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(file_name)
}

/// Runs the library on a fixture and returns (output, diagnostics).
fn run_fixture(name: &str) -> (String, String) {
    let input = fs::File::open(fixture(&format!("{name}.csv"))).unwrap();
    let mut output = Vec::new();
    let mut diagnostics = Vec::new();
    payments_engine::run(input, &mut output, &mut diagnostics).unwrap();
    (String::from_utf8(output).unwrap(), String::from_utf8(diagnostics).unwrap())
}

fn expected(name: &str) -> String {
    fs::read_to_string(fixture(&format!("{name}.expected.csv"))).unwrap()
}

#[test]
fn basic_deposits_and_withdrawals() {
    let (output, diagnostics) = run_fixture("basic");
    assert_eq!(output, expected("basic"));
    assert_eq!(diagnostics.lines().count(), 1, "{diagnostics}");
    assert!(diagnostics.contains("row 6: rejected"), "{diagnostics}");
}

#[test]
fn disputes_resolutions_and_chargebacks() {
    let (output, diagnostics) = run_fixture("disputes");
    assert_eq!(output, expected("disputes"));
    // Row 9 is a withdrawal from the locked account (the deposit in row 8 is
    // accepted). Rows 15-17 refer to an unknown transaction or to another
    // client's transaction.
    let rejected_rows: Vec<&str> =
        diagnostics.lines().map(|line| line.split(':').next().unwrap()).collect();
    assert_eq!(rejected_rows, ["row 9", "row 15", "row 16", "row 17"]);
}

#[test]
fn whitespace_and_malformed_rows() {
    let (output, diagnostics) = run_fixture("messy");
    assert_eq!(output, expected("messy"));
    // One line per bad row: the withdrawal that is too large (row 6) and rows 8-15.
    assert_eq!(diagnostics.lines().count(), 9, "{diagnostics}");
}

#[test]
fn input_without_transactions_yields_only_a_header() {
    let (output, diagnostics) = run_fixture("empty");
    assert_eq!(output, expected("empty"));
    assert!(diagnostics.is_empty());
}

#[test]
fn binary_writes_accounts_to_stdout() {
    let result = Command::new(env!("CARGO_BIN_EXE_payments-engine"))
        .arg(fixture("disputes.csv"))
        .output()
        .unwrap();
    assert!(result.status.success());
    assert_eq!(String::from_utf8(result.stdout).unwrap(), expected("disputes"));
    assert!(!result.stderr.is_empty(), "rejections are reported on stderr");
}

#[test]
fn binary_fails_without_exactly_one_argument() {
    let binary = env!("CARGO_BIN_EXE_payments-engine");

    let no_arguments = Command::new(binary).output().unwrap();
    assert!(!no_arguments.status.success());
    assert!(no_arguments.stdout.is_empty());

    let two_arguments = Command::new(binary).args(["a.csv", "b.csv"]).output().unwrap();
    assert!(!two_arguments.status.success());
}

#[test]
fn binary_fails_when_the_file_is_missing() {
    let result = Command::new(env!("CARGO_BIN_EXE_payments-engine"))
        .arg(fixture("does-not-exist.csv"))
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(result.stdout.is_empty());
}
