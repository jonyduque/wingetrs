//! Phase 2c oracle (Task 142): classifies every scoop manifest of a local catalog by
//! sub-case (S-psm, S-inno, S-file, S-ps1, S-hook) and ranks the commands its scripts
//! call. Reads the catalog read-only; never runs a script. Prints Markdown.
//!
//! Usage: `cargo run --release --example oracle_2c -- <catalog.db>`
use std::collections::{BTreeMap, BTreeSet, HashSet};

use anyhow::Context;
use rusqlite::{Connection, OpenFlags};
use wingetrs::install::classify_2c::{SubCase, classify, script_commands};

/// Functions defined in Scoop's `lib/*.ps1` (`function <name>`, `global:` prefix dropped),
/// read from ~/scoop/apps/scoop/current/lib at ScoopInstaller/Scoop commit
/// e6aa3b366bdee8ed138c1e0f7b85192ebdd35d0f (2026-09-30).
const SCOOP_FUNCTIONS: &[&str] = &[
    "abort",
    "Add-Path",
    "add_alias",
    "add_bucket",
    "add_first_in_path",
    "all_installed",
    "appdir",
    "applist",
    "appname_from_url",
    "appsdir",
    "apps_in_bucket",
    "app_status",
    "arch_specific",
    "aria_exit_code",
    "basedir",
    "bucketdir",
    "buckets",
    "cache_path",
    "check_hash",
    "check_long_paths",
    "check_main_bucket",
    "check_windows_defender",
    "clean_description",
    "coalesce",
    "commands",
    "command_files",
    "command_name",
    "command_path",
    "Compare-Version",
    "compare_versions",
    "Complete-ConfigChange",
    "Confirm-InstallationStatus",
    "Convert-RepositoryUri",
    "ConvertToPrettyJson",
    "cookie_header",
    "create_shims",
    "create_startmenu_shortcuts",
    "currentdir",
    "current_version",
    "debug",
    "download_json",
    "ensure",
    "ensure_install_dir_not_in_path",
    "ensure_in_psmodulepath",
    "ensure_none_failed",
    "env",
    "env_add_path",
    "env_rm",
    "env_rm_path",
    "env_set",
    "err",
    "error",
    "exec",
    "Expand-7zipArchive",
    "Expand-DarkArchive",
    "Expand-InnoArchive",
    "Expand-MsiArchive",
    "Expand-ZipArchive",
    "Expand-ZstdArchive",
    "extract_dir",
    "extract_to",
    "failed",
    "filesize",
    "file_path",
    "Find-AppBucket",
    "Find-BucketDirectory",
    "Find-HistoricalManifest",
    "Find-HistoricalManifestInCache",
    "Find-HistoricalManifestInGit",
    "Find-ScoopDBItem",
    "find_description",
    "find_dir_or_subdir",
    "find_hash_in_headers",
    "find_hash_in_json",
    "find_hash_in_rdf",
    "find_hash_in_textfile",
    "find_hash_in_xml",
    "find_is",
    "first_para",
    "fname",
    "Format-ArchitectureString",
    "Format-DownloadProgress",
    "format_hash",
    "friendly_path",
    "ftp_file_size",
    "fullpath",
    "generate_user_manifest",
    "Get-AbsolutePath",
    "Get-AppFilePath",
    "Get-CommandPath",
    "Get-DefaultArchitecture",
    "Get-Dependency",
    "Get-Encoding",
    "Get-EnvVar",
    "Get-GitHubToken",
    "Get-HelperPath",
    "Get-InstallationHelper",
    "Get-InstalledVersion",
    "Get-LatestScoopDBRow",
    "Get-LatestVersion",
    "Get-LocalBucket",
    "Get-Manifest",
    "Get-PEMachine",
    "Get-PESubsystem",
    "Get-RelativePath",
    "Get-RemoteFileSize",
    "Get-ScoopDBItem",
    "Get-ShimTarget",
    "Get-SQLite",
    "Get-SupportedArchitecture",
    "Get-UserAgent",
    "Get-VersionSubstitution",
    "Get-WindowsDeveloperModeStatus",
    "getopt",
    "get_app_name",
    "get_app_name_from_shim",
    "get_config",
    "get_filename_from_metalink",
    "get_hash",
    "get_hash_for_app",
    "get_magic_bytes",
    "get_magic_bytes_pretty",
    "get_shim_path",
    "github_ratelimit_reached",
    "handle_special_urls",
    "hash",
    "HashHelper",
    "hash_for_url",
    "html_body",
    "html_text",
    "info",
    "installed",
    "installed_apps",
    "installed_manifest",
    "installer",
    "install_app",
    "install_info",
    "install_psmodule",
    "Invoke-AutoUpdate",
    "Invoke-CachedAria2Download",
    "Invoke-CachedDownload",
    "Invoke-Download",
    "Invoke-ExternalCommand",
    "Invoke-Extraction",
    "Invoke-Git",
    "Invoke-GitLog",
    "Invoke-HookScript",
    "Invoke-Installer",
    "Invoke-ScoopDownload",
    "isFileLocked",
    "is_admin",
    "is_directory",
    "is_in_dir",
    "is_local",
    "is_scoop_outdated",
    "json_path",
    "json_path_legacy",
    "known_buckets",
    "known_bucket_repo",
    "known_bucket_repos",
    "latest_version",
    "link_current",
    "list_aliases",
    "list_buckets",
    "load_cfg",
    "manifest",
    "manifest_path",
    "meta_content",
    "meta_refresh",
    "meta_tags",
    "modulesdir",
    "movedir",
    "my_usage",
    "New-DirectoryJunction",
    "new_issue_msg",
    "nightly_version",
    "normalize_values",
    "Open-ScoopDB",
    "Optimize-SecurityProtocol",
    "Out-UTF8File",
    "parse_app",
    "parse_json",
    "persistdir",
    "persist_data",
    "persist_def",
    "persist_permission",
    "pluralize",
    "PropertyHelper",
    "prune_installed",
    "Publish-EnvVar",
    "qsort",
    "regex_escape",
    "Remove-Path",
    "Remove-ScoopDBItem",
    "remove_from_path",
    "rm_alias",
    "rm_bucket",
    "rm_shim",
    "rm_shims",
    "rm_startmenu_shortcuts",
    "run",
    "sanitary_path",
    "save_installed_manifest",
    "save_install_info",
    "scoop_help",
    "Select-CurrentVersion",
    "Select-LatestScoopDBRow",
    "Set-EnvVar",
    "Set-PESubsystem",
    "Set-ScoopDB",
    "Set-ScoopDBItem",
    "setup_proxy",
    "set_config",
    "shim",
    "shimdir",
    "shim_def",
    "shortcut_folder",
    "Show-DeprecatedWarning",
    "show_app",
    "show_notes",
    "show_suggestions",
    "sort_versions",
    "Split-PathLikeEnvVar",
    "SplitVersion",
    "Start-Download",
    "startmenu_shortcut",
    "strip_ext",
    "strip_filename",
    "strip_fragment",
    "strip_html",
    "strip_path",
    "substitute",
    "success",
    "summary",
    "Test-7zipRequirement",
    "Test-Aria2Enabled",
    "Test-CommandAvailable",
    "Test-GitAvailable",
    "Test-HelperInstalled",
    "Test-LessmsiRequirement",
    "Test-ScoopCoreOnHold",
    "test_running_process",
    "Trace-DownloadProgress",
    "uninstaller",
    "uninstall_psmodule",
    "unlink_current",
    "unlink_persist_data",
    "Update-ManifestProperty",
    "url",
    "url_filename",
    "url_manifest",
    "url_remote_filename",
    "usage",
    "usermanifest",
    "usermanifestsdir",
    "versiondir",
    "versions",
    "warn",
    "warn_on_overwrite",
    "wraptext",
    "Write-DownloadProgress",
    "Write-ManifestToUserCache",
];

#[derive(Default)]
struct Bucket {
    total: usize,
    invalid: usize,
    c2: usize,
    worst: BTreeMap<SubCase, usize>,
    present: BTreeMap<SubCase, usize>,
    no_powershell: usize,
}

struct ScriptManifest {
    has_ps1: bool,
    /// Lowercase command names (PowerShell is case-insensitive).
    commands: Vec<String>,
}

fn pct(n: usize, t: usize) -> String {
    format!("{n} ({:.1}%)", n as f64 * 100.0 / t.max(1) as f64)
}

fn sum_by(buckets: &BTreeMap<String, Bucket>, f: impl Fn(&Bucket) -> usize) -> usize {
    buckets.values().map(f).sum()
}

fn main() -> anyhow::Result<()> {
    let db = std::env::args().nth(1).context("faltou o catalog.db")?;
    let conn = Connection::open_with_flags(
        &db,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    let mut st =
        conn.prepare("SELECT source, raw FROM packages WHERE kind = 'scoop' AND raw IS NOT NULL")?;
    let rows = st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
    let scoop_fns: HashSet<String> = SCOOP_FUNCTIONS
        .iter()
        .map(|f| f.to_ascii_lowercase())
        .collect();

    let mut buckets: BTreeMap<String, Bucket> = BTreeMap::new();
    let mut scripted: Vec<ScriptManifest> = Vec::new();
    // lowercase name -> (first spelling seen, manifests using it)
    let mut ranking: BTreeMap<String, (String, usize)> = BTreeMap::new();
    for r in rows {
        let (source, raw) = r?;
        let name = source.strip_prefix("scoop:").unwrap_or(&source);
        let key = match name {
            "main" | "extras" => name.to_string(),
            _ => "demais".to_string(),
        };
        let b = buckets.entry(key).or_default();
        b.total += 1;
        let c = match classify(&raw) {
            Ok(Some(c)) => c,
            Ok(None) => continue,
            Err(_) => {
                b.invalid += 1;
                continue;
            }
        };
        b.c2 += 1;
        *b.worst.entry(c.worst).or_default() += 1;
        for s in &c.cases {
            *b.present.entry(*s).or_default() += 1;
        }
        if c.worst <= SubCase::File {
            b.no_powershell += 1;
        }
        let has_ps1 = c.cases.contains(&SubCase::Ps1);
        if has_ps1 || c.cases.contains(&SubCase::Hook) {
            let commands: Vec<String> = script_commands(&raw);
            for cmd in &commands {
                ranking
                    .entry(cmd.to_ascii_lowercase())
                    .or_insert_with(|| (cmd.clone(), 0))
                    .1 += 1;
            }
            scripted.push(ScriptManifest {
                has_ps1,
                commands: commands.iter().map(|c| c.to_ascii_lowercase()).collect(),
            });
        }
    }

    let all = Bucket {
        total: sum_by(&buckets, |b| b.total),
        invalid: sum_by(&buckets, |b| b.invalid),
        c2: sum_by(&buckets, |b| b.c2),
        worst: SubCase::ALL
            .iter()
            .map(|s| {
                (
                    *s,
                    sum_by(&buckets, |b| b.worst.get(s).copied().unwrap_or(0)),
                )
            })
            .collect(),
        present: SubCase::ALL
            .iter()
            .map(|s| {
                (
                    *s,
                    sum_by(&buckets, |b| b.present.get(s).copied().unwrap_or(0)),
                )
            })
            .collect(),
        no_powershell: sum_by(&buckets, |b| b.no_powershell),
    };
    let mut table: Vec<(&str, &Bucket)> = ["main", "extras", "demais"]
        .iter()
        .filter_map(|k| buckets.get(*k).map(|b| (*k, b)))
        .collect();
    table.push(("todos", &all));
    let labels: Vec<&str> = SubCase::ALL.iter().map(|s| s.label()).collect();

    println!("# Oráculo da fase 2c\n");
    println!(
        "Catálogo: `{db}` (somente leitura). Manifestos scoop: {}, JSON inválido: {}.\n",
        all.total, all.invalid
    );
    println!("## Manifestos 2c por bucket e pior sub-caso (`worst`)\n");
    println!(
        "| bucket | total | 2c | {} | só S-psm/S-inno/S-file (dos 2c) |",
        labels.join(" | ")
    );
    println!("|---|---:|---:|{}---:|", "---:|".repeat(labels.len()));
    for (name, b) in &table {
        let cols: Vec<String> = SubCase::ALL
            .iter()
            .map(|s| b.worst.get(s).copied().unwrap_or(0).to_string())
            .collect();
        println!(
            "| {name} | {} | {} | {} | {} |",
            b.total,
            pct(b.c2, b.total),
            cols.join(" | "),
            pct(b.no_powershell, b.c2)
        );
    }
    println!("\n## Presença de cada sub-caso (% dos 2c; um manifesto pode ter vários)\n");
    println!("| bucket | {} |", labels.join(" | "));
    println!("|---|{}", "---:|".repeat(labels.len()));
    for (name, b) in &table {
        let cols: Vec<String> = SubCase::ALL
            .iter()
            .map(|s| pct(b.present.get(s).copied().unwrap_or(0), b.c2))
            .collect();
        println!("| {name} | {} |", cols.join(" | "));
    }

    let mut ranked: Vec<(&String, &(String, usize))> = ranking.iter().collect();
    ranked.sort_by(|a, b| b.1.1.cmp(&a.1.1).then_with(|| a.0.cmp(b.0)));
    let is_scoop = |lower: &str| scoop_fns.contains(lower);
    let n_scoop = ranked.iter().filter(|(k, _)| is_scoop(k)).count();
    println!(
        "\n## Comandos chamados pelos scripts\n\n{} manifestos S-hook/S-ps1; {} comandos distintos, {} deles funções do Scoop (lista de {} funções).\n",
        scripted.len(),
        ranked.len(),
        n_scoop,
        SCOOP_FUNCTIONS.len()
    );
    println!("| # | comando | manifestos | tipo |");
    println!("|---:|---|---:|---|");
    for (i, (k, (shown, n))) in ranked.iter().enumerate().take(80) {
        let kind = if is_scoop(k) {
            "função do Scoop"
        } else {
            "cmdlet/outro"
        };
        println!("| {} | `{shown}` | {n} | {kind} |", i + 1);
    }
    let used_scoop: Vec<String> = ranked
        .iter()
        .filter(|(k, _)| is_scoop(k))
        .map(|(_, (shown, n))| format!("`{shown}` ({n})"))
        .collect();
    println!(
        "\nTodas as funções do Scoop usadas: {}.",
        used_scoop.join(", ")
    );

    let covered = |allowed: &BTreeSet<&str>| {
        scripted
            .iter()
            .filter(|m| !m.has_ps1 && m.commands.iter().all(|c| allowed.contains(c.as_str())))
            .count()
    };
    let scoop_ranked: Vec<&str> = ranked
        .iter()
        .filter(|(k, _)| is_scoop(k))
        .map(|(k, _)| k.as_str())
        .collect();
    let all_ranked: Vec<&str> = ranked.iter().map(|(k, _)| k.as_str()).collect();
    println!(
        "\n## Cobertura acumulada\n\nManifesto coberto = sem S-ps1 (o conteúdo do `.ps1` não está no manifesto) e todos os comandos dos seus scripts no conjunto implementado. Denominador: {} manifestos S-hook/S-ps1 ({} com S-ps1). Coluna 2: as N funções do Scoop mais usadas. Coluna 3: os N comandos mais usados de qualquer tipo (funções do Scoop + cmdlets e programas).\n",
        scripted.len(),
        scripted.iter().filter(|m| m.has_ps1).count()
    );
    println!("| N | só funções do Scoop | qualquer comando |");
    println!("|---|---:|---:|");
    for n in [5usize, 10, 20, 30, 50, 100, usize::MAX] {
        let label = if n == usize::MAX {
            "todas".to_string()
        } else {
            n.to_string()
        };
        let fns: BTreeSet<&str> = scoop_ranked.iter().take(n).copied().collect();
        let any: BTreeSet<&str> = all_ranked.iter().take(n).copied().collect();
        println!(
            "| {label} | {} | {} |",
            pct(covered(&fns), scripted.len()),
            pct(covered(&any), scripted.len())
        );
    }
    Ok(())
}
