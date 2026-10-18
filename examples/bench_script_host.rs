//! Script host cost (Task 163): runs one synthetic `runner.ps1` through `PwshHost` and exits.
//! Timed from outside by `scripts/bench.ps1 -Baseline target/release/examples/noop.exe`, so the
//! delta is PowerShell's start-up plus what the runner loads. Never runs a manifest script.
//!
//! Usage: `bench_script_host <locate|absolute PowerShell exe> <work dir> <empty|prelude>`
//! (`locate` = `PwshHost::locate()`, the production choice: pwsh 7 when installed.)
//! - `empty`: the runner file is empty.
//! - `prelude`: the real `runner_ps1` text (no variables) dot-sourcing the real prelude and
//!   running an empty hook.
#[cfg(windows)]
fn main() -> anyhow::Result<()> {
    use std::path::PathBuf;

    use anyhow::{Context, bail};
    use wingetrs::install::executor::{ScriptHost, ScriptJob};
    use wingetrs::install::manifest::HookKind;
    use wingetrs::install::prelude::PRELUDE;
    use wingetrs::install::runner::{RunnerInput, ScriptBody, runner_ps1};
    use wingetrs::install::script_host_win::PwshHost;

    let mut args = std::env::args().skip(1);
    let program = args.next().context("faltou o executável do PowerShell")?;
    let work = PathBuf::from(args.next().context("faltou a pasta de trabalho")?);
    let mode = args.next().context("faltou o modo (empty|prelude)")?;
    std::fs::create_dir_all(&work)?;
    let runner = work.join("runner.ps1");
    // UTF-8 with BOM, as the executor writes it (powershell 5.1 reads BOM-less as ANSI).
    let mut bytes = b"\xEF\xBB\xBF".to_vec();
    match mode.as_str() {
        "empty" => {}
        "prelude" => {
            let prelude = work.join("prelude.ps1");
            let hook = work.join("hook.ps1");
            std::fs::write(&prelude, PRELUDE)?;
            std::fs::write(&hook, "")?;
            let text = runner_ps1(&RunnerInput {
                contract: &[],
                vars: &[],
                prelude_path: &prelude,
                body: ScriptBody::Hook { script_path: &hook },
            })?;
            bytes.extend_from_slice(text.as_bytes());
        }
        other => bail!("modo desconhecido: {other}"),
    }
    std::fs::write(&runner, &bytes)?;
    let host = match program.as_str() {
        "locate" => PwshHost::locate(),
        p => PwshHost::with_program(PathBuf::from(p)),
    };
    let out = host.run(&ScriptJob {
        app: "bench".into(),
        hook: HookKind::PostInstall,
        workdir: work.clone(),
        runner,
        env: Vec::new(),
    })?;
    if out.code != 0 {
        bail!("runner terminou com {}: {:?}", out.code, out.lines);
    }
    Ok(())
}

#[cfg(not(windows))]
fn main() {}
