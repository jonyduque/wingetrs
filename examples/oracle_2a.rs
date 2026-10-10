//! Phase 2a compatibility oracle (Task 40): runs `resolve` + `plan_install` over the
//! scoop fixtures and over scoop manifests of a local catalog. Plans only; installs nothing.
//!
//! Usage: `cargo run --release --example oracle_2a -- <fixtures dir> <catalog.db> [sample size]`
//!
//! "Simple" manifest = no `pre_install`, `post_install`, `pre_uninstall`, `post_uninstall`,
//! `installer`, `uninstaller`, `psmodule`, `innosetup` at the root or in any architecture
//! variant. The sample is the first N simple manifests ordered by SHA-256 of `source/id`
//! (deterministic, independent of resolve's own checks).
use std::collections::BTreeMap;
use std::path::Path;

use anyhow::Context;
use rusqlite::{Connection, OpenFlags};
use serde_json::Value;
use sha2::{Digest, Sha256};
use wingetrs::install::layout::Layout;
use wingetrs::install::manifest::{Arch, resolve};
use wingetrs::install::planner::{PlanInput, plan_install};

const SCRIPT_KEYS: [&str; 8] = [
    "pre_install",
    "post_install",
    "pre_uninstall",
    "post_uninstall",
    "installer",
    "uninstaller",
    "psmodule",
    "innosetup",
];

fn is_simple(raw: &str) -> bool {
    let Ok(v) = serde_json::from_str::<Value>(raw) else {
        return false;
    };
    let has = |o: &Value| SCRIPT_KEYS.iter().any(|k| o.get(k).is_some());
    if has(&v) {
        return false;
    }
    match v.get("architecture").and_then(Value::as_object) {
        Some(arch) => !arch.values().any(has),
        None => true,
    }
}

/// `Ok(())` or the failure reason, with the error kind first (for grouping).
fn check(app: &str, bucket: &str, raw: &str, layout: &Layout) -> Result<(), String> {
    let m = resolve(raw, Arch::host()).map_err(|e| format!("resolve: {e}"))?;
    plan_install(&PlanInput {
        app,
        bucket,
        manifest: &m,
        layout,
        shortcut_root: Path::new(r"C:\oracle\Start Menu\wingetrs Apps"),
        installed: None,
        launcher_present: false,
        keep_old: false,
        manifest_sha256: "",
    })
    .map(|_| ())
    .map_err(|e| format!("plan: {e}"))
}

/// Groups a reason by its stable prefix (drops per-manifest details after `: ` in the
/// unsupported-extension and unsafe-path messages).
fn group(reason: &str) -> String {
    let mut parts = reason.splitn(3, ": ");
    let a = parts.next().unwrap_or_default();
    let b = parts.next().unwrap_or_default();
    match parts.next() {
        Some(c) if b == "não suportado" => {
            // Keep the extension-like tail short: first word of the detail.
            let head: String = c.chars().take(60).collect();
            format!("{a}: {b}: {head}")
        }
        _ => format!("{a}: {b}"),
    }
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let fixtures = args.get(1).context("faltou o diretório de fixtures")?;
    let db = args.get(2).context("faltou o catalog.db")?;
    let n: usize = args.get(3).map(|s| s.parse()).transpose()?.unwrap_or(50);
    let layout = Layout::new(r"C:\oracle\wingetrs");
    println!("host arch: {:?}", Arch::host());

    // 1. Fixtures.
    let mut names: Vec<String> = std::fs::read_dir(fixtures)?
        .map(|e| e.map(|e| e.file_name().to_string_lossy().into_owned()))
        .collect::<Result<_, _>>()?;
    names.sort();
    let (mut ok, mut expected_unsup, mut bad) = (0, 0, 0);
    for name in &names {
        let raw = std::fs::read_to_string(Path::new(fixtures).join(name))?;
        let r = check("app", "main", &raw, &layout);
        let unsup = name.starts_with("unsup-");
        match (&r, unsup) {
            (Ok(()), false) => ok += 1,
            (Err(_), true) => expected_unsup += 1,
            _ => bad += 1,
        }
        println!(
            "fixture {name}: {}",
            match &r {
                Ok(()) => "ok".to_string(),
                Err(e) => e.clone(),
            }
        );
    }
    println!(
        "fixtures: {} total, {ok} ok, {expected_unsup} unsup-* recusados como esperado, {bad} divergentes",
        names.len()
    );

    // 2. Catalog (read-only).
    let conn = Connection::open_with_flags(
        db,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    let mut st = conn
        .prepare("SELECT source, id, raw FROM packages WHERE kind = 'scoop' AND raw IS NOT NULL")?;
    let rows = st.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
        ))
    })?;
    let mut all = Vec::new();
    for r in rows {
        all.push(r?);
    }
    let total = all.len();
    let mut simple: Vec<(String, String, String, String)> = all
        .into_iter()
        .filter(|(_, _, raw)| is_simple(raw))
        .map(|(s, id, raw)| {
            let key = hex::encode(Sha256::digest(format!("{s}/{id}").as_bytes()));
            (key, s, id, raw)
        })
        .collect();
    simple.sort();
    println!(
        "catálogo: {total} manifestos scoop, {} simples",
        simple.len()
    );

    let run = |set: &[(String, String, String, String)], label: &str, verbose: bool| {
        let mut ok = 0usize;
        let mut reasons: BTreeMap<String, usize> = BTreeMap::new();
        for (_, source, id, raw) in set {
            let bucket = source.strip_prefix("scoop:").unwrap_or(source);
            match check(id, bucket, raw, &layout) {
                Ok(()) => ok += 1,
                Err(e) => {
                    if verbose || !e.starts_with("resolve: não suportado") {
                        println!("  falha {source}/{id}: {e}");
                    }
                    *reasons.entry(group(&e)).or_default() += 1;
                }
            }
        }
        println!(
            "{label}: {} manifestos, {ok} ok, {} falhas",
            set.len(),
            set.len() - ok
        );
        for (r, c) in &reasons {
            println!("  {c:>5}  {r}");
        }
    };
    let sample = &simple[..n.min(simple.len())];
    for (_, source, id, _) in sample {
        println!("  amostra {source}/{id}");
    }
    run(sample, "amostra", true);
    run(&simple, "todos os simples", false);

    // 3. Planner timing (pure: resolve + plan_install, no I/O), ripgrep fixture.
    let rg = std::fs::read_to_string(Path::new(fixtures).join("ripgrep__main__ripgrep.json"))?;
    const ITERS: u32 = 2000;
    let mut samples = Vec::with_capacity(5);
    for _ in 0..5 {
        let t = std::time::Instant::now();
        for _ in 0..ITERS {
            check("ripgrep", "main", std::hint::black_box(&rg), &layout)
                .map_err(anyhow::Error::msg)?;
        }
        samples.push(t.elapsed().as_secs_f64() * 1e6 / f64::from(ITERS));
    }
    samples.sort_by(f64::total_cmp);
    println!(
        "planejador (resolve + plan_install, ripgrep): {ITERS} iterações x 5 lotes, µs/chamada min={:.1} mediana={:.1} max={:.1}",
        samples[0], samples[2], samples[4]
    );
    Ok(())
}
