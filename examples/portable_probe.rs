//! Portable-mode probe (Task 229): for each winget id, plans it as the CLI would and, when it
//! is an MSI that needs the UAC, downloads it (SHA-256 checked), runs `portable_msi::analyze`
//! and tries our own extraction. Nothing runs: no `msiexec`, no installer. Everything goes
//! under `<work dir>`, which is deleted at the end. Prints TSV.
//!
//! Usage: `cargo run --release --example portable_probe -- <catalog.db> <work dir> [id...]`
#[cfg(windows)]
mod probe {
    use std::collections::BTreeMap;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicBool, Ordering};

    use anyhow::{Context, bail};
    use wingetrs::cache::{BlobStore, DataDir};
    use wingetrs::catalog::Catalog;
    use wingetrs::config::InstallerMode;
    use wingetrs::http::Http;
    use wingetrs::install::progress::ProgressEvent;
    use wingetrs::sources::winget::CDN_BASE;
    use wingetrs::sources::winget::details::merged_manifest;
    use wingetrs::sources::winget::manifest::InstallerKind;
    use wingetrs::winget_install::fetch::fetch_installer_until;
    use wingetrs::winget_install::host_win;
    use wingetrs::winget_install::launch::Elevation;
    use wingetrs::winget_install::plan::{PlanOrBridge, plan_with_mode};
    use wingetrs::winget_install::portable_msi::analyze;
    use wingetrs::winget_install::select::{Refusal, Scope, ScopeWanted};

    /// Larger installers are reported as "grande" and not analyzed.
    const MAX_BYTES: u64 = 500 * 1024 * 1024;

    const IDS: &[&str] = &[
        "7zip.7zip",
        "Microsoft.PowerShell",
        "VideoLAN.VLC",
        "Mozilla.Firefox",
        "KeePassXCTeam.KeePassXC",
        "OpenJS.NodeJS.LTS",
        "GoLang.Go",
        "Amazon.Corretto.21.JDK",
        "EclipseAdoptium.Temurin.21.JDK",
        "Inkscape.Inkscape",
        "TheDocumentFoundation.LibreOffice",
        "Google.Chrome",
        "WiresharkFoundation.Wireshark",
        "Microsoft.AzureCLI",
        "PuTTY.PuTTY",
        "WinSCP.WinSCP",
        "Zoom.Zoom",
        "Notepad++.Notepad++",
        "GIMP.GIMP",
        "Audacity.Audacity",
        "DBeaver.DBeaver.Community",
        "Git.Git",
        "Python.Python.3.12",
        "Docker.DockerDesktop",
        "Microsoft.VisualStudioCode",
    ];

    fn kind_name(k: &InstallerKind) -> String {
        format!("{k:?}").to_lowercase()
    }

    fn clean(s: &str) -> String {
        s.replace(['\t', '\r', '\n'], " ")
    }

    async fn probe(
        http: &Http,
        catalog: &Catalog,
        data: &DataDir,
        id: &str,
    ) -> anyhow::Result<String> {
        let pkg = catalog
            .get("winget", id)?
            .with_context(|| format!("{id} não está no catálogo"))?;
        let hash = pkg
            .winget_hash
            .as_deref()
            .context("sem hash de manifesto")?;
        let blobs = BlobStore::new(data.blobs_dir());
        let (m, _) = merged_manifest(
            http,
            &blobs,
            CDN_BASE,
            &pkg.entry.id,
            hash,
            Some(&pkg.entry.version),
        )
        .await?;
        let plan = match plan_with_mode(
            &m,
            &host_win::current(),
            ScopeWanted::Prefer(Scope::User),
            None,
            InstallerMode::default(),
        ) {
            Ok(PlanOrBridge::Native(p)) => p,
            Ok(PlanOrBridge::Portable(_)) => return Ok(format!("{id}\tportable\t-\tfora")),
            Ok(PlanOrBridge::Bridge(why)) => {
                return Ok(format!("{id}\t-\t-\tfora (ponte: {})", clean(&why)));
            }
            Err(Refusal(why)) => {
                return Ok(format!("{id}\t-\t-\tfora (recusado: {})", clean(&why)));
            }
        };
        let kind = kind_name(&plan.installer.kind);
        let elevation = format!("{:?}", plan.elevation).to_lowercase();
        let msi = matches!(plan.installer.kind, InstallerKind::Msi | InstallerKind::Wix);
        if !msi || plan.elevation != Elevation::Helper {
            return Ok(format!("{id}\t{kind}\t{elevation}\tfora"));
        }

        let big = AtomicBool::new(false);
        let progress = |e: &ProgressEvent| {
            if let ProgressEvent::Download { done, total, .. } = e
                && (*done > MAX_BYTES || total.is_some_and(|t| t > MAX_BYTES))
            {
                big.store(true, Ordering::Relaxed);
            }
        };
        let stop = || big.load(Ordering::Relaxed);
        let fetched = fetch_installer_until(
            http,
            data,
            &plan.id,
            &plan.version,
            &plan.installer,
            &progress,
            &stop,
        )
        .await;
        if big.load(Ordering::Relaxed) {
            return Ok(format!("{id}\t{kind}\t{elevation}\tgrande"));
        }
        let file = fetched?;
        let path = file.path().to_path_buf();
        let mb = std::fs::metadata(&path)?.len() as f64 / (1024.0 * 1024.0);
        let facts = match analyze(&path) {
            Ok(f) => f,
            Err(e) => {
                return Ok(format!(
                    "{id}\t{kind}\t{elevation}\terro\t{}\t\t\t\t{mb:.1}",
                    clean(&e.to_string())
                ));
            }
        };
        let verdict = if facts.blockers.is_empty() {
            "aceito"
        } else {
            "recusado"
        };
        let list = |v: Vec<String>| clean(&v.join("; "));
        let blockers = list(facts.blockers.iter().map(ToString::to_string).collect());
        let losses = list(facts.losses.iter().map(ToString::to_string).collect());
        let extraction = match &facts.main_dir {
            Some(dir) => {
                let out = data.root().join("probe-extract");
                let r = wingetrs::install::extract::extract(
                    &path,
                    &format!("{id}.msi"),
                    Some(&dir.join("\\")),
                    None,
                    &out,
                );
                if out.exists() {
                    std::fs::remove_dir_all(&out)?;
                }
                match r {
                    Ok(()) => "ok".to_string(),
                    Err(e) => clean(&e.to_string()),
                }
            }
            // No main folder: extract everything and show where the files went.
            None => {
                let out = data.root().join("probe-extract");
                let r = wingetrs::install::extract::extract(
                    &path,
                    &format!("{id}.msi"),
                    None,
                    None,
                    &out,
                );
                let folders = r.as_ref().map(|()| top_folders(&out));
                if out.exists() {
                    std::fs::remove_dir_all(&out)?;
                }
                match folders {
                    Ok(f) => format!("sem pasta principal: {f}"),
                    Err(e) => clean(&e.to_string()),
                }
            }
        };
        drop(file);
        remove_installers(data)?;
        Ok(format!(
            "{id}\t{kind}\t{elevation}\t{verdict}\t{blockers}\t{losses}\t{}\t{extraction}\t{mb:.1}",
            facts.per_user_capable
        ))
    }

    /// Files per folder two levels below `root` (`a\b: n`), most files first.
    fn top_folders(root: &Path) -> String {
        let mut counts: BTreeMap<String, usize> = BTreeMap::new();
        let mut stack = vec![root.to_path_buf()];
        while let Some(dir) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for e in entries.flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                } else if let Ok(rel) = p.strip_prefix(root) {
                    let parts: Vec<String> = rel
                        .parent()
                        .into_iter()
                        .flat_map(|d| d.components())
                        .take(2)
                        .map(|c| c.as_os_str().to_string_lossy().into_owned())
                        .collect();
                    *counts.entry(parts.join("\\")).or_default() += 1;
                }
            }
        }
        let mut v: Vec<(String, usize)> = counts.into_iter().collect();
        v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        v.iter()
            .map(|(d, n)| format!("{d}: {n}"))
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// Downloads are only needed for one package at a time.
    fn remove_installers(data: &DataDir) -> anyhow::Result<()> {
        for dir in [data.root().join("installers"), data.blobs_dir()] {
            if dir.exists() {
                std::fs::remove_dir_all(&dir)?;
            }
        }
        Ok(())
    }

    #[tokio::main]
    pub async fn main() -> anyhow::Result<()> {
        let args: Vec<String> = std::env::args().collect();
        let db = args.get(1).context("faltou o catalog.db")?;
        let work = PathBuf::from(args.get(2).context("faltou a pasta de trabalho")?);
        if work.exists() {
            bail!("a pasta de trabalho já existe: {}", work.display());
        }
        let ids: Vec<&str> = if args.len() > 3 {
            args[3..].iter().map(String::as_str).collect()
        } else {
            IDS.to_vec()
        };
        let catalog = Catalog::open_read_only(Path::new(db))?;
        let data = DataDir::new(&work);
        data.ensure()?;
        let http = Http::new()?;
        println!("id\ttipo\televação\tveredito\tbloqueios\tperdas\tper_user_capable\textração\tMB");
        for id in ids {
            match probe(&http, &catalog, &data, id).await {
                Ok(line) => println!("{line}"),
                Err(e) => println!("{id}\t-\t-\terro\t{}", clean(&format!("{e:#}"))),
            }
            remove_installers(&data)?;
        }
        std::fs::remove_dir_all(&work)?;
        Ok(())
    }
}

fn main() -> anyhow::Result<()> {
    #[cfg(windows)]
    return probe::main();
    #[cfg(not(windows))]
    Ok(())
}
