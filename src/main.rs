use clap::{CommandFactory, FromArgMatches};

/// `--color X` / `--color=X` from argv (before `--`), so the help is colored as asked.
fn color_from_args(args: &[std::ffi::OsString]) -> Option<clap::ColorChoice> {
    use clap::ValueEnum;
    let mut value = None;
    let mut it = args
        .iter()
        .skip(1)
        .map(|a| a.to_string_lossy().into_owned());
    while let Some(a) = it.next() {
        if a == "--" {
            break;
        }
        if a == "--color" {
            value = it.next();
        } else if let Some(v) = a.strip_prefix("--color=") {
            value = Some(v.to_string());
        }
    }
    let arg = wingetrs::cli::style::ColorArg::from_str(&value?, true).ok()?;
    Some(match arg {
        wingetrs::cli::style::ColorArg::Auto => clap::ColorChoice::Auto,
        wingetrs::cli::style::ColorArg::Always => clap::ColorChoice::Always,
        wingetrs::cli::style::ColorArg::Never => clap::ColorChoice::Never,
    })
}

fn parse() -> wingetrs::cli::Cli {
    let args: Vec<std::ffi::OsString> = std::env::args_os().collect();
    let mut cmd = wingetrs::cli::Cli::command();
    if let Some(choice) = color_from_args(&args) {
        cmd = cmd.color(choice);
    }
    let matches = cmd.try_get_matches_from(args).unwrap_or_else(|e| e.exit());
    wingetrs::cli::Cli::from_arg_matches(&matches).unwrap_or_else(|e| e.exit())
}

fn main() -> std::process::ExitCode {
    let cli = parse();
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
