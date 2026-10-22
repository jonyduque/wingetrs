//! Task 227: what PowerShell's parser changes in the script gate, measured before wiring it.
//! For every script of every scoop manifest of a local catalog, the verdict of the lexer alone
//! (`LexerOnly`) and with the parser (`PwshAst` of `PwshHost::locate`), the scripts whose
//! verdict got worse, the parser failures and the times. Reads the catalog read-only; never
//! runs a script (the parser only parses). Prints Markdown.
//!
//! Usage: `cargo run --release --example gate_ast_delta -- <catalog.db>`
#[cfg(not(windows))]
fn main() {}

#[cfg(windows)]
fn main() -> anyhow::Result<()> {
    win::main()
}

#[cfg(windows)]
mod win {
    use std::collections::BTreeMap;
    use std::time::{Duration, Instant};

    use anyhow::Context;
    use rusqlite::{Connection, OpenFlags};
    use wingetrs::install::manifest::{Arch, HookKind, Resolved, resolve};
    use wingetrs::install::prelude::PRELUDE_FUNCTIONS;
    use wingetrs::install::script_ast::{AstGate, LexerOnly};
    use wingetrs::install::script_ast_win::PwshAst;
    use wingetrs::install::script_host_win::PwshHost;
    use wingetrs::install::script_scan::{CallRoots, FlagKind, Verdict, scan_gated};

    fn scripts_of(m: &Resolved) -> Vec<(HookKind, &str)> {
        [
            (HookKind::PreInstall, m.hooks.pre_install.as_deref()),
            (
                HookKind::InstallerScript,
                m.installer.as_ref().and_then(|i| i.script.as_deref()),
            ),
            (HookKind::PostInstall, m.hooks.post_install.as_deref()),
            (HookKind::PreUninstall, m.hooks.pre_uninstall.as_deref()),
            (
                HookKind::UninstallerScript,
                m.uninstaller.as_ref().and_then(|i| i.script.as_deref()),
            ),
            (HookKind::PostUninstall, m.hooks.post_uninstall.as_deref()),
        ]
        .into_iter()
        .filter_map(|(k, s)| s.map(|s| (k, s)))
        .collect()
    }

    fn name(v: Verdict) -> &'static str {
        match v {
            Verdict::Confirm => "Confirm",
            Verdict::ConfirmStrong => "ConfirmStrong",
            Verdict::Refuse => "Refuse",
        }
    }

    pub fn main() -> anyhow::Result<()> {
        let db = std::env::args().nth(1).context("faltou o catalog.db")?;
        let conn = Connection::open_with_flags(
            &db,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        let mut st = conn.prepare(
            "SELECT source, id, raw FROM packages WHERE kind = 'scoop' AND raw IS NOT NULL",
        )?;
        let rows = st.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?;
        let host = PwshHost::locate();
        let ast = PwshAst::for_program(host.program().to_path_buf());
        let roots = CallRoots::default();

        let mut before: BTreeMap<&str, usize> = BTreeMap::new();
        let mut after: BTreeMap<&str, usize> = BTreeMap::new();
        let mut worse: Vec<String> = Vec::new();
        let (mut total, mut accepted, mut newly_refused) = (0usize, 0usize, 0usize);
        let (mut parser_failed, mut syntax) = (0usize, 0usize);
        let mut first: Option<Duration> = None;
        let mut parse_time = Duration::ZERO;
        let start = Instant::now();
        for r in rows {
            let (source, id, raw) = r?;
            let Ok(m) = resolve(&raw, Arch::X64) else {
                continue;
            };
            for (hook, script) in scripts_of(&m) {
                total += 1;
                let (lex, _) = scan_gated(script, PRELUDE_FUNCTIONS, &roots, &LexerOnly);
                let t = Instant::now();
                // Warms the cache: `scan_gated` below reads the same answer.
                let _ = ast.facts(script);
                let took = t.elapsed();
                first.get_or_insert(took);
                parse_time += took;
                let (gated, _) = scan_gated(script, PRELUDE_FUNCTIONS, &roots, &ast);
                *before.entry(name(lex.verdict)).or_default() += 1;
                *after.entry(name(gated.verdict)).or_default() += 1;
                if lex.verdict != Verdict::Refuse {
                    accepted += 1;
                    if gated.verdict == Verdict::Refuse {
                        newly_refused += 1;
                    }
                }
                if gated.flags.iter().any(|f| f.kind == FlagKind::ParserFailed) {
                    parser_failed += 1;
                }
                if gated.flags.iter().any(|f| f.kind == FlagKind::SyntaxError) {
                    syntax += 1;
                }
                if gated.verdict > lex.verdict {
                    let new: Vec<String> = gated
                        .flags
                        .iter()
                        .filter(|f| f.verdict == gated.verdict && !lex.flags.contains(f))
                        .map(|f| format!("{:?} linha {}: {}", f.kind, f.line, f.detail))
                        .collect();
                    worse.push(format!(
                        "| {source}/{id} | {} | {} → {} | {} |",
                        hook.key(),
                        name(lex.verdict),
                        name(gated.verdict),
                        new.join("; ")
                    ));
                }
            }
        }
        let wall = start.elapsed();
        println!("# Delta do portão com o parser do PowerShell (Task 227)\n");
        println!("PowerShell: `{}`\n", host.program().display());
        println!("Scripts analisados: **{total}**\n");
        println!("| Veredito | Só léxico | Léxico + AST |");
        println!("|---|---|---|");
        for v in ["Confirm", "ConfirmStrong", "Refuse"] {
            println!(
                "| {v} | {} | {} |",
                before.get(v).copied().unwrap_or(0),
                after.get(v).copied().unwrap_or(0)
            );
        }
        let pct = if accepted == 0 {
            0.0
        } else {
            100.0 * newly_refused as f64 / accepted as f64
        };
        println!(
            "\nAceitos pelo léxico que passam a Refuse: **{newly_refused}** de {accepted} ({pct:.2} %; limite da tarefa: 5 %)"
        );
        println!(
            "Falhas do parser (ParserFailed): **{parser_failed}**; erro de sintaxe: **{syntax}**\n"
        );
        let first = first.unwrap_or_default();
        let per = if total == 0 {
            Duration::ZERO
        } else {
            parse_time / u32::try_from(total).unwrap_or(u32::MAX)
        };
        println!(
            "Tempo: primeira resposta (com a partida) {first:?}; parser no total {parse_time:?}; por script {per:?}; execução inteira {wall:?}\n"
        );
        println!("## Scripts com veredito pior ({})\n", worse.len());
        println!("| App | Hook | Veredito | Flags novas |");
        println!("|---|---|---|---|");
        for w in &worse {
            println!("{w}");
        }
        Ok(())
    }
}
