//! Static gate for manifest scripts (Task 150).
//!
//! Reads a PowerShell script (never runs it) and reports what it does that the
//! wingetrs sandbox cannot allow (`Refuse`) or that needs a stronger confirmation
//! (`ConfirmStrong`). Every script needs at least a plain confirmation (`Confirm`).
//! The same report feeds the dry-run, the confirmation dialog, `show` and the oracle.
use std::collections::BTreeSet;

use super::classify_2c::commands_in;
use super::manifest::{HookKind, InstallerSpec, Resolved};

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
const D_HIDDEN: &str = "possível código escondido em comentário ou here-string";

/// Scans one script. `supported` lists the Scoop functions the wingetrs prelude implements
/// (compared case-insensitively); any other name of [`SCOOP_FUNCTIONS`] is refused.
pub fn scan(script: &str, supported: &[&str]) -> ScriptReport {
    let text = strip_here_strings(&strip_comments(script));
    // Backticks are dropped before matching: `Run`As` and `HK`LM:` mean the same to pwsh.
    let lines: Vec<String> = text
        .lines()
        .map(|l| l.replace('`', "").to_ascii_lowercase())
        .collect();
    let script_lines: Vec<String> = script.lines().map(str::to_ascii_lowercase).collect();
    let commands = commands_in(script);
    let has_cmd = |names: &[&str]| -> Vec<&String> {
        commands
            .iter()
            .filter(|c| names.iter().any(|n| c.eq_ignore_ascii_case(n)))
            .collect()
    };
    let line_of = |name: &str| -> usize {
        let name = name.to_ascii_lowercase();
        let find = |ls: &[String]| ls.iter().position(|l| words(l).any(|w| w == name));
        find(&lines)
            .or_else(|| find(&script_lines))
            .map_or(1, |i| i + 1)
    };

    let mut flags: Vec<Flag> = Vec::new();
    let mut push = |kind: FlagKind, verdict: Verdict, detail: &str, line: usize| {
        let f = Flag {
            kind,
            verdict,
            detail: detail.to_string(),
            line,
        };
        if !flags.contains(&f) {
            flags.push(f);
        }
    };

    // Command-based rules.
    for c in has_cmd(&["sudo", "gsudo"]) {
        push(
            FlagKind::Elevation,
            Verdict::Refuse,
            D_ELEVATION,
            line_of(c),
        );
    }
    for c in has_cmd(&["scoop", "scoop.cmd", "scoop.ps1"]) {
        push(FlagKind::ScoopCommand, Verdict::Refuse, D_SCOOP, line_of(c));
    }
    for c in has_cmd(&["msiexec", "msiexec.exe"]) {
        push(FlagKind::Msiexec, Verdict::Refuse, D_MSIEXEC, line_of(c));
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
            push(
                FlagKind::UnknownScoopFunction,
                Verdict::Refuse,
                &detail,
                line_of(name),
            );
        }
    }
    let calls_reg = !has_cmd(&["reg", "reg.exe"]).is_empty();

    // Line-based rules, on the text without comments and here-string bodies.
    for (i, l) in lines.iter().enumerate() {
        for (kind, verdict, detail) in line_rules(l, calls_reg) {
            push(kind, verdict, detail, i + 1);
        }
    }
    // Fail closed: the comment/here-string stripping may disagree with pwsh about where
    // a comment starts. A refusal rule that matches only in the raw line (comments and
    // here-strings kept) still asks for a strong confirmation.
    let raw_lines = script
        .lines()
        .map(|l| l.replace('`', "").to_ascii_lowercase());
    for (i, raw) in raw_lines.enumerate() {
        let clean = lines
            .get(i)
            .map(|l| line_rules(l, calls_reg))
            .unwrap_or_default();
        for (kind, verdict, detail) in line_rules(&raw, calls_reg) {
            if verdict == Verdict::Refuse && !clean.iter().any(|c| c.0 == kind) {
                let detail = format!("{D_HIDDEN}: {detail}");
                push(FlagKind::HiddenCode, Verdict::ConfirmStrong, &detail, i + 1);
            }
        }
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
fn line_rules(l: &str, calls_reg: bool) -> Vec<(FlagKind, Verdict, &'static str)> {
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
    out
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
