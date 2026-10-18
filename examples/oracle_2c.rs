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
/// Approved verbs (`Get-Verb`, PowerShell 7.6.7): `Verb-Noun` with one of them counts as a
/// cmdlet when it is not a Scoop function.
const APPROVED_VERBS: &[&str] = &[
    "Add",
    "Approve",
    "Assert",
    "Backup",
    "Block",
    "Build",
    "Checkpoint",
    "Clear",
    "Close",
    "Compare",
    "Complete",
    "Compress",
    "Confirm",
    "Connect",
    "Convert",
    "ConvertFrom",
    "ConvertTo",
    "Copy",
    "Debug",
    "Deny",
    "Deploy",
    "Disable",
    "Disconnect",
    "Dismount",
    "Edit",
    "Enable",
    "Enter",
    "Exit",
    "Expand",
    "Export",
    "Find",
    "Format",
    "Get",
    "Grant",
    "Group",
    "Hide",
    "Import",
    "Initialize",
    "Install",
    "Invoke",
    "Join",
    "Limit",
    "Lock",
    "Measure",
    "Merge",
    "Mount",
    "Move",
    "New",
    "Open",
    "Optimize",
    "Out",
    "Ping",
    "Pop",
    "Protect",
    "Publish",
    "Push",
    "Read",
    "Receive",
    "Redo",
    "Register",
    "Remove",
    "Rename",
    "Repair",
    "Request",
    "Reset",
    "Resize",
    "Resolve",
    "Restart",
    "Restore",
    "Resume",
    "Revoke",
    "Save",
    "Search",
    "Select",
    "Send",
    "Set",
    "Show",
    "Skip",
    "Split",
    "Start",
    "Step",
    "Stop",
    "Submit",
    "Suspend",
    "Switch",
    "Sync",
    "Test",
    "Trace",
    "Unblock",
    "Undo",
    "Uninstall",
    "Unlock",
    "Unprotect",
    "Unpublish",
    "Unregister",
    "Update",
    "Use",
    "Wait",
    "Watch",
    "Write",
];

/// Built-in aliases (`pwsh -NoProfile`, `Get-Alias`, PowerShell 7.6.7) and built-in
/// functions without a module (`Get-Command -CommandType Function`).
const PWSH_BUILTINS: &[&str] = &[
    "?",
    "%",
    "ac",
    "cat",
    "cd",
    "chdir",
    "clc",
    "clear",
    "clhy",
    "cli",
    "clp",
    "cls",
    "clv",
    "cnsn",
    "compare",
    "copy",
    "cp",
    "cpi",
    "cpp",
    "cvpa",
    "dbp",
    "del",
    "diff",
    "dir",
    "dnsn",
    "ebp",
    "echo",
    "epal",
    "epcsv",
    "erase",
    "etsn",
    "exsn",
    "fc",
    "fhx",
    "fl",
    "foreach",
    "ft",
    "fw",
    "gal",
    "gbp",
    "gc",
    "gci",
    "gcm",
    "gcs",
    "gdr",
    "gerr",
    "ghy",
    "gi",
    "gjb",
    "gl",
    "gm",
    "gmo",
    "gp",
    "gps",
    "gpv",
    "group",
    "gsn",
    "gsv",
    "gu",
    "gv",
    "h",
    "history",
    "icm",
    "iex",
    "ihy",
    "ii",
    "ipal",
    "ipcsv",
    "ipmo",
    "irm",
    "iwr",
    "kill",
    "ls",
    "man",
    "md",
    "measure",
    "mi",
    "mount",
    "move",
    "mp",
    "mv",
    "nal",
    "ndr",
    "ni",
    "nmo",
    "nsn",
    "nv",
    "ogv",
    "oh",
    "popd",
    "ps",
    "pushd",
    "pwd",
    "r",
    "rbp",
    "rcjb",
    "rcsn",
    "rd",
    "rdr",
    "ren",
    "ri",
    "rjb",
    "rm",
    "rmdir",
    "rmo",
    "rni",
    "rnp",
    "rp",
    "rsn",
    "rv",
    "rvpa",
    "sajb",
    "sal",
    "saps",
    "sasv",
    "sbp",
    "select",
    "set",
    "shcm",
    "si",
    "sl",
    "sleep",
    "sls",
    "sort",
    "sp",
    "spjb",
    "spps",
    "spsv",
    "start",
    "sv",
    "tee",
    "type",
    "where",
    "wjb",
    "write",
    "cd..",
    "Clear-Host",
    "help",
    "mkdir",
    "oss",
    "Pause",
    "prompt",
    "TabExpansion2",
];

/// Cmdlets of the `Microsoft.PowerShell.*` modules whose verb is not approved
/// (`Get-Command -Module Microsoft.PowerShell.*`, PowerShell 7.6.7).
const UNAPPROVED_CMDLETS: &[&str] = &[
    "ForEach-Object",
    "Sort-Object",
    "Tee-Object",
    "Where-Object",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Scoop,
    Cmdlet,
    External,
}

impl Kind {
    fn label(self) -> &'static str {
        match self {
            Kind::Scoop => "função do Scoop",
            Kind::Cmdlet => "cmdlet",
            Kind::External => "externo ou definido no script",
        }
    }
}

/// Kind of a lowercase command name: Scoop function first, then pwsh builtin alias /
/// function or `Verb-Noun` with an approved verb, else external program (or a function
/// defined outside the manifest's scripts).
fn kind(lower: &str, scoop_fns: &HashSet<String>) -> Kind {
    if scoop_fns.contains(lower) {
        return Kind::Scoop;
    }
    if PWSH_BUILTINS
        .iter()
        .chain(UNAPPROVED_CMDLETS)
        .any(|b| b.eq_ignore_ascii_case(lower))
    {
        return Kind::Cmdlet;
    }
    match lower.split_once('-') {
        Some((verb, noun))
            if !noun.is_empty() && APPROVED_VERBS.iter().any(|v| v.eq_ignore_ascii_case(verb)) =>
        {
            Kind::Cmdlet
        }
        _ => Kind::External,
    }
}

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
    let kind_of = |lower: &str| kind(lower, &scoop_fns);
    let count_kind = |k: Kind| ranked.iter().filter(|(c, _)| kind_of(c) == k).count();
    println!(
        "\n## Comandos chamados pelos scripts\n\n{} manifestos S-hook/S-ps1; {} comandos distintos: {} funções do Scoop (lista de {}), {} cmdlets, {} externos ou definidos fora do manifesto. Funções definidas pelo próprio manifesto (`function`/`filter`) não entram.\n",
        scripted.len(),
        ranked.len(),
        count_kind(Kind::Scoop),
        SCOOP_FUNCTIONS.len(),
        count_kind(Kind::Cmdlet),
        count_kind(Kind::External),
    );
    println!("| # | comando | manifestos | tipo |");
    println!("|---:|---|---:|---|");
    for (i, (k, (shown, n))) in ranked.iter().enumerate().take(80) {
        println!("| {} | `{shown}` | {n} | {} |", i + 1, kind_of(k).label());
    }
    let list = |k: Kind| -> String {
        ranked
            .iter()
            .filter(|(c, _)| kind_of(c) == k)
            .map(|(_, (shown, n))| format!("`{shown}` ({n})"))
            .collect::<Vec<_>>()
            .join(", ")
    };
    println!("\nTodas as funções do Scoop usadas: {}.", list(Kind::Scoop));
    println!(
        "\nTodos os externos ou definidos fora do manifesto: {}.",
        list(Kind::External)
    );

    // Metric B (decision D1 = B): own prelude for Scoop functions; cmdlets and native
    // commands run in the real pwsh. A manifest is covered when the Scoop functions it
    // calls are all among the N most used ones. S-ps1 is never covered.
    let scoop_ranked: Vec<&str> = ranked
        .iter()
        .filter(|(c, _)| kind_of(c) == Kind::Scoop)
        .map(|(c, _)| c.as_str())
        .collect();
    let uses_scoop = |m: &ScriptManifest| m.commands.iter().any(|c| c == "scoop");
    let n_uses_scoop = scripted.iter().filter(|m| uses_scoop(m)).count();
    let without_scoop: Vec<&ScriptManifest> = scripted.iter().filter(|m| !uses_scoop(m)).collect();
    let covered_b = |set: &[&ScriptManifest], top: &BTreeSet<&str>| {
        set.iter()
            .filter(|m| {
                !m.has_ps1
                    && m.commands
                        .iter()
                        .filter(|c| scoop_fns.contains(c.as_str()))
                        .all(|c| top.contains(c.as_str()))
            })
            .count()
    };
    let all_refs: Vec<&ScriptManifest> = scripted.iter().collect();
    println!(
        "\n## Cobertura acumulada (métrica B: prelúdio próprio com as funções do Scoop; cmdlets e comandos nativos rodam no pwsh real)\n\nManifesto coberto = sem S-ps1 (o conteúdo do `.ps1` não está no manifesto) e todas as funções do Scoop que os seus scripts chamam estão entre as N mais usadas; o que não é função do Scoop é ignorado. Denominador: {} manifestos S-hook/S-ps1 ({} com S-ps1). Usam `scoop` como comando externo: {} manifestos; a última coluna os exclui do denominador.\n",
        scripted.len(),
        scripted.iter().filter(|m| m.has_ps1).count(),
        n_uses_scoop
    );
    println!("| N funções do Scoop | cobertos | cobertos sem os que usam `scoop` |");
    println!("|---|---:|---:|");
    for n in [5usize, 10, 20, usize::MAX] {
        let label = if n == usize::MAX {
            format!("todas ({})", scoop_ranked.len())
        } else {
            n.to_string()
        };
        let top: BTreeSet<&str> = scoop_ranked.iter().take(n).copied().collect();
        println!(
            "| {label} | {} | {} |",
            pct(covered_b(&all_refs, &top), all_refs.len()),
            pct(covered_b(&without_scoop, &top), without_scoop.len())
        );
    }

    // Floor: every command (of any kind) must be in the set of the N most used.
    let all_ranked: Vec<&str> = ranked.iter().map(|(c, _)| c.as_str()).collect();
    let covered_floor = |top: &BTreeSet<&str>| {
        scripted
            .iter()
            .filter(|m| !m.has_ps1 && m.commands.iter().all(|c| top.contains(c.as_str())))
            .count()
    };
    println!("\n## Piso: implementar os N comandos mais usados de qualquer tipo (sem pwsh real)\n");
    println!("| N comandos | cobertos |");
    println!("|---|---:|");
    for n in [5usize, 10, 20, 30, 50, 100, usize::MAX] {
        let label = if n == usize::MAX {
            format!("todos ({})", all_ranked.len())
        } else {
            n.to_string()
        };
        let top: BTreeSet<&str> = all_ranked.iter().take(n).copied().collect();
        println!("| {label} | {} |", pct(covered_floor(&top), scripted.len()));
    }
    Ok(())
}
