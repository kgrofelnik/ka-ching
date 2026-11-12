# payments-engine

A simple payments engine. It reads transactions from a CSV file, applies them to
client accounts, and prints the final state of every account as CSV. It supports
deposits, withdrawals, disputes, resolves and chargebacks.

## Usage

```bash
cargo run -- transactions.csv > accounts.csv
```

The input file is the only argument. The account balances are written to stdout.
Every row that is skipped or rejected is reported on stderr with the row number
and the reason, so stdout always contains only valid CSV.

Input:

```csv
type, client, tx, amount
deposit, 1, 1, 1.0
deposit, 2, 2, 2.0
deposit, 1, 3, 2.0
withdrawal, 1, 4, 1.5
withdrawal, 2, 5, 3.0
```

Output:

```csv
client,available,held,total,locked
1,1.5000,0.0000,1.5000,false
2,2.0000,0.0000,2.0000,false
```

Rows are sorted by client. Amounts are always printed with four decimal places.

The exit code is non-zero only when the program cannot do its work: wrong number
of arguments, the input file cannot be opened, or an I/O error. A bad row never
stops the run.

## Transactions

Every transaction belongs to one client. There are no transfers between clients.
The table shows what each accepted transaction changes.

| Transaction | available | held | total | locked | Deposit state |
| --- | --- | --- | --- | --- | --- |
| deposit | + amount | | + amount | | new, settled |
| withdrawal | − amount | | − amount | | |
| dispute | − amount | + amount | same | | settled → disputed |
| resolve | + amount | − amount | same | | disputed → settled |
| chargeback | | − amount | − amount | true | disputed → charged back |

A dispute, resolve or chargeback has no amount of its own. It uses the amount of
the deposit that it refers to.

## Assumptions

The specification does not define every case. These are the decisions I made. In
each case I chose the behavior that makes sense for a bank.

1. Only deposits can be disputed. A dispute decreases `available` and
   increases `held`, and a chargeback then removes the held funds. This is
   correct for a deposit. For a withdrawal it would take the money from the
   client a second time. A dispute that refers to a withdrawal is ignored, the
   same as a dispute that refers to an unknown transaction.

2. Only the owner of a deposit can dispute it. If the `client` in a dispute,
   resolve or chargeback is not the client who made the deposit, the row is
   ignored.

3. `available` can be negative. A client can deposit, withdraw the money and
   then dispute the deposit. In this case the full amount is still held and
   `available` becomes negative. This is the fraud case from the task
   description, and the negative balance shows how much the client owes.

4. A locked account cannot withdraw, but it still accepts deposits. A
   chargeback immediately blocks withdrawals. Deposits are still accepted so the
   client can repay a negative balance. Disputes, resolves and chargebacks are
   still processed, so one chargeback does not block other disputes for the
   same client.

   The account stays locked even after the balance is repaid. The specification
   has no operation to unlock it.

5. A resolved deposit can be disputed again. A chargeback is final. Each
   deposit has one of three states. Any other transition is ignored.

   ```mermaid
   stateDiagram-v2
       direction LR
       [*] --> Settled: deposit
       Settled --> Disputed: dispute
       Disputed --> Settled: resolve
       Disputed --> ChargedBack: chargeback
   ```

6. Amounts must be positive and have at most four decimal places. A deposit
   or withdrawal with a missing, zero or negative amount is skipped. An amount
   that needs more than four decimal places (`1.23456`) is also skipped and not
   rounded. Trailing zeros (`1.50000`) are accepted. An amount in a dispute,
   resolve or chargeback row is ignored.

7. A deposit with an already used transaction ID is rejected. The
   specification says that IDs are unique, so a repeated ID is treated as an
   error. This check exists only for deposits (see [Efficiency](#efficiency)).

8. A client record is created by the first deposit or withdrawal of that
   client. This is true even if the transaction is rejected, for example a
   withdrawal from an empty account. Such a client appears in the output with
   zero balances. Disputes, resolves and chargebacks never create a client.

9. Invalid rows are skipped. A row with an unknown transaction type, an
   invalid number or missing columns is reported on stderr, and processing
   continues. One bad row should not block the results for all other clients.
   Transaction types must be lower case, as in the specification.

## Correctness

- Money is never a float. `Amount` is an `i64` that counts ten-thousandths, so
  every value with four decimal places is exact.
- All arithmetic is checked. An operation that would overflow is rejected like
  any other invalid transaction.
- `total` is not stored. It is calculated from `available` and `held`, so the
  three values always agree.
- An account operation calculates the new balances first and writes them only if
  every step succeeded. A rejected transaction changes nothing.
- A `Transaction` is created only from a CSV row, and that conversion rejects a
  deposit or withdrawal without a positive amount. The modules are private, so
  no other code can create one.

`cargo test` runs unit tests for the rules above and end-to-end tests that
compare the output for the files in `tests/fixtures/` with the expected output.
The fixtures cover the example from the specification, the dispute flow and
invalid input.

## Error handling

There is no `unsafe`, and no `unwrap` or `expect` outside of the tests. An
invalid row or a rejected transaction is reported on stderr and skipped, and the
balances are not changed. Only an I/O error stops the program, with a message on
stderr and a non-zero exit code.

## Efficiency

Rows are read, processed and dropped one at a time, so the input file is never
loaded into memory.

The engine keeps one small record per client (at most 65,536) and one per
accepted deposit, because any deposit can be disputed later. Withdrawals are not
stored. This is why duplicate IDs are detected only for deposits.

In a test with a generated file of 5,000,000 rows (137 MB, 2.7 million
deposits), a release build needed about 1.6 seconds and about 210 MB of memory
on a laptop.

If all 2^32 transaction IDs were deposits, the deposit map would need more than
200 GB. In that case the map should be moved to disk, for example to an embedded
key-value store with the transaction ID as the key. Nothing else in the engine
grows with the size of the input. A real system would also limit how long a
deposit can be disputed, and remove older deposits.

### Running in a server

`run` accepts any `std::io::Read`, so it works with a TCP stream in the same way
as with a file. For thousands of concurrent streams I would not put the engine
behind a lock. I would use several workers, each with its own engine, and send
every transaction to the worker selected by `client % workers`. All transactions
of one client then arrive at the same worker in order, and every worker needs
only the deposits of its own clients.
