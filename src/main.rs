use std::{
    env,
    fs::File,
    io::{self, BufWriter},
    process::ExitCode,
};

fn main() -> ExitCode {
    let mut args = env::args_os().skip(1);
    let (Some(path), None) = (args.next(), args.next()) else {
        eprintln!("usage: payments-engine <transactions.csv>");
        return ExitCode::from(2);
    };

    let input = match File::open(&path) {
        Ok(file) => file,
        Err(error) => {
            eprintln!("cannot open {}: {error}", path.to_string_lossy());
            return ExitCode::FAILURE;
        },
    };

    // Account balances go to stdout; skipped and rejected rows go to stderr.
    let diagnostics = BufWriter::new(io::stderr().lock());
    match payments_engine::run(input, io::stdout().lock(), diagnostics) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        },
    }
}
