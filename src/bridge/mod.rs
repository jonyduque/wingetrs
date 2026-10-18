//! Upgrade bridge: apps installed by winget or scoop are upgraded through their own CLI.
//! Everything here is pure (rules, command lines, exit codes, output splitting); only
//! `spawn_win` starts a process.
use crate::installed::correlate::{Correlation, InstallScope, ListRow, UpdateState};
use crate::sources::winget::manifest::{Installer, MergedManifest};
use std::collections::{BTreeSet, VecDeque};
use std::path::{Path, PathBuf};

#[cfg(windows)]
pub mod elevate_win;
pub mod elevated;
#[cfg(windows)]
pub mod spawn_win;
pub mod worker;

pub const PINNED_NOTE: &str = "versão fixada";
pub const NOT_OURS_NOTE: &str = "instalado fora do wingetrs";

/// Runner of a bridge command: `(program, args, on_line)` -> exit code and last lines.
pub type BridgeExec<'a> =
    &'a mut dyn FnMut(&str, &[String], &mut dyn FnMut(&str)) -> std::io::Result<(i32, Vec<String>)>;

/// Lines kept for the error log.
pub const TAIL_LINES: usize = 40;
/// Longest line kept, in bytes.
pub const LINE_MAX: usize = 1024;
/// Longest unterminated line buffered; the rest of it is dropped until the next `\n`.
pub const PARTIAL_MAX: usize = 64 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Winget,
    Scoop,
}

impl Tool {
    pub fn name(self) -> &'static str {
        match self {
            Tool::Winget => "winget",
            Tool::Scoop => "scoop",
        }
    }
}

/// One upgrade run by the tool that installed the app.
#[derive(Debug, Clone, PartialEq)]
pub struct BridgeJob {
    pub tool: Tool,
    /// winget package id, or scoop app name.
    pub target: String,
    /// Name of the list row.
    pub name: String,
    /// Installed version of the list row, for the job log.
    pub installed: Option<String>,
    /// Version the catalog offers, for the job log and the winget check.
    pub available: Option<String>,
    /// Who the app was installed for; a machine-wide winget app upgrades elevated.
    pub scope: Option<InstallScope>,
}

/// Whether `r` is upgraded through winget (ARP/MSIX matched in the winget catalog) or scoop
/// (installed by scoop); `Err` carries the reason, as the note of a disabled menu item.
pub fn bridge_target(r: &ListRow, pins: &BTreeSet<String>) -> Result<BridgeJob, &'static str> {
    if !matches!(r.origin.as_str(), "arp" | "msix" | "scoop") {
        return Err(NOT_OURS_NOTE);
    }
    if crate::pins::is_pinned(pins, &r.origin, &r.name) {
        return Err(PINNED_NOTE);
    }
    if !matches!(r.state, UpdateState::Available { .. }) {
        return Err("sem atualização");
    }
    let (tool, target) = match (r.origin.as_str(), &r.correlation) {
        ("scoop", _) => (Tool::Scoop, r.name.clone()),
        (
            _,
            Correlation::Matched {
                source, package_id, ..
            },
        ) if source == "winget" => (Tool::Winget, package_id.clone()),
        _ => return Err(NOT_OURS_NOTE),
    };
    let available = match &r.state {
        UpdateState::Available { latest } => Some(latest.clone()),
        _ => None,
    };
    Ok(BridgeJob {
        tool,
        target,
        name: r.name.clone(),
        installed: r.installed_version.clone(),
        available,
        scope: r.scope,
    })
}

/// Whether winget would take the upgrade to a manifest (`winget_applicability`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Applicability {
    Ok,
    /// Every installer for this machine says `UpgradeBehavior: deny`.
    Denied,
    /// Every installer for this machine installs for the other scope.
    ScopeMismatch {
        installed: InstallScope,
        offered: InstallScope,
    },
    /// No installer for this machine's architecture: decided as today.
    Unknown,
}

/// `Applicability` plus `RequireExplicitUpgrade`, which keeps the app out of batches only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WingetFit {
    pub applicability: Applicability,
    pub explicit_only: bool,
}

/// winget architecture name of this machine.
pub fn host_arch() -> &'static str {
    match std::env::consts::ARCH {
        "aarch64" => "arm64",
        "x86" => "x86",
        _ => "x64",
    }
}

/// Installer architectures that run on `arch`, as `install::manifest::Arch::preference`
/// (plus winget's `neutral`).
fn runs_on(installer: &str, arch: &str) -> bool {
    let ok: &[&str] = match arch {
        "x64" => &["x64", "x86"],
        "x86" => &["x86"],
        "arm64" => &["arm64", "x64", "x86"],
        _ => &[],
    };
    installer.eq_ignore_ascii_case("neutral")
        || installer.eq_ignore_ascii_case(arch)
        || ok.iter().any(|a| installer.eq_ignore_ascii_case(a))
}

fn parse_scope(s: Option<&str>) -> Option<InstallScope> {
    match s {
        Some(s) if s.eq_ignore_ascii_case("machine") => Some(InstallScope::Machine),
        Some(s) if s.eq_ignore_ascii_case("user") => Some(InstallScope::User),
        _ => None,
    }
}

/// Whether `winget upgrade` would take `manifest` for an app installed with `scope` on an
/// `arch` machine. Installer fields win over the root ones.
// Rules of winget-cli src/AppInstallerCLICore/Workflows/UpdateFlow.cpp (deny, explicit
// upgrade) and src/AppInstallerCLICore/Workflows/ManifestComparator.cpp (installed scope).
pub fn winget_applicability(
    manifest: &MergedManifest,
    scope: Option<InstallScope>,
    arch: &str,
) -> WingetFit {
    let candidates: Vec<_> = manifest
        .installers
        .iter()
        .filter(|i| runs_on(&i.architecture, arch))
        .collect();
    let explicit_only = candidates.iter().any(|i| {
        i.require_explicit_upgrade
            .or(manifest.require_explicit_upgrade)
            .unwrap_or(false)
    });
    let verdict = |applicability| WingetFit {
        applicability,
        explicit_only,
    };
    if candidates.is_empty() {
        return verdict(Applicability::Unknown);
    }
    let allowed: Vec<_> = candidates
        .into_iter()
        .filter(|i| {
            !i.upgrade_behavior
                .as_deref()
                .or(manifest.upgrade_behavior.as_deref())
                .is_some_and(|b| b.eq_ignore_ascii_case("deny"))
        })
        .collect();
    if allowed.is_empty() {
        return verdict(Applicability::Denied);
    }
    let offered = |i: &&Installer| parse_scope(i.scope.as_deref().or(manifest.scope.as_deref()));
    // An installer without a scope fits either.
    if let Some(installed) = scope
        && !allowed
            .iter()
            .any(|i| offered(i).is_none_or(|s| s == installed))
        && let Some(offered) = allowed.iter().find_map(offered)
    {
        return verdict(Applicability::ScopeMismatch { installed, offered });
    }
    verdict(Applicability::Ok)
}

/// Readable install scope, for the notes.
pub fn scope_label(s: InstallScope) -> &'static str {
    match s {
        InstallScope::Machine => "máquina",
        InstallScope::User => "usuário",
    }
}

/// The note of a disabled "upgrade via winget" item; `None` when winget would take it.
pub fn applicability_note(a: Applicability) -> Option<String> {
    match a {
        Applicability::Denied => Some("o editor não permite atualizar pelo winget".to_string()),
        Applicability::ScopeMismatch { installed, offered } => Some(format!(
            "a versão nova só instala para {}; a instalada é para {}",
            scope_label(offered),
            scope_label(installed)
        )),
        Applicability::Ok | Applicability::Unknown => None,
    }
}

/// `^[A-Za-z0-9][A-Za-z0-9._+-]*$`, at most 128 bytes: the target goes into a command line
/// (and, for scoop, into a PowerShell script), so a leading `-`, quotes, spaces and
/// separators are refused.
pub fn valid_target(_tool: Tool, s: &str) -> bool {
    let mut bytes = s.bytes();
    s.len() <= 128
        && bytes.next().is_some_and(|c| c.is_ascii_alphanumeric())
        && bytes.all(|c| c.is_ascii_alphanumeric() || b"._+-".contains(&c))
}

/// `<root>\apps\scoop\current\bin\scoop.ps1`.
pub fn scoop_ps1(scoop_root: &Path) -> PathBuf {
    scoop_root
        .join("apps")
        .join("scoop")
        .join("current")
        .join("bin")
        .join("scoop.ps1")
}

/// Program and arguments of the upgrade (task 105 probe, §4.1).
pub fn command(job: &BridgeJob, scoop_root: &Path, pwsh: &str) -> (String, Vec<String>) {
    match job.tool {
        Tool::Winget => {
            let args = [
                "upgrade",
                "--id",
                &job.target,
                "--exact",
                "--source",
                "winget",
                "--silent",
                "--disable-interactivity",
                "--accept-package-agreements",
                "--accept-source-agreements",
            ];
            (
                "winget".to_string(),
                args.into_iter().map(str::to_string).collect(),
            )
        }
        Tool::Scoop => {
            let ps1 = ps_single_quoted(&scoop_ps1(scoop_root).display().to_string());
            let script = format!(
                "[Console]::OutputEncoding = [Text.UTF8Encoding]::new($false); & '{ps1}' update {}; exit $LASTEXITCODE",
                job.target
            );
            let args = ["-NoProfile", "-ExecutionPolicy", "Bypass", "-Command"];
            let mut args: Vec<String> = args.into_iter().map(str::to_string).collect();
            args.push(script);
            (pwsh.to_string(), args)
        }
    }
}

/// Program and arguments that refresh winget's own copy of its catalog.
pub fn winget_source_update() -> (String, Vec<String>) {
    let args = [
        "source",
        "update",
        "--name",
        "winget",
        "--disable-interactivity",
    ];
    (
        "winget".to_string(),
        args.into_iter().map(str::to_string).collect(),
    )
}

/// `s` escaped for a PowerShell single-quoted string: every quote PowerShell reads as `'`
/// (ASCII and the typographic U+2018..U+201B) is doubled.
fn ps_single_quoted(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if matches!(c, '\'' | '\u{2018}' | '\u{2019}' | '\u{201A}' | '\u{201B}') {
            out.push(c);
        }
        out.push(c);
    }
    out
}

/// The command as the user reads it in the confirmation (arguments with spaces in quotes).
pub fn command_text(program: &str, args: &[String]) -> String {
    std::iter::once(program.to_string())
        .chain(args.iter().map(|a| {
            if a.contains(char::is_whitespace) {
                format!("\"{a}\"")
            } else {
                a.clone()
            }
        }))
        .collect::<Vec<_>>()
        .join(" ")
}

/// `pwsh` when `pwsh.exe` is in one of the `PATH` directories, else `powershell` (like
/// scoop's own `scoop.cmd`).
pub fn find_pwsh(path: Option<&std::ffi::OsStr>) -> String {
    let found =
        path.is_some_and(|p| std::env::split_paths(p).any(|d| d.join("pwsh.exe").is_file()));
    if found { "pwsh" } else { "powershell" }.to_string()
}

/// Installed version of a scoop app (`<root>\apps\<app>\current\manifest.json`); `app` must
/// have passed `valid_target`.
pub fn scoop_version(scoop_root: &Path, app: &str) -> Option<String> {
    let path = scoop_root
        .join("apps")
        .join(app)
        .join("current")
        .join("manifest.json");
    let json: serde_json::Value = serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
    json.get("version")?.as_str().map(str::to_string)
}

/// Whether winget's exit code means the package was upgraded.
pub fn winget_updated(code: i32) -> bool {
    code == 0 || code as u32 == 0x8A15_0109
}

/// Texts of `InstallFlowReturnCodeRebootRequiredToFinish` in the 11 locales winget ships
/// (Localization/Resources/<locale>/winget.resw, winget-cli commit 3973956; task 113 report).
/// winget exits 0 and prints one of these as its last line, never 0x8A150109 (task 113).
pub const REBOOT_MARKERS: [&str; 11] = [
    "Restart your PC to finish installation.",
    "Starten Sie den PC neu, um die Installation abzuschließen.",
    "Reinicie el equipo para finalizar la instalación.",
    "Redémarrez votre PC pour terminer l’installation.",
    "Riavvia il PC per completare l'installazione..",
    "PC を再起動してインストールを完了します。",
    "PC를 다시 시작하여 설치를 완료합니다.",
    "Reiniciar seu PC para terminar a instalação.",
    "Перезапустите компьютер, чтобы завершить установку.",
    "重启电脑以完成安装。",
    "重新啟動您的電腦以完成安裝。",
];

/// Drops `ESC [ ... letter` sequences and folds the accented Latin letters to ASCII, so a
/// marker still matches when winget colors the line or the pipe mangles accents.
fn plain(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' && chars.peek() == Some(&'[') {
            chars.next();
            for n in chars.by_ref() {
                if n.is_ascii_alphabetic() {
                    break;
                }
            }
            continue;
        }
        out.push(match c {
            'ç' => 'c',
            'Ç' => 'C',
            'ã' | 'á' | 'â' | 'à' => 'a',
            'Ã' | 'Á' | 'Â' | 'À' => 'A',
            'õ' | 'ó' | 'ô' => 'o',
            'Õ' | 'Ó' | 'Ô' => 'O',
            'é' | 'ê' => 'e',
            'É' | 'Ê' => 'E',
            'í' => 'i',
            'Í' => 'I',
            'ú' => 'u',
            'Ú' => 'U',
            other => other,
        });
    }
    out
}

/// Whether a winget run asks for a reboot: the HRESULT, or exit 0 with a marker line.
pub fn reboot_requested(code: i32, lines: &[String], markers: &[&str]) -> bool {
    if code as u32 == 0x8A15_0109 {
        return true;
    }
    if code != 0 {
        return false;
    }
    let markers: Vec<String> = markers.iter().map(|m| plain(m)).collect();
    lines.iter().any(|l| {
        let l = plain(l);
        markers.iter().any(|m| l.contains(m.as_str()))
    })
}

/// The version winget picked, from its `<name> [<target>] <word> <version>` line (any
/// language: the id in brackets and the last word of the line).
pub fn winget_chosen_version(target: &str, lines: &[String]) -> Option<String> {
    let id = format!("[{target}]").to_ascii_lowercase();
    lines.iter().find_map(|l| {
        let l = plain(l);
        let at = l.to_ascii_lowercase().find(&id)?;
        let rest = l[at + id.len()..].split_whitespace().collect::<Vec<_>>();
        let version = rest.last().filter(|_| rest.len() >= 2)?;
        version
            .starts_with(|c: char| c.is_ascii_digit())
            .then(|| version.to_string())
    })
}

/// The installer's own log path winget prints (a line ending in `.log` with an absolute path).
pub fn winget_installer_log(lines: &[String]) -> Option<String> {
    lines.iter().rev().find_map(|l| {
        let l = plain(l);
        let l = l.trim();
        if !l.to_ascii_lowercase().ends_with(".log") {
            return None;
        }
        let b = l.as_bytes();
        let start = (0..b.len().saturating_sub(2)).find(|&i| {
            b[i].is_ascii_alphabetic()
                && b[i + 1] == b':'
                && b[i + 2] == b'\\'
                && (i == 0 || b[i - 1] == b' ')
        })?;
        Some(l[start..].to_string())
    })
}

/// winget's exit code (an HRESULT) as a message (task 105 probe, §4.4).
pub fn winget_message(code: i32) -> String {
    let text = match code as u32 {
        0 => "Atualizado.",
        0x8A15_002B => "Nenhuma atualização aplicável para este pacote.",
        0x8A15_0014 => "O winget não encontrou este pacote instalado.",
        0x8A15_0114 => {
            "O editor não permite atualizar este pacote pelo winget; use o atualizador do próprio app."
        }
        0x8A15_0016 => "Mais de um pacote corresponde a este id.",
        0x8A15_0068 => "Pacote fixado no winget (pin); remova o pin para atualizar.",
        0x8A15_008E => {
            "A nova versão usa outro tipo de instalador; atualize pelo winget manualmente."
        }
        0x8A15_0006 | 0x8A15_0049 | 0x8A15_0052 => "O instalador falhou.",
        0x8A15_0101 | 0x8A15_0103 => "O aplicativo está em uso; feche-o e tente de novo.",
        0x8A15_0102 => "Outra instalação está em andamento; tente mais tarde.",
        0x8A15_0109 => "Atualizado; reinicie o computador para concluir.",
        0x8A15_010A | 0x8A15_010B => "É preciso reiniciar o computador.",
        0x8A15_010C | 0x8000_4004 => "Atualização cancelada.",
        0x8007_04C7 => "Permissão de administrador recusada.",
        0x8A15_0008 => "Falha no download do instalador.",
        0x8A15_0011 => "O hash do instalador não confere com o manifesto.",
        0x8A15_0041 | 0x8A15_0046 => "Contrato não aceito.",
        other => return format!("O winget terminou com erro 0x{other:08X}."),
    };
    text.to_string()
}

/// Whether `scoop update <app>` upgraded it, and the message. Scoop exits 0 for "already
/// current", "not installed", "held" and "app running" too, so 0 counts only with a new
/// installed version; the reason otherwise comes from its fixed English lines (probe §2.2).
pub fn scoop_message(
    code: i32,
    before: Option<&str>,
    after: Option<&str>,
    lines: &[String],
) -> (bool, String) {
    if code != 0 {
        return (false, format!("O scoop falhou (código {code})."));
    }
    if let Some(new) = after
        && after != before
    {
        return (true, format!("Atualizado para a versão {new}."));
    }
    let has = |needle: &str| lines.iter().any(|l| l.contains(needle));
    let text = if has("isn't installed") {
        "O scoop não encontrou este app."
    } else if has("(latest version)") || has("is already installed") {
        "Já está na versão mais recente."
    } else if has("is held to version") {
        "App segurado no scoop (hold)."
    } else if has("Running process detected") {
        "O app está aberto; feche-o e tente de novo."
    } else {
        "O scoop não atualizou o app."
    };
    (false, text.to_string())
}

/// Splits a tool's output into lines and keeps the last `TAIL_LINES` useful ones. A line ends
/// at `\n`; a trailing `\r` is dropped and only the text after the last `\r` is kept (progress
/// redrawn in place); blank lines are dropped; bytes are decoded (lossy UTF-8 unless
/// `with_decoder`) and cut at `LINE_MAX`. An unterminated line longer than `PARTIAL_MAX` is
/// cut and the rest of it dropped until the next `\n`, so memory stays bounded.
#[derive(Debug)]
pub struct Tail {
    lines: VecDeque<String>,
    partial: Vec<u8>,
    /// The current line passed `PARTIAL_MAX`: its bytes are dropped until `\n`.
    skipping: bool,
    /// Turns one line's raw bytes (without `\r`/`\n`) into text.
    decode: fn(&[u8]) -> String,
}

/// Lossy UTF-8: invalid bytes become `U+FFFD`.
pub fn utf8_lossy(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}

impl Default for Tail {
    fn default() -> Tail {
        Tail::with_decoder(utf8_lossy)
    }
}

impl Tail {
    pub fn new() -> Tail {
        Tail::default()
    }

    /// A tail whose lines are decoded by `decode` (e.g. an OEM code page) instead of UTF-8.
    pub fn with_decoder(decode: fn(&[u8]) -> String) -> Tail {
        Tail {
            lines: VecDeque::new(),
            partial: Vec::new(),
            skipping: false,
            decode,
        }
    }

    /// Takes the next piece of output; returns the lines it completed, in order.
    pub fn push(&mut self, chunk: &[u8]) -> Vec<String> {
        let mut done = Vec::new();
        let mut rest = chunk;
        while let Some(i) = rest.iter().position(|&b| b == b'\n') {
            self.take(&rest[..i]);
            done.extend(self.end_line());
            rest = &rest[i + 1..];
        }
        self.take(rest);
        done
    }

    /// End of output: the unterminated last line, if any.
    pub fn finish(&mut self) -> Option<String> {
        self.end_line()
    }

    /// Keeps `line` (already split) as the newest one.
    pub fn add(&mut self, line: String) {
        if self.lines.len() == TAIL_LINES {
            self.lines.pop_front();
        }
        self.lines.push_back(line);
    }

    pub fn last(&self) -> Option<&str> {
        self.lines.back().map(String::as_str)
    }

    /// The kept lines, oldest first.
    pub fn lines(&self) -> Vec<String> {
        self.lines.iter().cloned().collect()
    }

    /// Bytes of the unterminated line held in memory.
    pub fn buffered(&self) -> usize {
        self.partial.len()
    }

    fn take(&mut self, bytes: &[u8]) {
        if self.skipping {
            return;
        }
        let room = PARTIAL_MAX - self.partial.len();
        if bytes.len() > room {
            self.partial.extend_from_slice(&bytes[..room]);
            self.skipping = true;
        } else {
            self.partial.extend_from_slice(bytes);
        }
    }

    fn end_line(&mut self) -> Option<String> {
        self.skipping = false;
        let raw = std::mem::take(&mut self.partial);
        let raw = match raw.last() {
            Some(b'\r') => &raw[..raw.len() - 1],
            _ => &raw[..],
        };
        let seg = match raw.iter().rposition(|&b| b == b'\r') {
            Some(i) => &raw[i + 1..],
            None => raw,
        };
        let decoded = (self.decode)(seg);
        let mut line = decoded.trim_end().to_string();
        if line.trim_start().is_empty() {
            return None;
        }
        if line.len() > LINE_MAX {
            let mut cut = LINE_MAX;
            while !line.is_char_boundary(cut) {
                cut -= 1;
            }
            line.truncate(cut);
        }
        self.add(line.clone());
        Some(line)
    }
}
