//! Bridge census: for every `winget` package in the catalog (latest version), plans it as the
//! CLI would and prints why it goes native, portable, to the winget bridge, or is refused.
//! Only manifests are downloaded (small YAML); nothing is installed or executed. Everything
//! goes under `<work dir>`, which is deleted at the end. TSV on stdout, totals on stderr.
//!
//! Usage: `cargo run --release --example bridge_census -- <catalog.db> <work dir> [--limit N]`
#[cfg(windows)]
mod probe {
    use std::collections::BTreeMap;
    use std::path::{Path, PathBuf};

    use anyhow::{Context, bail};
    use futures_util::StreamExt;
    use wingetrs::cache::{BlobStore, DataDir};
    use wingetrs::catalog::Catalog;
    use wingetrs::config::InstallerMode;
    use wingetrs::http::Http;
    use wingetrs::sources::winget::CDN_BASE;
    use wingetrs::sources::winget::details::merged_manifest;
    use wingetrs::sources::winget::manifest::{Dependencies, EffectiveInstaller};
    use wingetrs::winget_install::host_win;
    use wingetrs::winget_install::plan::{PlanOrBridge, plan_with_mode};
    use wingetrs::winget_install::select::{Scope, ScopeWanted, choose};

    struct Row {
        id: String,
        outcome: &'static str,
        reason: String,
        kind: String,
        nested: String,
        deps: String,
        extra: String,
        deps_only_pkg: Option<bool>,
        dep_kinds: [bool; 4],
    }

    fn clean(s: &str) -> String {
        s.replace(['\t', '\r', '\n'], " ")
    }

    fn deps_summary(d: &Option<Dependencies>) -> String {
        let Some(d) = d else {
            return "none".to_string();
        };
        let mut s = format!(
            "pkg={};feat={};lib={};ext={}",
            d.package_dependencies.len(),
            d.windows_features.len(),
            d.windows_libraries.len(),
            d.external_dependencies.len()
        );
        if !d.windows_features.is_empty() {
            s.push_str(&format!(";featnames={}", d.windows_features.join(",")));
        }
        if !d.windows_libraries.is_empty() {
            s.push_str(&format!(";libnames={}", d.windows_libraries.join(",")));
        }
        s
    }

    fn fill(row: &mut Row, i: &EffectiveInstaller) {
        row.kind = format!("{:?}", i.kind).to_lowercase();
        row.nested = i
            .nested_kind
            .as_ref()
            .map(|k| format!("{k:?}").to_lowercase())
            .unwrap_or_default();
        row.deps = deps_summary(&i.dependencies);
        if let Some(d) = &i.dependencies {
            row.dep_kinds = [
                !d.package_dependencies.is_empty(),
                !d.windows_features.is_empty(),
                !d.windows_libraries.is_empty(),
                !d.external_dependencies.is_empty(),
            ];
            row.deps_only_pkg = Some(
                !d.package_dependencies.is_empty()
                    && d.windows_features.is_empty()
                    && d.windows_libraries.is_empty()
                    && d.external_dependencies.is_empty(),
            );
        }
    }

    async fn one(http: &Http, blobs: &BlobStore, id: String, ver: String, hash: Vec<u8>) -> Row {
        let mut row = Row {
            id: id.clone(),
            outcome: "error",
            reason: String::new(),
            kind: String::new(),
            nested: String::new(),
            deps: String::new(),
            extra: String::new(),
            deps_only_pkg: None,
            dep_kinds: [false; 4],
        };
        let m = match merged_manifest(http, blobs, CDN_BASE, &id, &hash, Some(&ver)).await {
            Ok((m, _)) => m,
            Err(e) => {
                row.reason = clean(&format!("{e:#}"));
                return row;
            }
        };
        let host = host_win::current();
        let wanted = ScopeWanted::Prefer(Scope::User);
        // Selected installer (also for bridge/refused rows) straight from `choose`.
        if let Ok(sel) = choose(&m.effective(), &host, wanted) {
            fill(&mut row, &sel.installer);
        }
        match plan_with_mode(&m, &host, wanted, None, InstallerMode::default()) {
            Ok(PlanOrBridge::Native(p)) => {
                row.outcome = "native";
                fill(&mut row, &p.installer);
                row.extra = format!(
                    "elevation={};interactive={}",
                    format!("{:?}", p.elevation).to_lowercase(),
                    p.interactive
                );
            }
            Ok(PlanOrBridge::Portable(_)) => row.outcome = "portable",
            Ok(PlanOrBridge::Bridge(why)) => {
                row.outcome = "bridge";
                row.reason = clean(&why);
            }
            Err(e) => {
                row.outcome = "refused";
                row.reason = clean(&e.0);
            }
        }
        row
    }

    /// Deletes the work folder when the probe ends, on every path.
    struct WorkDir(PathBuf);

    impl Drop for WorkDir {
        fn drop(&mut self) {
            if self.0.exists()
                && let Err(e) = std::fs::remove_dir_all(&self.0)
            {
                eprintln!("não apagou {}: {e}", self.0.display());
            }
        }
    }

    fn prefix(reason: &str) -> &str {
        reason.split(':').next().unwrap_or(reason)
    }

    #[tokio::main]
    pub async fn main() -> anyhow::Result<()> {
        let args: Vec<String> = std::env::args().collect();
        let db = args.get(1).context("faltou o catalog.db")?;
        let work = PathBuf::from(args.get(2).context("faltou a pasta de trabalho")?);
        if work.exists() {
            bail!("a pasta de trabalho já existe: {}", work.display());
        }
        let limit: Option<usize> = match args.iter().position(|a| a == "--limit") {
            Some(p) => Some(
                args.get(p + 1)
                    .context("faltou o N de --limit")?
                    .parse()
                    .context("--limit inválido")?,
            ),
            None => None,
        };
        let catalog = Catalog::open_read_only(Path::new(db))?;
        let mut todo: Vec<(String, String, Vec<u8>)> = Vec::new();
        let mut nohash: Vec<String> = Vec::new();
        for e in catalog.load_entries()? {
            if e.source_name != "winget" {
                continue;
            }
            let hash = catalog.get("winget", &e.id)?.and_then(|p| p.winget_hash);
            match hash {
                Some(h) => todo.push((e.id, e.version, h)),
                None => nohash.push(e.id),
            }
        }
        if let Some(n) = limit {
            todo.truncate(n);
        }
        let _cleanup = WorkDir(work.clone());
        let data = DataDir::new(&work);
        data.ensure()?;
        let http = Http::new()?;
        let blobs = BlobStore::new(data.blobs_dir());
        println!("id\toutcome\treason\tkind\tnested\tdeps\textra");
        let started = std::time::Instant::now();
        let mut rows: Vec<Row> = Vec::new();
        for id in nohash {
            println!("{id}\terror\tsem hash de manifesto\t\t\t\t");
            rows.push(Row {
                id,
                outcome: "error",
                reason: "sem hash de manifesto".into(),
                kind: String::new(),
                nested: String::new(),
                deps: String::new(),
                extra: String::new(),
                deps_only_pkg: None,
                dep_kinds: [false; 4],
            });
        }
        let total = todo.len();
        let mut stream = futures_util::stream::iter(todo)
            .map(|(id, ver, hash)| {
                let (http, blobs) = (&http, &blobs);
                async move { one(http, blobs, id, ver, hash).await }
            })
            .buffer_unordered(16);
        while let Some(r) = stream.next().await {
            println!(
                "{}\t{}\t{}\t{}\t{}\t{}\t{}",
                r.id, r.outcome, r.reason, r.kind, r.nested, r.deps, r.extra
            );
            rows.push(r);
            if rows.len().is_multiple_of(500) {
                eprintln!("{}/{total} ({:?})", rows.len(), started.elapsed());
            }
        }
        drop(stream);

        let mut by_outcome: BTreeMap<&str, usize> = BTreeMap::new();
        let mut by_reason: BTreeMap<String, usize> = BTreeMap::new();
        let mut zip_nested: BTreeMap<String, usize> = BTreeMap::new();
        let mut errors: BTreeMap<String, usize> = BTreeMap::new();
        let (mut dep_rows, mut dep_only_pkg) = (0usize, 0usize);
        let mut dep_kind = [0usize; 4];
        for r in &rows {
            *by_outcome.entry(r.outcome).or_default() += 1;
            if r.outcome == "bridge" || r.outcome == "refused" {
                *by_reason
                    .entry(format!("{}: {}", r.outcome, prefix(&r.reason)))
                    .or_default() += 1;
            }
            if r.outcome == "error" {
                *errors.entry(r.reason.clone()).or_default() += 1;
            }
            if r.outcome == "bridge" && r.kind == "zip" {
                let n = if r.nested.is_empty() {
                    "(none)"
                } else {
                    &r.nested
                };
                *zip_nested.entry(n.to_string()).or_default() += 1;
            }
            if r.outcome == "bridge" && r.reason.starts_with("tem dependências") {
                dep_rows += 1;
                if r.deps_only_pkg == Some(true) {
                    dep_only_pkg += 1;
                }
                for (i, k) in r.dep_kinds.iter().enumerate() {
                    if *k {
                        dep_kind[i] += 1;
                    }
                }
            }
        }
        eprintln!(
            "== runtime {:?}, {} packages",
            started.elapsed(),
            rows.len()
        );
        eprintln!("== outcomes");
        for (k, v) in &by_outcome {
            eprintln!("{v}\t{k}");
        }
        eprintln!("== bridge/refused reasons (prefix before ':')");
        let mut v: Vec<_> = by_reason.iter().collect();
        v.sort_by(|a, b| b.1.cmp(a.1));
        for (k, n) in v {
            eprintln!("{n}\t{k}");
        }
        eprintln!("== zip bridge rows by NestedInstallerType");
        for (k, n) in &zip_nested {
            eprintln!("{n}\t{k}");
        }
        eprintln!("== 'tem dependências' rows: {dep_rows}");
        eprintln!("only package deps\t{dep_only_pkg}");
        eprintln!(
            "has pkg\t{}\nhas feat\t{}\nhas lib\t{}\nhas ext\t{}",
            dep_kind[0], dep_kind[1], dep_kind[2], dep_kind[3]
        );
        eprintln!("== errors by message");
        let mut v: Vec<_> = errors.iter().collect();
        v.sort_by(|a, b| b.1.cmp(a.1));
        for (k, n) in v {
            eprintln!("{n}\t{k}");
        }
        Ok(())
    }
}

fn main() -> anyhow::Result<()> {
    #[cfg(windows)]
    return probe::main();
    #[cfg(not(windows))]
    Ok(())
}
