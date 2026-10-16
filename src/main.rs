use clap::Parser;

fn main() -> std::process::ExitCode {
    let cli = wingetrs::cli::Cli::parse();
    match wingetrs::cli::run(cli) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            anstream::eprintln!(
                "{} {e:#}",
                wingetrs::cli::style::paint(wingetrs::cli::style::ERROR, "erro:")
            );
            std::process::ExitCode::FAILURE
        }
    }
}
