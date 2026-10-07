use clap::Parser;

fn main() -> std::process::ExitCode {
    let cli = wingetrs::cli::Cli::parse();
    match wingetrs::cli::run(cli) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("erro: {e:#}");
            std::process::ExitCode::FAILURE
        }
    }
}
