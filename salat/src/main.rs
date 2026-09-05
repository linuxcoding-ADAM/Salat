//! Process boundary (§18.1 layer 3): install the panic hook, parse `Cli`,
//! dispatch to a `cli::commands::*` function, map any error to its exit
//! code. Four things, nothing else (§19).

use clap::Parser as _;

fn main() -> std::process::ExitCode {
    // The color-eyre hook restores the terminal before printing a panic;
    // installation itself cannot be meaningfully handled and is ignored on
    // failure so a broken hook never masks the real error path.
    let _ = color_eyre::install();

    let cli = salat::cli::Cli::parse();
    salat::app::init_logging(cli.verbose);

    match salat::cli::dispatch(cli) {
        Ok(code) => code.into(),
        Err(err) => {
            // Diagnostics go to stderr — stdout carries a frame or nothing
            // (§14.6). Detail lines (the offending key, the path, …)
            // follow the error line.
            eprintln!("salat: error[{}]: {err}", err.code());
            for line in err.details() {
                eprintln!("{line}");
            }
            err.exit_code().into()
        }
    }
}
