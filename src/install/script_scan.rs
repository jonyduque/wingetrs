//! Static gate for manifest scripts (Task 150).
//!
//! Reads a PowerShell script (never runs it) and reports what it does that the
//! wingetrs sandbox cannot allow (`Refuse`) or that needs a stronger confirmation
//! (`ConfirmStrong`). Every script needs at least a plain confirmation (`Confirm`).
//! The same report feeds the dry-run, the confirmation dialog, `show` and the oracle.
use std::collections::BTreeSet;

use super::classify_2c::{commands_in, normalize_quotes};
use super::manifest::{HookKind, InstallerSpec, Resolved};
use super::prelude::PRELUDE_FUNCTIONS;

/// Functions defined in Scoop's `lib/*.ps1` (`function <name>`, `global:` prefix dropped),
/// read from ~/scoop/apps/scoop/current/lib at ScoopInstaller/Scoop commit
/// e6aa3b366bdee8ed138c1e0f7b85192ebdd35d0f (2026-09-30).
pub const SCOOP_FUNCTIONS: &[&str] = &[
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
/// How much the user must confirm before a script runs. Ordered: the worst flag wins.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Verdict {
    Confirm,
    ConfirmStrong,
    Refuse,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlagKind {
    Elevation,
    Hklm,
    GlobalSwitch,
    SystemWrite,
    UnknownScoopFunction,
    ScoopCommand,
    Msiexec,
    DynamicCode,
    Download,
    RegistryUser,
    ScheduledTaskOrService,
    AbsolutePath,
    /// A refusal rule that matches only with comments and here-strings kept.
    HiddenCode,
    /// Touches the prelude's internals (`__wr_effect`, `$__wr_effects`, ...).
    PreludeInternal,
    /// Redefines or removes a prelude function.
    PreludeRedefinition,
    /// `[Environment]::SetEnvironmentVariable(...)`.
    EnvironmentVariable,
    /// Declares its own `$global` (parameter, `param(...)`, `foreach`), which clashes with
    /// the runner's constant `$global`.
    GlobalName,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Flag {
    pub kind: FlagKind,
    pub verdict: Verdict,
    /// Portuguese, shown to the user.
    pub detail: String,
    /// 1-based line of the original script.
    pub line: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptReport {
    /// From [`commands_in`].
    pub commands: Vec<String>,
    /// `$x` names, lowercase, without `$` (`$env:X` is `"env:x"`).
    pub variables: BTreeSet<String>,
    /// Sorted by line.
    pub flags: Vec<Flag>,
    pub verdict: Verdict,
}

const WRITE_CMDLETS: &[&str] = &[
    "new-item",
    "copy-item",
    "move-item",
    "set-content",
    "add-content",
    "out-file",
    "remove-item",
    "rename-item",
    // Aliases and native copiers.
    "mkdir",
    "md",
    "ni",
    "cp",
    "cpi",
    "copy",
    "mv",
    "mi",
    "move",
    "rm",
    "ri",
    "rd",
    "rmdir",
    "del",
    "erase",
    "ren",
    "rni",
    "sc",
    "ac",
    "robocopy",
    "robocopy.exe",
    "xcopy",
    "xcopy.exe",
];
/// .NET writers, matched as text (`[IO.File]::WriteAllText`, `[System.IO.File]::WriteAllBytes`).
const WRITE_TEXT: &[&str] = &["io.file]::write"];
const SYSTEM_DIRS: &[&str] = &[
    "env:programfiles",
    "env:programw6432",
    "env:commonprogramfiles",
    "env:windir",
    "env:systemroot",
    "c:\\program files",
    "c:\\windows",
];
const REG_CMDLETS: &[&str] = &[
    "set-itemproperty",
    "new-itemproperty",
    "remove-itemproperty",
    "new-item",
    "remove-item",
];
const DOWNLOAD_WORDS: &[&str] = &[
    "invoke-webrequest",
    "iwr",
    "invoke-restmethod",
    "irm",
    "start-bitstransfer",
    "curl",
    "curl.exe",
    "wget",
    "wget.exe",
];
const DOWNLOAD_TEXT: &[&str] = &["net.webclient", "downloadfile", "downloadstring"];
const TASK_WORDS: &[&str] = &[
    "register-scheduledtask",
    "schtasks",
    "schtasks.exe",
    "new-service",
    "sc.exe",
    "set-service",
    "start-service",
    "stop-service",
];
const USER_DIRS: &[&str] = &[
    "env:userprofile",
    "env:appdata",
    "env:localappdata",
    "environment]::getfolderpath",
];

const D_ELEVATION: &str = "pede elevação (RunAs/sudo)";
const D_HKLM: &str = "mexe no registro da máquina (HKLM)";
const D_GLOBAL: &str = "usa -Global (instalação para todos os usuários)";
const D_SYSTEM: &str = "grava em Program Files/Windows";
const D_SCOOP: &str = "chama o comando scoop (instalaria no ~/scoop); não suportado";
const D_MSIEXEC: &str = "chama o msiexec; o wingetrs só usa msiexec /a pela fila própria";
const D_DYNAMIC: &str = "executa código montado na hora (iex, scriptblock, & $var)";
const D_DOWNLOAD: &str = "baixa arquivos da internet por conta própria";
const D_REG_USER: &str = "mexe no registro do usuário (HKCU ou reg)";
const D_TASK: &str = "cria ou mexe em tarefa agendada ou serviço do Windows";
const D_ABSOLUTE: &str = "caminho absoluto fora da pasta do app";
const D_PRELUDE_INTERNAL: &str = "mexe nos internos do prelúdio do wingetrs (__wr)";
const D_ENV_PERSISTENT: &str =
    "grava variável de ambiente do usuário ou da máquina por [Environment]::SetEnvironmentVariable";
const D_ENV_PROCESS: &str = "muda variável de ambiente por [Environment]::SetEnvironmentVariable";
const D_HIDDEN: &str = "possível código escondido em comentário ou here-string";
const D_GLOBAL_NAME: &str =
    "usa `$global` como nome próprio (incompatível com o `$global` fixo do wingetrs)";

/// Detail of a flag found only inside an `if ($global) { ... }` body.
fn global_only(detail: &str) -> String {
    format!("só roda em instalação global; não roda no wingetrs ({detail})")
}

/// Scans one script. `supported` lists the Scoop functions the wingetrs prelude implements
/// (compared case-insensitively); any other name of [`SCOOP_FUNCTIONS`] is refused.
pub fn scan(script: &str, supported: &[&str]) -> ScriptReport {
    // Typographic quotes are string delimiters to pwsh (see `normalize_quotes`).
    let script = &normalize_quotes(script);
    let text = strip_here_strings(&strip_comments(script));
    // `$global` is a constant `$false` in the runner: the bodies of `if ($global) { ... }`
    // never run. Their text is scanned apart and only informs (`Verdict::Confirm`).
    let dead = global_dead_mask(script);
    let (live_text, dead_text) = split_by_mask(&text, &dead);
    let (live_raw, dead_raw) = split_by_mask(script, &dead);
    // Backticks are dropped before matching: `Run`As` and `HK`LM:` mean the same to pwsh.
    let clean = |t: &str| -> Vec<String> {
        t.lines()
            .map(|l| l.replace('`', "").to_ascii_lowercase())
            .collect()
    };
    let lines = clean(&live_text);
    let dead_lines = clean(&dead_text);
    let script_lines: Vec<String> = live_raw.lines().map(str::to_ascii_lowercase).collect();
    let dead_script_lines: Vec<String> = dead_raw.lines().map(str::to_ascii_lowercase).collect();
    let commands = commands_in(script);
    let has_cmd = |names: &[&str]| -> Vec<&String> {
        commands
            .iter()
            .filter(|c| names.iter().any(|n| c.eq_ignore_ascii_case(n)))
            .collect()
    };
    // (1-based line, whether the name appears only inside `if ($global)` bodies).
    let line_of = |name: &str| -> (usize, bool) {
        let name = name.to_ascii_lowercase();
        let find = |ls: &[String]| ls.iter().position(|l| words(l).any(|w| w == name));
        if let Some(i) = find(&lines).or_else(|| find(&script_lines)) {
            return (i + 1, false);
        }
        match find(&dead_lines).or_else(|| find(&dead_script_lines)) {
            Some(i) => (i + 1, true),
            None => (1, false),
        }
    };

    let mut flags: Vec<Flag> = Vec::new();
    // `at` = (1-based line, inside an `if ($global)` body). A flag found only inside those
    // bodies is informational.
    let mut push_at = |kind: FlagKind, verdict: Verdict, detail: &str, at: (usize, bool)| {
        let (verdict, detail) = if at.1 {
            (Verdict::Confirm, global_only(detail))
        } else {
            (verdict, detail.to_string())
        };
        let f = Flag {
            kind,
            verdict,
            detail,
            line: at.0,
        };
        if !flags.contains(&f) {
            flags.push(f);
        }
    };

    // Command-based rules.
    for c in has_cmd(&["sudo", "gsudo"]) {
        push_at(
            FlagKind::Elevation,
            Verdict::Refuse,
            D_ELEVATION,
            line_of(c),
        );
    }
    for c in has_cmd(&["scoop", "scoop.cmd", "scoop.ps1"]) {
        push_at(FlagKind::ScoopCommand, Verdict::Refuse, D_SCOOP, line_of(c));
    }
    for c in has_cmd(&["msiexec", "msiexec.exe"]) {
        push_at(FlagKind::Msiexec, Verdict::Refuse, D_MSIEXEC, line_of(c));
    }
    for name in &commands {
        let Some(canon) = SCOOP_FUNCTIONS
            .iter()
            .find(|f| f.eq_ignore_ascii_case(name))
        else {
            continue;
        };
        if !supported.iter().any(|s| s.eq_ignore_ascii_case(name)) {
            let detail = format!("função do Scoop ainda não suportada: {canon}");
            push_at(
                FlagKind::UnknownScoopFunction,
                Verdict::Refuse,
                &detail,
                line_of(name),
            );
        }
    }
    let calls_reg = !has_cmd(&["reg", "reg.exe"]).is_empty();

    // Line-based rules, on the text without comments and here-string bodies; the live
    // text first, then the `if ($global)` bodies.
    for (clean_lines, raw_text, in_dead) in
        [(&lines, &live_raw, false), (&dead_lines, &dead_raw, true)]
    {
        for (i, l) in clean_lines.iter().enumerate() {
            for (kind, verdict, detail) in line_rules(l, calls_reg) {
                push_at(kind, verdict, &detail, (i + 1, in_dead));
            }
        }
        // Fail closed: the comment/here-string stripping may disagree with pwsh about where
        // a comment starts. A refusal rule that matches only in the raw line (comments and
        // here-strings kept) still asks for a strong confirmation.
        let raw_lines = raw_text
            .lines()
            .map(|l| l.replace('`', "").to_ascii_lowercase());
        for (i, raw) in raw_lines.enumerate() {
            let clean = clean_lines
                .get(i)
                .map(|l| line_rules(l, calls_reg))
                .unwrap_or_default();
            for (kind, verdict, detail) in line_rules(&raw, calls_reg) {
                if verdict == Verdict::Refuse && !clean.iter().any(|c| c.0 == kind) {
                    let detail = format!("{D_HIDDEN}: {detail}");
                    push_at(
                        FlagKind::HiddenCode,
                        Verdict::ConfirmStrong,
                        &detail,
                        (i + 1, in_dead),
                    );
                }
            }
        }
    }
    for line in own_global_lines(&text) {
        push_at(
            FlagKind::GlobalName,
            Verdict::Refuse,
            D_GLOBAL_NAME,
            (line, false),
        );
    }

    flags.sort_by_key(|f| f.line);
    #[rustfmt::skip]
    let verdict = flags.iter().map(|f| f.verdict).max().unwrap_or(Verdict::Confirm);
    ScriptReport {
        commands,
        variables: variables_in(&text),
        flags,
        verdict,
    }
}

/// The line rules matched by one (lowercase, backtick-free) line.
fn line_rules(l: &str, calls_reg: bool) -> Vec<(FlagKind, Verdict, String)> {
    let ws: Vec<&str> = words(l).collect();
    let any = |set: &[&str]| ws.iter().any(|w| set.contains(w));
    let contains = |set: &[&str]| set.iter().any(|s| l.contains(s));
    let mut out = Vec::new();
    if any(&["runas", "-runas"]) {
        out.push((FlagKind::Elevation, Verdict::Refuse, D_ELEVATION));
    }
    let hklm_dotnet = contains(&["registry]::localmachine", "registryhive]::localmachine"])
        || (l.contains("openbasekey") && l.contains("localmachine"));
    if contains(&["hklm:", "hklm\\", "hkey_local_machine"]) || hklm_dotnet {
        out.push((FlagKind::Hklm, Verdict::Refuse, D_HKLM));
    }
    if global_switch(l) {
        out.push((FlagKind::GlobalSwitch, Verdict::Refuse, D_GLOBAL));
    }
    if (any(WRITE_CMDLETS) || contains(WRITE_TEXT)) && contains(SYSTEM_DIRS) {
        out.push((FlagKind::SystemWrite, Verdict::Refuse, D_SYSTEM));
    }
    if any(&["start-process", "saps", "start"]) && any(&["msiexec", "msiexec.exe"]) {
        out.push((FlagKind::Msiexec, Verdict::Refuse, D_MSIEXEC));
    }
    if any(&["iex", "invoke-expression"])
        || l.contains("[scriptblock]::create")
        || invokes_variable(l)
    {
        out.push((FlagKind::DynamicCode, Verdict::ConfirmStrong, D_DYNAMIC));
    }
    if any(DOWNLOAD_WORDS) || contains(DOWNLOAD_TEXT) {
        out.push((FlagKind::Download, Verdict::ConfirmStrong, D_DOWNLOAD));
    }
    let reg_exe = calls_reg && any(&["reg", "reg.exe"]) && any(&["import", "add", "delete"]);
    let reg_cmdlet = any(REG_CMDLETS) && contains(&["hkcu:", "hkey_current_user"]);
    if reg_exe || reg_cmdlet {
        out.push((FlagKind::RegistryUser, Verdict::ConfirmStrong, D_REG_USER));
    }
    if any(TASK_WORDS) {
        out.push((
            FlagKind::ScheduledTaskOrService,
            Verdict::ConfirmStrong,
            D_TASK,
        ));
    }
    if contains(USER_DIRS) || has_absolute_literal(l) {
        out.push((FlagKind::AbsolutePath, Verdict::ConfirmStrong, D_ABSOLUTE));
    }
    let mut out: Vec<(FlagKind, Verdict, String)> = out
        .into_iter()
        .map(|(k, v, d)| (k, v, d.to_string()))
        .collect();
    if l.contains("__wr") {
        out.push((
            FlagKind::PreludeInternal,
            Verdict::Refuse,
            D_PRELUDE_INTERNAL.to_string(),
        ));
    }
    for name in redefined_prelude_functions(l) {
        out.push((
            FlagKind::PreludeRedefinition,
            Verdict::Refuse,
            format!("redefine a função do prelúdio do wingetrs: {name}"),
        ));
    }
    if let Some(persistent) = set_environment_variable(l) {
        let (verdict, detail) = if persistent {
            (Verdict::Refuse, D_ENV_PERSISTENT)
        } else {
            (Verdict::ConfirmStrong, D_ENV_PROCESS)
        };
        out.push((FlagKind::EnvironmentVariable, verdict, detail.to_string()));
    }
    out
}

/// Prelude functions (canonical spelling) that a (lowercase) line redefines or removes:
/// `function <name>`/`filter <name>` (scope prefix such as `global:` ignored), or
/// `function:<name>` used with `Set-Item`/`New-Item`/`Remove-Item` (or their aliases) or
/// as a `$function:<name>`/`${function:<name>}` variable.
fn redefined_prelude_functions(l: &str) -> Vec<&'static str> {
    let canon = |name: &str| -> Option<&'static str> {
        let name = name.rsplit(':').next().unwrap_or(name);
        PRELUDE_FUNCTIONS
            .iter()
            .copied()
            .find(|f| f.eq_ignore_ascii_case(name))
    };
    let mut out = Vec::new();
    let tokens: Vec<&str> = l
        .split(|c: char| c.is_whitespace() || matches!(c, ';' | '{' | '(' | '|'))
        .filter(|t| !t.is_empty())
        .collect();
    for pair in tokens.windows(2) {
        if (pair[0] == "function" || pair[0] == "filter")
            && let Some(f) = canon(pair[1])
        {
            out.push(f);
        }
    }
    let item_cmd = words(l).any(|w| {
        matches!(
            w,
            "set-item" | "new-item" | "remove-item" | "si" | "ni" | "ri" | "rm" | "del" | "erase"
        )
    });
    let mut from = 0;
    while let Some(pos) = l[from..].find("function:") {
        let at = from + pos;
        from = at + "function:".len();
        let name: String = l[from..]
            .chars()
            .take_while(|c| c.is_alphanumeric() || matches!(c, '_' | '-' | ':'))
            .collect();
        let before = &l[..at];
        let variable = before.ends_with('$') || before.ends_with("${");
        if (item_cmd || variable)
            && let Some(f) = canon(&name)
        {
            out.push(f);
        }
    }
    out.sort_unstable();
    out.dedup();
    out
}

/// `Some(persistent)` when a (lowercase) line calls
/// `[Environment]::SetEnvironmentVariable(` / `[System.Environment]::...`: persistent when
/// the third argument names the `User` or `Machine` target or `[EnvironmentVariableTarget]`.
fn set_environment_variable(l: &str) -> Option<bool> {
    let at = l.find("environment]::setenvironmentvariable(")?;
    let args = &l[at + "environment]::setenvironmentvariable(".len()..];
    let mut depth = 0usize;
    let mut quote: Option<char> = None;
    let mut index = 0usize;
    let mut third = String::new();
    for c in args.chars() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => {}
            None => match c {
                '\'' | '"' => quote = Some(c),
                '(' | '[' | '{' => depth += 1,
                ')' if depth == 0 => break,
                ')' | ']' | '}' => depth = depth.saturating_sub(1),
                ',' if depth == 0 => {
                    index += 1;
                    continue;
                }
                _ => {}
            },
        }
        if index == 2 {
            third.push(c);
        }
    }
    let third = third.trim();
    let persistent = [
        "'user'",
        "\"user\"",
        "'machine'",
        "\"machine\"",
        "environmentvariabletarget]",
    ]
    .iter()
    .any(|t| third.contains(t));
    Some(persistent)
}

/// Invisible characters that reorder or hide text on screen (Unicode `Cf`: bidi embeddings,
/// overrides and isolates, zero-width marks, BOM): "Trojan Source".
fn is_hidden_format(c: char) -> bool {
    matches!(
        c,
        '\u{00AD}'
            | '\u{061C}'
            | '\u{180E}'
            | '\u{200B}'..='\u{200F}'
            | '\u{202A}'..='\u{202E}'
            | '\u{2060}'..='\u{2064}'
            | '\u{2066}'..='\u{206F}'
            | '\u{FEFF}'
            | '\u{FFF9}'..='\u{FFFB}'
    )
}

/// Third-party text (a manifest script, file name, argument) made safe to show: control
/// characters (escape sequences) and hidden format characters become a visible `\u{..}`;
/// tabs are kept. Shared by the CLI confirmation and the TUI.
pub fn printable(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if (c.is_control() && c != '\t') || is_hidden_format(c) {
            out.push_str(&format!("\\u{{{:04x}}}", u32::from(c)));
        } else {
            out.push(c);
        }
    }
    out
}

/// Gate report of an `installer.file` ending in `.ps1`: its content is not in the manifest, so
/// the plan cannot scan it (the executor scans the real file before running it).
pub fn ps1_installer_report(file: &str) -> ScriptReport {
    ScriptReport {
        commands: vec![file.to_string()],
        variables: BTreeSet::new(),
        flags: vec![Flag {
            kind: FlagKind::DynamicCode,
            verdict: Verdict::ConfirmStrong,
            detail: "instalador .ps1: conteúdo não analisado".to_string(),
            line: 1,
        }],
        verdict: Verdict::ConfirmStrong,
    }
}

/// One report per script present in the manifest, in [`HookKind`] order.
pub fn scan_manifest(m: &Resolved, supported: &[&str]) -> Vec<(HookKind, ScriptReport)> {
    fn script(s: Option<&InstallerSpec>) -> Option<&str> {
        s.and_then(|i| i.script.as_deref())
    }
    [
        (HookKind::PreInstall, m.hooks.pre_install.as_deref()),
        (HookKind::InstallerScript, script(m.installer.as_ref())),
        (HookKind::PostInstall, m.hooks.post_install.as_deref()),
        (HookKind::PreUninstall, m.hooks.pre_uninstall.as_deref()),
        (HookKind::UninstallerScript, script(m.uninstaller.as_ref())),
        (HookKind::PostUninstall, m.hooks.post_uninstall.as_deref()),
    ]
    .into_iter()
    .filter_map(|(k, s)| s.map(|s| (k, scan(s, supported))))
    .collect()
}

/// First literal argument (positional or `-Name`) of each `Get-EnvVar`/`Set-EnvVar` call,
/// case preserved. Arguments that are variables or expressions are skipped.
pub fn env_var_names(script: &str) -> BTreeSet<String> {
    let script = &normalize_quotes(script);
    let cleaned = strip_here_strings(&strip_comments(script));
    let chars: Vec<char> = cleaned.chars().collect();
    let lower: Vec<char> = chars.iter().map(|c| c.to_ascii_lowercase()).collect();
    let callees: [Vec<char>; 2] = ["get-envvar", "set-envvar"].map(|w| w.chars().collect());
    let mut out = BTreeSet::new();
    let mut i = 0;
    while i < chars.len() {
        let at = callees.iter().find(|w| {
            lower[i..].starts_with(w)
                && (i == 0 || !is_word(chars[i - 1]))
                && chars.get(i + w.len()).is_none_or(|c| !is_word(*c))
        });
        let Some(w) = at else {
            i += 1;
            continue;
        };
        i += w.len();
        // At most a few arguments: switches before the name are skipped.
        for _ in 0..8 {
            let Some((arg, next)) = next_arg(&chars, i) else {
                break;
            };
            i = next;
            match arg {
                Arg::Literal(s) => {
                    out.insert(s);
                    break;
                }
                Arg::Switch(s) if s.eq_ignore_ascii_case("-name") => {
                    if let Some((Arg::Literal(s), next)) = next_arg(&chars, i) {
                        out.insert(s);
                        i = next;
                    }
                    break;
                }
                Arg::Switch(_) => {}
                Arg::Other => break,
            }
        }
    }
    out
}

enum Arg {
    Literal(String),
    Switch(String),
    Other,
}

fn arg_end(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\r' | ';' | '|' | ')' | '}')
}

/// The argument starting after blanks at `i` on the same statement, and the index after it.
fn next_arg(chars: &[char], mut i: usize) -> Option<(Arg, usize)> {
    while i < chars.len() && matches!(chars[i], ' ' | '\t') {
        i += 1;
    }
    let c = *chars.get(i)?;
    if arg_end(c) {
        return None;
    }
    if c == '\'' || c == '"' {
        let start = i + 1;
        let mut j = start;
        while j < chars.len() && chars[j] != c {
            j += 1;
        }
        let s: String = chars[start..j].iter().collect();
        let literal = !(c == '"' && (s.contains('$') || s.contains('`')));
        let arg = if literal { Arg::Literal(s) } else { Arg::Other };
        return Some((arg, j + 1));
    }
    let start = i;
    while i < chars.len() && !arg_end(chars[i]) {
        i += 1;
    }
    let s: String = chars[start..i].iter().collect();
    let arg = match s.chars().next() {
        Some('-') => Arg::Switch(s),
        Some(f) if f.is_alphanumeric() || f == '_' => Arg::Literal(s),
        _ => Arg::Other,
    };
    Some((arg, i))
}

/// Chars that belong to one word for the line rules: a `$var`, a path or a dotted name
/// is one word, so `$dir\curl.exe` is not `curl` and `$iex` is not `iex`.
fn is_word(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '_' | '-' | '.' | '$' | '\\' | '/')
}

/// Chars after which `#` is part of the word, not a comment (same set as the
/// `classify_2c` lexer).
fn continues_word(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '-' | '_' | '.' | '\\' | '/' | ':')
}

/// `-Global` or an abbreviation pwsh accepts (`-Glo`, `-Glob`, `-Globa`), except
/// `-Global:$global` (forwards Scoop's own scope flag, `$false` for the per-user installs
/// wingetrs does; 84 lines of the 2026-10-18 catalog). `$globalx` is another variable.
fn global_switch(line: &str) -> bool {
    let chars: Vec<char> = line.chars().collect();
    (0..chars.len()).any(|i| {
        if chars[i] != '-' || i.checked_sub(1).is_some_and(|p| is_word(chars[p])) {
            return false;
        }
        let end = (i + 1..chars.len())
            .find(|&k| !is_word(chars[k]))
            .unwrap_or(chars.len());
        let tok: String = chars[i..end].iter().collect();
        if !["-glo", "-glob", "-globa", "-global"].contains(&tok.as_str()) {
            return false;
        }
        let rest: String = chars[end..].iter().collect();
        let passthrough = rest
            .strip_prefix(":$global")
            .is_some_and(|r| r.chars().next().is_none_or(|c| !is_word(c)));
        !passthrough
    })
}

/// Words of a (lowercase) line, with leading/trailing dots trimmed.
fn words(line: &str) -> impl Iterator<Item = &str> {
    line.split(|c: char| !is_word(c))
        .map(|w| w.trim_matches('.'))
        .filter(|w| !w.is_empty())
}

/// `& $var` (call operator) or `. $var` (dot-sourcing at command position).
fn invokes_variable(line: &str) -> bool {
    let chars: Vec<char> = line.chars().collect();
    let then_var = |mut j: usize, need_blank: bool| {
        let start = j;
        while j < chars.len() && matches!(chars[j], ' ' | '\t') {
            j += 1;
        }
        (!need_blank || j > start) && chars.get(j) == Some(&'$')
    };
    chars.iter().enumerate().any(|(i, &c)| {
        let prev = i.checked_sub(1).map(|p| chars[p]);
        let next = chars.get(i + 1).copied();
        let call = c == '&' && prev != Some('&') && next != Some('&') && then_var(i + 1, false);
        let dot = c == '.' && {
            let before = chars[..i].iter().rev().find(|c| !matches!(c, ' ' | '\t'));
            before.is_none_or(|b| matches!(b, ';' | '|' | '{' | '(' | '&')) && then_var(i + 1, true)
        };
        call || dot
    })
}

/// A drive path `X:\` not glued to a longer name (`HKCU:\` is not one), or a UNC `\\host`.
fn has_absolute_literal(line: &str) -> bool {
    let chars: Vec<char> = line.chars().collect();
    (0..chars.len()).any(|i| {
        let prev = i.checked_sub(1).map(|p| chars[p]);
        let drive = chars[i].is_ascii_alphabetic()
            && chars.get(i + 1) == Some(&':')
            && chars.get(i + 2) == Some(&'\\')
            && prev.is_none_or(|p| !(p.is_alphanumeric() || matches!(p, '_' | '$' | ':' | '{')));
        let unc = chars[i] == '\\'
            && chars.get(i + 1) == Some(&'\\')
            && chars
                .get(i + 2)
                .is_some_and(|c| c.is_alphanumeric() || matches!(c, '?' | '.'))
            && prev.is_none_or(|p| matches!(p, '\'' | '"' | ' ' | '\t' | '(' | '=' | ','));
        drive || unc
    })
}

/// `$name` and `${name}` variables, lowercase without `$`.
fn variables_in(text: &str) -> BTreeSet<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = BTreeSet::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] != '$' {
            i += 1;
            continue;
        }
        i += 1;
        let name: String = if chars.get(i) == Some(&'{') {
            i += 1;
            let start = i;
            while i < chars.len() && chars[i] != '}' && chars[i] != '\n' {
                i += 1;
            }
            chars[start..i].iter().collect()
        } else {
            let start = i;
            while i < chars.len() && (chars[i].is_alphanumeric() || matches!(chars[i], '_' | ':')) {
                i += 1;
            }
            chars[start..i].iter().collect()
        };
        let name = name.trim_end_matches(':').to_ascii_lowercase();
        if !name.is_empty() {
            out.insert(name);
        }
    }
    out
}

/// The script with `#` and `<# #>` comments blanked (newlines kept, so line numbers hold).
/// Strings and here-strings are kept as they are.
fn strip_comments(script: &str) -> String {
    blank(script, true, false)
}

/// The text with here-string bodies blanked (newlines kept). Quoted strings are kept.
fn strip_here_strings(text: &str) -> String {
    blank(text, false, true)
}

/// Copies `src`, blanking comments and/or here-string bodies with spaces (newlines kept).
fn blank(src: &str, comments: bool, here_strings: bool) -> String {
    let chars: Vec<char> = src.chars().collect();
    let n = chars.len();
    let mut out = String::with_capacity(src.len());
    let put = |out: &mut String, c: char, blank: bool| {
        out.push(if blank && c != '\n' && c != '\r' {
            ' '
        } else {
            c
        });
    };
    let mut i = 0;
    while i < n {
        let c = chars[i];
        let prev = i.checked_sub(1).map(|p| chars[p]);
        match c {
            '`' => {
                out.extend(&chars[i..(i + 2).min(n)]);
                i += 2;
            }
            '@' if here_string_start(&chars, i) => {
                let quote = chars[i + 1];
                out.push('@');
                out.push(quote);
                i += 2;
                // Body up to a line starting (after blanks) with quote + '@'.
                while i < n {
                    if chars[i] == '\n' {
                        let mut j = i + 1;
                        while j < n && matches!(chars[j], ' ' | '\t') {
                            j += 1;
                        }
                        if chars.get(j) == Some(&quote) && chars.get(j + 1) == Some(&'@') {
                            out.extend(&chars[i..j + 2]);
                            i = j + 2;
                            break;
                        }
                    }
                    put(&mut out, chars[i], here_strings);
                    i += 1;
                }
            }
            '\'' | '"' => {
                out.push(c);
                i += 1;
                while i < n {
                    let d = chars[i];
                    if c == '"' && d == '`' {
                        out.extend(&chars[i..(i + 2).min(n)]);
                        i += 2;
                        continue;
                    }
                    out.push(d);
                    i += 1;
                    if d == c {
                        if c == '\'' && chars.get(i) == Some(&'\'') {
                            out.push('\'');
                            i += 1;
                            continue;
                        }
                        break;
                    }
                }
            }
            '<' if chars.get(i + 1) == Some(&'#') => {
                let end = (i + 2..n)
                    .find(|&k| chars[k] == '#' && chars.get(k + 1) == Some(&'>'))
                    .map_or(n, |k| k + 2);
                for &d in &chars[i..end] {
                    put(&mut out, d, comments);
                }
                i = end;
            }
            // As in pwsh, `#` opens a comment unless it continues a word (`a#b`).
            '#' if prev.is_none_or(|p| !continues_word(p)) => {
                while i < n && chars[i] != '\n' {
                    put(&mut out, chars[i], comments);
                    i += 1;
                }
            }
            _ => {
                out.push(c);
                i += 1;
            }
        }
    }
    out
}

/// `@'` or `@"` followed only by blanks up to the end of the line (same rule as the
/// private `here_string_start` of `classify_2c`).
fn here_string_start(chars: &[char], i: usize) -> bool {
    if !matches!(chars.get(i + 1), Some('\'' | '"')) {
        return false;
    }
    let mut j = i + 2;
    while j < chars.len() && matches!(chars[j], ' ' | '\t' | '\r') {
        j += 1;
    }
    j >= chars.len() || chars[j] == '\n'
}
/// `src` split in two copies of the same length (newlines kept): the chars outside the
/// mask (others blanked) and the chars inside it. `mask` is per char of `src`.
fn split_by_mask(src: &str, mask: &[bool]) -> (String, String) {
    let mut live = String::with_capacity(src.len());
    let mut dead = String::with_capacity(src.len());
    for (i, c) in src.chars().enumerate() {
        let in_mask = mask.get(i).copied().unwrap_or(false);
        let keep = |out: &mut String, yes: bool| {
            out.push(if yes || c == '\n' || c == '\r' {
                c
            } else {
                ' '
            });
        };
        keep(&mut live, !in_mask);
        keep(&mut dead, in_mask);
    }
    (live, dead)
}

/// Per char of `script`: inside the body (between the braces) of an `if ($global) { ... }`
/// or `elseif ($global) { ... }` whose bounds are certain (see [`global_block`]).
fn global_dead_mask(script: &str) -> Vec<bool> {
    let chars: Vec<char> = script.chars().collect();
    // Same length as `chars` (`blank` maps char to char).
    let uncommented: Vec<char> = strip_comments(script).chars().collect();
    let n = chars.len();
    let mut mask = vec![false; n];
    let mut last_close: Option<usize> = None;
    let mut i = 0;
    // Same tokenizing as `blank`: the keyword counts only in code.
    while i < n {
        let c = chars[i];
        let prev = i.checked_sub(1).map(|p| chars[p]);
        match c {
            '`' => i += 2,
            '@' if here_string_start(&chars, i) => {
                let quote = chars[i + 1];
                i += 2;
                while i < n {
                    if chars[i] == '\n' {
                        let mut j = i + 1;
                        while j < n && matches!(chars[j], ' ' | '\t') {
                            j += 1;
                        }
                        if chars.get(j) == Some(&quote) && chars.get(j + 1) == Some(&'@') {
                            i = j + 2;
                            break;
                        }
                    }
                    i += 1;
                }
            }
            '\'' | '"' => i = string_end(&chars, i).unwrap_or(n),
            '<' if chars.get(i + 1) == Some(&'#') => {
                i = (i + 2..n)
                    .find(|&k| chars[k] == '#' && chars.get(k + 1) == Some(&'>'))
                    .map_or(n, |k| k + 2);
            }
            '#' if prev.is_none_or(|p| !continues_word(p)) => {
                while i < n && chars[i] != '\n' {
                    i += 1;
                }
            }
            _ => {
                let starts_word = prev.is_none_or(|p| !continues_word(p) && p != '$');
                let keyword = ["if", "elseif"].into_iter().find(|k| {
                    let end = i + k.len();
                    end <= n
                        && chars[i..end]
                            .iter()
                            .collect::<String>()
                            .eq_ignore_ascii_case(k)
                        && chars.get(end).is_none_or(|c| !is_word(*c))
                });
                // Statement position only: after a command name, `&`, `=`, `,`, `(`, `|` or a
                // `}` on the same line, `if` is an argument and the `{...}` after it is a live
                // scriptblock literal (`& { ... } if ($global) { ... }` runs it).
                let before = (0..i)
                    .rev()
                    .find(|&k| !matches!(chars[k], ' ' | '\t' | '\r'));
                // Checked on the raw text and on the comment-blanked text; both must agree
                // (a backtick inside a comment is no continuation to pwsh, but on doubt the
                // block stays live).
                let statement = match keyword {
                    Some("if") => {
                        statement_position(&chars, i) && statement_position(&uncommented, i)
                    }
                    // `elseif` only right after the `}` of a block this mask covers.
                    Some(_) => before.is_some() && before == last_close,
                    None => false,
                };
                match keyword.filter(|_| starts_word && statement) {
                    Some(k) => match global_block(&chars, i + k.len()) {
                        Some((open, close)) => {
                            mask[open + 1..close].iter_mut().for_each(|m| *m = true);
                            last_close = Some(close);
                            i = close + 1;
                        }
                        None => i += k.len(),
                    },
                    None => i += 1,
                }
            }
        }
    }
    mask
}

/// Index just past the end of the quoted string opening at `i` (`''` and backtick escapes
/// as in `blank`), or `None` when it never closes.
fn string_end(chars: &[char], i: usize) -> Option<usize> {
    let q = chars[i];
    let mut k = i + 1;
    while k < chars.len() {
        let d = chars[k];
        if q == '"' && d == '`' {
            k += 2;
            continue;
        }
        k += 1;
        if d == q {
            if q == '\'' && chars.get(k) == Some(&'\'') {
                k += 1;
                continue;
            }
            return Some(k);
        }
    }
    None
}

/// After `if`/`elseif` at `j`: `(open, close)` brace indices of the block when the condition
/// is exactly `$global` or starts with `$global -and ` and the bounds are certain. `None`
/// (the block is scanned as live code) on any doubt: another condition (`-or`, `-not`, `!`,
/// extra parentheses, quotes), opening brace neither on the condition's line nor the next,
/// unbalanced braces, a brace inside a string or comment, a here-string or block comment.
fn global_block(chars: &[char], j: usize) -> Option<(usize, usize)> {
    let n = chars.len();
    let skip_blank = |mut k: usize| {
        while k < n && matches!(chars[k], ' ' | '\t') {
            k += 1;
        }
        k
    };
    let j = skip_blank(j);
    if chars.get(j) != Some(&'(') {
        return None;
    }
    let close_paren = (j + 1..n)
        .take_while(|&k| chars[k] != '\n')
        .find(|&k| chars[k] == ')')?;
    let cond: String = chars[j + 1..close_paren]
        .iter()
        .collect::<String>()
        .trim()
        .to_ascii_lowercase();
    let rest = cond.strip_prefix("$global")?;
    let and = rest
        .strip_prefix(char::is_whitespace)
        .map(str::trim_start)
        .and_then(|r| r.strip_prefix("-and"))
        .is_some_and(|r| r.starts_with(char::is_whitespace));
    if !(rest.is_empty() || and) {
        return None;
    }
    let banned = ['(', '!', '\'', '"', '`', '|', ';', '{', '}', '#', '@'];
    if cond.contains(banned) || ["-or", "-xor", "-not"].iter().any(|op| cond.contains(op)) {
        return None;
    }
    let mut k = skip_blank(close_paren + 1);
    if chars.get(k) == Some(&'\r') {
        k += 1;
    }
    if chars.get(k) == Some(&'\n') {
        k = skip_blank(k + 1);
    }
    if chars.get(k) != Some(&'{') {
        return None;
    }
    let open = k;
    let mut depth = 0usize;
    while k < n {
        let c = chars[k];
        let prev = k.checked_sub(1).map(|p| chars[p]);
        match c {
            '`' => {
                if matches!(chars.get(k + 1), Some('{' | '}')) {
                    return None;
                }
                k += 2;
                continue;
            }
            '\'' | '"' => {
                let end = string_end(chars, k)?;
                if chars[k..end].iter().any(|d| matches!(d, '{' | '}')) {
                    return None;
                }
                k = end;
                continue;
            }
            '@' if here_string_start(chars, k) => return None,
            '<' if chars.get(k + 1) == Some(&'#') => return None,
            // `a#b` is a word to pwsh; no guessing about what follows.
            '#' if prev.is_some_and(continues_word) => return None,
            '#' => {
                while k < n && chars[k] != '\n' {
                    if matches!(chars[k], '{' | '}') {
                        return None;
                    }
                    k += 1;
                }
                continue;
            }
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some((open, k));
                }
            }
            _ => {}
        }
        k += 1;
    }
    None
}

/// 1-based lines of `text` (comments and here-strings blanked) where the script declares
/// its own `$global`: `foreach ($global ...`, or a `param(...)` or `function`/`filter`
/// parameter list naming `$global` (or `${global}`).
fn own_global_lines(text: &str) -> Vec<usize> {
    let chars: Vec<char> = text
        .chars()
        .filter(|c| *c != '`')
        .map(|c| c.to_ascii_lowercase())
        .collect();
    let n = chars.len();
    let line_at = |k: usize| chars[..k].iter().filter(|c| **c == '\n').count() + 1;
    let skip_ws = |mut k: usize| {
        while k < n && chars[k].is_whitespace() {
            k += 1;
        }
        k
    };
    // `$global`/`${global}` at `k` (not `$globalx`, not `$global:x`).
    let global_at = |k: usize| -> bool {
        let s: String = chars[k..(k + 9).min(n)].iter().collect();
        s.starts_with("${global}")
            || (s.starts_with("$global")
                && chars
                    .get(k + 7)
                    .is_none_or(|c| !(c.is_alphanumeric() || matches!(c, '_' | ':'))))
    };
    // Index of the `)` matching the `(` at `k` (quotes skipped), or the end.
    let paren_end = |k: usize| -> usize {
        let mut depth = 0usize;
        let mut m = k;
        while m < n {
            match chars[m] {
                '\'' | '"' => {
                    m = string_end(&chars, m).unwrap_or(n);
                    continue;
                }
                '(' => depth += 1,
                ')' => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        return m;
                    }
                }
                _ => {}
            }
            m += 1;
        }
        n
    };
    let mut out = Vec::new();
    let mut i = 0;
    while i < n {
        let prev = i.checked_sub(1).map(|p| chars[p]);
        if prev.is_some_and(|p| continues_word(p) || p == '$') {
            i += 1;
            continue;
        }
        let word_end = (i..n)
            .find(|&k| !(chars[k].is_alphanumeric() || chars[k] == '_'))
            .unwrap_or(n);
        let word: String = chars[i..word_end].iter().collect();
        let open = match word.as_str() {
            "foreach" | "param" => Some(skip_ws(word_end)),
            "function" | "filter" => {
                // Skip the name (scope prefix included) up to its parameter list.
                let name = skip_ws(word_end);
                let after = (name..n)
                    .find(|&k| chars[k].is_whitespace() || matches!(chars[k], '(' | '{'))
                    .unwrap_or(n);
                Some(skip_ws(after))
            }
            _ => None,
        };
        if let Some(open) = open.filter(|&o| chars.get(o) == Some(&'(')) {
            let span = if word == "foreach" {
                let first = skip_ws(open + 1);
                first..(first + 1).min(n)
            } else {
                open + 1..paren_end(open)
            };
            if let Some(k) = span.into_iter().find(|&k| chars[k] == '$' && global_at(k)) {
                out.push(line_at(k));
            }
        }
        i = word_end.max(i + 1);
    }
    out
}

/// Whether a keyword at `i` of `view` starts a statement: the previous char (blanks and CR
/// skipped) is the start, `;`, `{`, or a newline that does not continue the line before it
/// (backtick continuation, or a trailing `,`, `|`, `=`, `(`, `&`, `.`, `[` or operator char
/// that lets pwsh read on: fail closed).
fn statement_position(view: &[char], i: usize) -> bool {
    let prev_sig = |end: usize| {
        (0..end.min(view.len()))
            .rev()
            .find(|&k| !matches!(view[k], ' ' | '\t' | '\r'))
    };
    match prev_sig(i) {
        None => true,
        Some(k) if matches!(view[k], ';' | '{') => true,
        Some(k) if view[k] == '\n' => prev_sig(k).is_none_or(|j| {
            !matches!(
                view[j],
                '`' | ',' | '|' | '=' | '(' | '&' | '.' | '[' | '+' | '-' | '*' | '/' | '%' | '!'
            )
        }),
        Some(_) => false,
    }
}
