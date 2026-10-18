//! Protocol and loop of the elevated bridge helper (`wingetrs bridge-worker`): one JSON object
//! per line each way. The helper only ever runs `bridge::command` of a winget upgrade or install
//! with a valid id, `bridge::winget_source_update`, or a hash-verified installer whose fields
//! pass `InstallerRequest::check`; anything else is refused or closes the channel.
use super::{BridgeJob, BridgeOp, Tool};
use crate::installed::Hive;
use crate::installed::correlate::InstallScope;
use crate::installed::guid::pack_guid;
use crate::winget_install::elevated_rules::{
    KnownFolders, elevated_duplicate, expand_known, in_admin_folder, still_orphan,
};
use crate::winget_install::uninstall_cmd::{ArpDetails, Rule, uninstall_command, valid_arp_key};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::io::{BufRead, Read, Write};
use std::path::{Path, PathBuf};

/// Longest line either side reads or writes, without its `\n`.
pub const LINE_CAP: usize = 64 * 1024;

/// What the TUI asks the helper.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum Request {
    /// `winget upgrade --id <target> ...` (the fixed command of `bridge::command`).
    Upgrade { target: String },
    /// `winget install --id <target> ...`, plus `--scope <scope>` when given.
    Install {
        target: String,
        scope: Option<ScopeArg>,
    },
    /// `winget source update --name winget ...`.
    // Struct variants, not unit ones: serde lets extra fields through on a unit variant of an
    // internally tagged enum even with `deny_unknown_fields`.
    SourceUpdate {},
    /// Runs a hash-verified copy of the installer at `source` (Task 180).
    RunInstaller {
        source: String,
        sha256: String,
        file_name: String,
        kind: ElevatedKind,
        params: String,
    },
    /// Runs the uninstaller of an HKLM ARP entry, recomputed from the key as it is now
    /// (Task 182); `rule` is what the user confirmed.
    RunUninstaller {
        hive: MachineHive,
        key: String,
        rule: Rule,
        nsis_hint: bool,
    },
    /// Deletes an HKLM ARP key once `reason` holds again (Task 182).
    DeleteArpKey {
        hive: MachineHive,
        key: String,
        reason: DeleteReason,
    },
    /// Ends the helper.
    Quit {},
}

/// The `--scope` of a winget install: a closed set, never free text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ScopeArg {
    User,
    Machine,
}

/// An HKLM ARP view. HKCU has no value on purpose: the user writes it, so nothing from it
/// ever runs or is deleted elevated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MachineHive {
    Hklm64,
    Hklm32,
}

impl From<MachineHive> for Hive {
    fn from(h: MachineHive) -> Hive {
        match h {
            MachineHive::Hklm64 => Hive::MachineX64,
            MachineHive::Hklm32 => Hive::MachineX86,
        }
    }
}

/// Why an ARP key may be deleted; the helper checks it again before deleting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DeleteReason {
    /// Its uninstaller no longer exists.
    Orphan,
    /// Another record has the same name.
    Duplicate,
}

impl From<ScopeArg> for InstallScope {
    fn from(s: ScopeArg) -> InstallScope {
        match s {
            ScopeArg::User => InstallScope::User,
            ScopeArg::Machine => InstallScope::Machine,
        }
    }
}

impl From<InstallScope> for ScopeArg {
    fn from(s: InstallScope) -> ScopeArg {
        match s {
            InstallScope::User => ScopeArg::User,
            InstallScope::Machine => ScopeArg::Machine,
        }
    }
}

/// Installer types the helper may run (msix never goes through it).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ElevatedKind {
    Msi,
    Wix,
    Burn,
    Inno,
    Nullsoft,
    Exe,
}

impl ElevatedKind {
    /// Whether it runs through `msiexec /i`.
    pub fn is_msi(self) -> bool {
        matches!(self, ElevatedKind::Msi | ElevatedKind::Wix)
    }
}

/// The fields of a `RunInstaller` request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallerRequest {
    /// Absolute path of the downloaded installer (in the user's folders; only read).
    pub source: String,
    /// Expected SHA-256, hex.
    pub sha256: String,
    /// Name of the copy in the execution folder.
    pub file_name: String,
    pub kind: ElevatedKind,
    /// Installer parameters; `<LOGPATH>` becomes the log in the execution folder.
    pub params: String,
}

/// Longest `source` accepted, in bytes (the Win32 long path limit).
pub const SOURCE_MAX: usize = 32_767;
/// Most installer log lines sent back, from its end.
pub const INSTALLER_LINES: usize = 40;
/// Longest installer log line sent back, in bytes (escaped, it stays under `LINE_CAP`).
pub const INSTALLER_LINE_MAX: usize = LINE_CAP / 8;

impl InstallerRequest {
    /// Checks every field before anything touches the disk; the refusal reason otherwise.
    /// `program_files` are the folders (from the known folders) an install folder may be in.
    pub fn check(&self, program_files: &[String]) -> Result<(), &'static str> {
        if !valid_source(&self.source) {
            return Err("caminho do instalador inválido");
        }
        if self.sha256.len() != 64 || !self.sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err("hash do instalador inválido");
        }
        if !source_is_blob(&self.source, &self.sha256) {
            return Err("o instalador não está no cache de downloads");
        }
        if !valid_file_name(&self.file_name, self.kind) {
            return Err("nome do instalador inválido");
        }
        if crate::winget_install::switches::validate(&self.params).is_err()
            || self.params.contains("\\\"")
        {
            return Err("parâmetros do instalador inválidos");
        }
        let allowed = if self.kind.is_msi() {
            msi_params_allowed_in(&self.params, program_files)
        } else {
            exe_params_allowed(self.kind, &self.params, program_files)
        };
        if !allowed {
            return Err("parâmetros do instalador fora da lista permitida");
        }
        Ok(())
    }
}

/// `...\blobs\<sha256 in lowercase>`: the download cache's name for the file.
fn source_is_blob(source: &str, sha256: &str) -> bool {
    let mut parts = source.rsplit('\\');
    parts.next() == Some(sha256.to_ascii_lowercase().as_str())
        && parts
            .next()
            .is_some_and(|p| p.eq_ignore_ascii_case("blobs"))
}

/// `value` (quotes around it removed) is an absolute path strictly inside one of
/// `program_files` (case-insensitive), with no `..`, device name or stream.
fn under_program_files(value: &str, program_files: &[String]) -> bool {
    let value = value
        .strip_prefix('"')
        .and_then(|v| v.strip_suffix('"'))
        .unwrap_or(value);
    // A `/` cannot lead out: `X:/` fails `valid_source`, and right after the root it fails the
    // `\` check below.
    if value.contains('"') || !valid_source(value) {
        return false;
    }
    let value = value.trim_end_matches('\\').to_ascii_lowercase();
    program_files.iter().any(|root| {
        let root = root.trim_end_matches('\\').to_ascii_lowercase();
        !root.is_empty()
            && value
                .strip_prefix(&root)
                .is_some_and(|rest| rest.len() > 1 && rest.starts_with('\\'))
    })
}

/// Switches that load or save a configuration file (Inno `/LOADINF`, `/SAVEINF`).
const CONFIG_SWITCHES: [&str; 2] = ["loadinf", "saveinf"];

/// Inno Setup switches without a value the helper passes (any case).
const INNO_FLAGS: [&str; 9] = [
    "/verysilent",
    "/silent",
    "/suppressmsgboxes",
    "/norestart",
    "/sp-",
    "/closeapplications",
    "/nocancel",
    "/allusers",
    "/currentuser",
];

/// Burn switches without a value the helper passes (`/` or `-`, any case).
const BURN_FLAGS: [&str; 3] = ["quiet", "passive", "norestart"];

/// `v` is the log placeholder, bare or quoted (it becomes a file in the execution folder).
fn is_log(v: &str) -> bool {
    let token = crate::winget_install::switches::LOG_TOKEN;
    v == token || v.strip_prefix('"').and_then(|v| v.strip_suffix('"')) == Some(token)
}

/// A value of a plain exe switch: no `\`, no `/`, no `:`, no `..`, no quote inside a quoted
/// run (so it cannot name any path).
fn plain_value(v: &str) -> bool {
    let inner = v
        .strip_prefix('"')
        .and_then(|v| v.strip_suffix('"'))
        .unwrap_or(v);
    !inner.contains(['\\', '/', ':', '"']) && !inner.contains("..")
}

/// Whether `params` for an exe-type installer holds only the switches the helper allows for
/// `kind` (an allowlist; any other token, positional ones included, is refused):
/// - inno: `INNO_FLAGS`, `/LOG=<LOGPATH>`, `/DIR=<folder in program_files>`;
/// - nullsoft: `/S`, and `/D=<folder in program_files>` last (the rest of the line);
/// - burn: `BURN_FLAGS` and `/log <LOGPATH>` (`/` or `-`);
/// - exe: `/key`, `-key`, `/key=value` or `/key:value` with `plain_value`, or a key followed by
///   `<LOGPATH>`; never `/LOADINF`/`/SAVEINF`.
pub fn exe_params_allowed(kind: ElevatedKind, params: &str, program_files: &[String]) -> bool {
    let Some(tokens) = msi_tokens(params) else {
        return false;
    };
    let mut tokens = tokens.into_iter().peekable();
    while let Some(t) = tokens.next() {
        let lower = t.to_ascii_lowercase();
        let ok = match kind {
            ElevatedKind::Inno => {
                INNO_FLAGS.contains(&lower.as_str())
                    || lower.strip_prefix("/log=").is_some() && is_log(&t[5..])
                    || lower.strip_prefix("/dir=").is_some()
                        && under_program_files(&t[5..], program_files)
            }
            ElevatedKind::Nullsoft => {
                if let Some(rest) = t.strip_prefix("/D=") {
                    // `/D=` is the last parameter and its folder is never quoted, spaces and
                    // all: the value is the rest of the line.
                    let at = rest.as_ptr() as usize - params.as_ptr() as usize;
                    return under_program_files(params[at..].trim_end(), program_files);
                }
                t == "/S"
            }
            ElevatedKind::Burn => {
                let key = lower.strip_prefix(['/', '-']).unwrap_or("");
                BURN_FLAGS.contains(&key) || key == "log" && tokens.next().is_some_and(is_log)
            }
            ElevatedKind::Exe => exe_switch(t, &mut tokens),
            ElevatedKind::Msi | ElevatedKind::Wix => false,
        };
        if !ok {
            return false;
        }
    }
    true
}

/// One switch `t` of a plain exe installer; a following
/// `<LOGPATH>` token is taken as its value.
fn exe_switch<'a>(
    t: &str,
    tokens: &mut std::iter::Peekable<impl Iterator<Item = &'a str>>,
) -> bool {
    let Some(body) = t.strip_prefix(['/', '-']) else {
        return false;
    };
    let body = body.trim_start_matches('-');
    let (key, value) = match body.find(['=', ':']) {
        Some(i) => (body[..i].to_ascii_lowercase(), Some(&body[i + 1..])),
        None => (body.to_ascii_lowercase(), None),
    };
    let key_ok = !key.is_empty()
        && key
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'+' | b'!'))
        && !CONFIG_SWITCHES.iter().any(|c| key.starts_with(c));
    if !key_ok {
        return false;
    }
    match value {
        Some(v) => is_log(v) || plain_value(v),
        None => {
            if tokens.peek().is_some_and(|n| is_log(n)) {
                tokens.next();
            }
            true
        }
    }
}

/// `X:\...`: a drive letter, no `..` or device-name component, no `\\?\` or UNC prefix, no
/// other `:`.
fn valid_source(s: &str) -> bool {
    let b = s.as_bytes();
    if s.len() > SOURCE_MAX || b.len() < 3 || s.contains('\0') {
        return false;
    }
    if !(b[0].is_ascii_alphabetic() && b[1] == b':' && b[2] == b'\\') {
        return false;
    }
    // A second `:` would name an alternate data stream.
    if s[2..].contains(':') {
        return false;
    }
    // `..` would leave the folder and a device name (`CON`, `NUL`...) is not a file: both are
    // refused by `is_windows_unfriendly` (`..` ends with a dot, like `.`).
    !s[3..]
        .split(['\\', '/'])
        .any(|c| !c.is_empty() && crate::install::safepath::is_windows_unfriendly(c))
}

/// `^[A-Za-z0-9][A-Za-z0-9._+-]{0,127}\.(exe|msi)$`, `.msi` for msi/wix and `.exe` otherwise.
fn valid_file_name(name: &str, kind: ElevatedKind) -> bool {
    let ext = if kind.is_msi() { ".msi" } else { ".exe" };
    let Some(stem) = name.strip_suffix(ext) else {
        return false;
    };
    let mut chars = stem.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    first.is_ascii_alphanumeric()
        && stem.len() <= 128
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '+' | '-'))
}

/// `params` split on spaces outside double quotes; `None` when a quote is left open.
fn msi_tokens(params: &str) -> Option<Vec<&str>> {
    let mut tokens = Vec::new();
    let mut start = None;
    let mut quoted = false;
    for (i, c) in params.char_indices() {
        match c {
            '"' => {
                quoted = !quoted;
                start.get_or_insert(i);
            }
            ' ' | '\t' if !quoted => {
                if let Some(s) = start.take() {
                    tokens.push(&params[s..i]);
                }
            }
            _ => {
                start.get_or_insert(i);
            }
        }
    }
    if quoted {
        return None;
    }
    if let Some(s) = start {
        tokens.push(&params[s..]);
    }
    Some(tokens)
}

/// `NAME=value` with a name of a closed list (any case) and a value it allows: `ALLUSERS`
/// (empty, 1, 2), `MSIINSTALLPERUSER` (empty, 1), `REBOOT` (`ReallySuppress`, `Suppress`,
/// `Force` or their initials), `ADDLOCAL` (feature names and commas), `INSTALLDIR`/`TARGETDIR`
/// (a folder inside `program_files`). The value is bare without quotes, or one quoted run
/// without quotes inside. `TRANSFORMS`, `PATCH`, `ACTION`, `REINSTALL`... are refused.
fn msi_property(token: &str, program_files: &[String]) -> bool {
    let Some((name, value)) = token.split_once('=') else {
        return false;
    };
    let value = match value.strip_prefix('"') {
        Some(rest) => match rest.strip_suffix('"') {
            Some(inner) if !inner.contains('"') => inner,
            _ => return false,
        },
        None if value.contains('"') => return false,
        None => value,
    };
    let one_of = |allowed: &[&str]| allowed.iter().any(|a| a.eq_ignore_ascii_case(value));
    match name.to_ascii_uppercase().as_str() {
        "ALLUSERS" => one_of(&["", "1", "2"]),
        "MSIINSTALLPERUSER" => one_of(&["", "1"]),
        "REBOOT" => one_of(&["ReallySuppress", "Suppress", "Force", "R", "S", "F"]),
        "ADDLOCAL" => {
            !value.is_empty()
                && value
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b','))
        }
        "INSTALLDIR" | "TARGETDIR" => under_program_files(value, program_files),
        _ => false,
    }
}

/// `msi_params_allowed_in` with no install folder allowed (`INSTALLDIR`/`TARGETDIR` refused).
pub fn msi_params_allowed(params: &str) -> bool {
    msi_params_allowed_in(params, &[])
}

/// Whether `params` for `msiexec /i` only holds the switches the helper allows: `/quiet`,
/// `/passive`, `/qn`, `/qb`, `/norestart`, `/promptrestart`, `/log "<LOGPATH>"` and the
/// properties of `msi_property`. Any other switch (`/i`, `/x`, `/a`, `/p`, `/f*`, `/j*`,
/// `/l*`...) is refused.
pub fn msi_params_allowed_in(params: &str, program_files: &[String]) -> bool {
    let Some(tokens) = msi_tokens(params) else {
        return false;
    };
    let log = format!("\"{}\"", crate::winget_install::switches::LOG_TOKEN);
    let mut tokens = tokens.into_iter();
    while let Some(t) = tokens.next() {
        let lower = t.to_ascii_lowercase();
        let ok = match lower.as_str() {
            "/quiet" | "/passive" | "/qn" | "/qb" | "/norestart" | "/promptrestart" => true,
            "/log" => tokens.next() == Some(log.as_str()),
            // A switch never passes: a property name starts with a letter or `_`.
            _ => msi_property(t, program_files),
        };
        if !ok {
            return false;
        }
    }
    true
}

/// What the helper does for each accepted request; the real one runs winget and installers
/// (`elevate_win`), tests use fakes.
pub trait WorkerOps {
    /// Runs `program args` (a winget command), relaying its output lines.
    fn run(
        &mut self,
        program: &str,
        args: &[String],
        on_line: &mut dyn FnMut(&str),
    ) -> std::io::Result<(i32, Vec<String>)>;
    /// Folders an install folder (`INSTALLDIR`, `/DIR=`, `/D=`) may be in: `Program Files`
    /// from the known folders. None by default (those parameters are then refused).
    fn program_files(&self) -> Vec<String> {
        Vec::new()
    }
    /// Runs the verified copy of an installer; `on_line` gets the end of its log.
    fn run_installer(
        &mut self,
        req: &InstallerRequest,
        on_line: &mut dyn FnMut(&str),
    ) -> std::io::Result<i32>;
    /// The known folders (never the environment). Unavailable by default.
    fn known_folders(&self) -> std::io::Result<KnownFolders> {
        Err(unsupported())
    }
    /// Reads one ARP key with the view of `hive`; `Ok(None)` when it does not exist.
    fn read_arp(&mut self, _hive: Hive, _key: &str) -> std::io::Result<Option<ArpDetails>> {
        Err(unsupported())
    }
    /// Every ARP entry of HKLM 64 and HKLM 32 (never HKCU: the user writes it).
    fn arp_entries(&mut self) -> std::io::Result<Vec<ArpDetails>> {
        Err(unsupported())
    }
    /// Whether something is at `path`. By default everything exists (nothing is orphan).
    fn exists(&self, _path: &Path) -> bool {
        true
    }
    /// `System32\msiexec.exe` from the known folder.
    fn msiexec(&self) -> std::io::Result<PathBuf> {
        Err(unsupported())
    }
    /// Runs an uninstaller the helper already checked; its exit code.
    fn run_uninstaller(&mut self, _program: &Path, _args: &str) -> std::io::Result<i32> {
        Err(unsupported())
    }
    /// Deletes the ARP key `key` and its tree with the view of `hive`, refusing a link.
    fn delete_arp_key(&mut self, _hive: MachineHive, _key: &str) -> std::io::Result<()> {
        Err(unsupported())
    }
}

fn unsupported() -> std::io::Error {
    std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "operação não suportada por este auxiliar",
    )
}

/// A winget exec (a `BridgeExec` closure) as operations: `run` calls it, and installers are
/// refused without running anything.
impl<F> WorkerOps for F
where
    F: FnMut(&str, &[String], &mut dyn FnMut(&str)) -> std::io::Result<(i32, Vec<String>)>,
{
    fn run(
        &mut self,
        program: &str,
        args: &[String],
        on_line: &mut dyn FnMut(&str),
    ) -> std::io::Result<(i32, Vec<String>)> {
        self(program, args, on_line)
    }

    fn run_installer(
        &mut self,
        _req: &InstallerRequest,
        _on_line: &mut dyn FnMut(&str),
    ) -> std::io::Result<i32> {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "este auxiliar não roda instaladores",
        ))
    }
}

/// What the helper answers: any number of `Line`, then one `End` or `Error`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Reply {
    Line {
        text: String,
    },
    End {
        code: i32,
    },
    /// Nothing ran (refused request), or the tool could not be started.
    Error {
        message: String,
        not_found: bool,
    },
}

fn invalid(message: impl Into<String>) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, message.into())
}

/// `msg` as one line (with its `\n`); refused when it would pass `LINE_CAP`.
pub fn encode<T: Serialize>(msg: &T) -> std::io::Result<String> {
    let mut text = serde_json::to_string(msg).map_err(|e| invalid(e.to_string()))?;
    if text.len() > LINE_CAP {
        return Err(invalid(format!("mensagem com {} bytes", text.len())));
    }
    text.push('\n');
    Ok(text)
}

/// One line (without its `\n`) as a message; unknown kinds and extra fields are refused.
pub fn decode<T: DeserializeOwned>(line: &str) -> std::io::Result<T> {
    serde_json::from_str(line).map_err(|e| invalid(e.to_string()))
}

/// The next line, without `\n`; `None` at the end of the stream. A line longer than
/// `LINE_CAP` is refused before it is buffered whole.
pub fn read_line(reader: &mut dyn BufRead) -> std::io::Result<Option<String>> {
    let mut buf = Vec::new();
    reader
        .take(LINE_CAP as u64 + 1)
        .read_until(b'\n', &mut buf)?;
    if buf.is_empty() {
        return Ok(None);
    }
    if buf.last() == Some(&b'\n') {
        buf.pop();
    }
    if buf.len() > LINE_CAP {
        return Err(invalid(format!("linha com mais de {LINE_CAP} bytes")));
    }
    String::from_utf8(buf)
        .map(Some)
        .map_err(|_| invalid("linha que não é UTF-8"))
}

fn send(writer: &mut dyn Write, reply: &Reply) -> std::io::Result<()> {
    writer.write_all(encode(reply)?.as_bytes())?;
    writer.flush()
}

/// The helper's loop: reads requests from `reader`, runs each with `ops` and writes the
/// output lines and the end to `writer`. Returns at the end of `reader` or at `quit`; an
/// invalid line (too long, not a known request) ends it with an error, without running it.
pub fn serve(
    reader: &mut dyn BufRead,
    writer: &mut dyn Write,
    ops: &mut dyn WorkerOps,
) -> std::io::Result<()> {
    while let Some(line) = read_line(reader)? {
        let request: Request = decode(&line).inspect_err(|e| {
            tracing::warn!(error = %e, "bridge worker: invalid request; closing");
        })?;
        let request = match request {
            Request::Quit {} => return Ok(()),
            Request::SourceUpdate {} => {
                let (program, args) = super::winget_source_update();
                run(writer, ops, &program, &args)?;
                continue;
            }
            Request::RunInstaller {
                source,
                sha256,
                file_name,
                kind,
                params,
            } => {
                let req = InstallerRequest {
                    source,
                    sha256,
                    file_name,
                    kind,
                    params,
                };
                run_installer(writer, ops, &req)?;
                continue;
            }
            Request::RunUninstaller {
                hive,
                key,
                rule,
                nsis_hint,
            } => {
                serve_uninstaller(writer, ops, hive, &key, rule, nsis_hint)?;
                continue;
            }
            Request::DeleteArpKey { hive, key, reason } => {
                serve_delete(writer, ops, hive, &key, reason)?;
                continue;
            }
            Request::Upgrade { target } => (target, BridgeOp::Upgrade),
            Request::Install { target, scope } => (
                target,
                BridgeOp::Install {
                    scope: scope.map(InstallScope::from),
                    agreements: Vec::new(),
                },
            ),
        };
        let (program, args) = match request {
            (target, _) if !super::valid_target(Tool::Winget, &target) => {
                tracing::warn!(len = target.len(), "bridge worker: invalid target refused");
                send(
                    writer,
                    &Reply::Error {
                        message: "identificador inválido".to_string(),
                        not_found: false,
                    },
                )?;
                continue;
            }
            (target, op) => {
                let job = BridgeJob {
                    tool: Tool::Winget,
                    op,
                    name: target.clone(),
                    target,
                    installed: None,
                    available: None,
                    scope: None,
                };
                super::command(&job, Path::new(""), "")
            }
        };
        run(writer, ops, &program, &args)?;
    }
    Ok(())
}

/// `text` cut at a char boundary to at most `INSTALLER_LINE_MAX` bytes.
fn clip(text: &str) -> &str {
    if text.len() <= INSTALLER_LINE_MAX {
        return text;
    }
    let mut end = INSTALLER_LINE_MAX;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

/// Checks `req` and, when valid, runs it through `ops`; sends at most the last
/// `INSTALLER_LINES` log lines it reported, then the end. A refused request runs nothing.
fn run_installer(
    writer: &mut dyn Write,
    ops: &mut dyn WorkerOps,
    req: &InstallerRequest,
) -> std::io::Result<()> {
    if let Err(reason) = req.check(&ops.program_files()) {
        tracing::warn!(reason, "bridge worker: installer request refused");
        return send(
            writer,
            &Reply::Error {
                message: reason.to_string(),
                not_found: false,
            },
        );
    }
    tracing::info!(kind = ?req.kind, file = %req.file_name, "bridge worker: running installer");
    let mut tail = std::collections::VecDeque::with_capacity(INSTALLER_LINES);
    let result = ops.run_installer(req, &mut |text: &str| {
        if tail.len() == INSTALLER_LINES {
            tail.pop_front();
        }
        tail.push_back(clip(text).to_string());
    });
    for text in tail {
        send(writer, &Reply::Line { text })?;
    }
    let reply = match result {
        Ok(code) => Reply::End { code },
        Err(e) => Reply::Error {
            message: e.to_string(),
            not_found: e.kind() == std::io::ErrorKind::NotFound,
        },
    };
    send(writer, &reply)
}

/// Runs one accepted command, relaying its lines and its end to `writer`.
fn run(
    writer: &mut dyn Write,
    ops: &mut dyn WorkerOps,
    program: &str,
    args: &[String],
) -> std::io::Result<()> {
    tracing::info!(
        cmd = %super::command_text(program, args),
        "bridge worker: running"
    );
    // A broken channel does not stop the tool; the first write error ends the loop after it.
    let mut broken: Option<std::io::Error> = None;
    let result = ops.run(program, args, &mut |text: &str| {
        if broken.is_none()
            && let Err(e) = send(
                writer,
                &Reply::Line {
                    text: text.to_string(),
                },
            )
        {
            broken = Some(e);
        }
    });
    if let Some(e) = broken {
        return Err(e);
    }
    let reply = match result {
        Ok((code, _)) => Reply::End { code },
        Err(e) => Reply::Error {
            message: e.to_string(),
            not_found: e.kind() == std::io::ErrorKind::NotFound,
        },
    };
    send(writer, &reply)
}
/// Answer of an ARP key whose name is not one safe sub-key component.
pub const BAD_KEY: &str = "nome de chave ARP inválido";
/// Answer when the key is gone.
pub const KEY_GONE: &str = "a chave não existe mais";
/// Answer when the recomputed rule is not the confirmed one.
pub const KEY_CHANGED: &str = "a chave mudou desde a confirmação";
/// Answer when the uninstaller is not an absolute `.exe` in an administrator folder.
pub const OUTSIDE_ADMIN: &str = "desinstalador fora de pasta de administrador";
/// Answer when the uninstaller is a shell or script host.
pub const INTERPRETER: &str = "desinstalador por interpretador recusado";
/// Answer when an `msiexec` line is not `/x{GUID}` with quiet switches only.
pub const MSIEXEC_REFUSED: &str = "parâmetros do msiexec recusados";

/// Programs that run whatever their arguments name; never run elevated from an ARP line.
const INTERPRETERS: [&str; 8] = [
    "cmd.exe",
    "powershell.exe",
    "pwsh.exe",
    "rundll32.exe",
    "regsvr32.exe",
    "wscript.exe",
    "cscript.exe",
    "mshta.exe",
];

/// Switches an `msiexec /x` line may carry besides the product code.
const MSIEXEC_SWITCHES: [&str; 9] = [
    "/quiet",
    "/passive",
    "/q",
    "/qn",
    "/qb",
    "/qb-",
    "/qb!",
    "/norestart",
    "reboot=reallysuppress",
];

fn refusal(message: &str) -> Reply {
    Reply::Error {
        message: message.to_string(),
        not_found: false,
    }
}

fn failure(e: &std::io::Error) -> Reply {
    Reply::Error {
        message: e.to_string(),
        not_found: e.kind() == std::io::ErrorKind::NotFound,
    }
}

/// The last component of a program as written (`\` or `/` separated), lowercase.
fn program_file(program: &str) -> String {
    program
        .rsplit(['\\', '/'])
        .next()
        .unwrap_or(program)
        .to_ascii_lowercase()
}

fn is_msiexec(program: &str) -> bool {
    matches!(program_file(program).as_str(), "msiexec" | "msiexec.exe")
}

/// `args` of an `msiexec` uninstall rebuilt from its tokens: `/x{GUID}` (or `/x {GUID}`) with
/// a valid product code, then only quiet/no-restart switches; `None` for anything else
/// (`/i`, `/f`, `/p`, properties, files).
pub fn msiexec_uninstall_args(args: &str) -> Option<String> {
    let mut tokens = args.split_whitespace();
    let first = tokens.next()?;
    let lower = first.to_ascii_lowercase();
    let rest = lower
        .strip_prefix("/x")
        .or_else(|| lower.strip_prefix("-x"))?;
    let guid = if rest.is_empty() {
        tokens.next()?
    } else {
        first.get(2..)?
    };
    pack_guid(guid)?;
    let mut out = format!("/x{guid}");
    for t in tokens {
        if !MSIEXEC_SWITCHES.contains(&t.to_ascii_lowercase().as_str()) {
            return None;
        }
        out.push(' ');
        out.push_str(t);
    }
    Some(out)
}

/// The program and arguments that uninstall `key`, checked as the helper requires: the rule
/// recomputed from the key as it is now equals `rule`; `msiexec` is the system one with
/// `/x{GUID}`; anything else is an existing absolute `.exe` in an administrator folder,
/// expanded with the known folders only, and not a shell or script host.
fn uninstall_plan(
    ops: &mut dyn WorkerOps,
    hive: MachineHive,
    key: &str,
    rule: Rule,
    nsis_hint: bool,
) -> Result<(PathBuf, String), Reply> {
    if !valid_arp_key(key) {
        return Err(refusal(BAD_KEY));
    }
    let d = ops
        .read_arp(hive.into(), key)
        .map_err(|e| failure(&e))?
        .ok_or_else(|| refusal(KEY_GONE))?;
    let cmd = uninstall_command(&d, nsis_hint)
        .filter(|c| c.rule == rule)
        .ok_or_else(|| refusal(KEY_CHANGED))?;
    if is_msiexec(&cmd.program) {
        let args = msiexec_uninstall_args(&cmd.args).ok_or_else(|| refusal(MSIEXEC_REFUSED))?;
        let msiexec = ops.msiexec().map_err(|e| failure(&e))?;
        return Ok((msiexec, args));
    }
    let k = ops.known_folders().map_err(|e| failure(&e))?;
    let program = Some(cmd.program.trim())
        .filter(|p| p.to_ascii_lowercase().ends_with(".exe"))
        .and_then(|p| expand_known(p, &k))
        .map(PathBuf::from)
        .filter(|p| in_admin_folder(p, &k))
        .ok_or_else(|| refusal(OUTSIDE_ADMIN))?;
    if INTERPRETERS.contains(&program_file(&cmd.program).as_str()) {
        return Err(refusal(INTERPRETER));
    }
    if !ops.exists(&program) {
        return Err(Reply::Error {
            message: format!("desinstalador não encontrado: {}", program.display()),
            not_found: true,
        });
    }
    Ok((program, cmd.args))
}

/// `RunUninstaller`: runs the checked uninstaller (see `uninstall_plan`) and answers its exit
/// code; a refused request runs nothing.
fn serve_uninstaller(
    writer: &mut dyn Write,
    ops: &mut dyn WorkerOps,
    hive: MachineHive,
    key: &str,
    rule: Rule,
    nsis_hint: bool,
) -> std::io::Result<()> {
    let (program, args) = match uninstall_plan(ops, hive, key, rule, nsis_hint) {
        Ok(plan) => plan,
        Err(reply) => {
            tracing::warn!(?hive, ?rule, ?reply, "bridge worker: uninstall refused");
            return send(writer, &reply);
        }
    };
    tracing::info!(?hive, ?rule, program = %program.display(), "bridge worker: running uninstaller");
    let reply = match ops.run_uninstaller(&program, &args) {
        Ok(code) => Reply::End { code },
        Err(e) => failure(&e),
    };
    send(writer, &reply)
}

/// Whether `reason` holds for `key` now, read by the helper itself.
fn reason_holds(
    ops: &mut dyn WorkerOps,
    hive: MachineHive,
    key: &str,
    reason: DeleteReason,
) -> Result<bool, Reply> {
    if !valid_arp_key(key) {
        return Err(refusal(BAD_KEY));
    }
    let d = ops
        .read_arp(hive.into(), key)
        .map_err(|e| failure(&e))?
        .ok_or_else(|| refusal(KEY_GONE))?;
    Ok(match reason {
        DeleteReason::Orphan => {
            let k = ops.known_folders().map_err(|e| failure(&e))?;
            still_orphan(&d, &k, |p| ops.exists(p))
        }
        DeleteReason::Duplicate => {
            let all = ops.arp_entries().map_err(|e| failure(&e))?;
            elevated_duplicate(&d, &all)
        }
    })
}

/// `DeleteArpKey`: deletes the key with the same view once `reason` holds again.
fn serve_delete(
    writer: &mut dyn Write,
    ops: &mut dyn WorkerOps,
    hive: MachineHive,
    key: &str,
    reason: DeleteReason,
) -> std::io::Result<()> {
    let reply = match reason_holds(ops, hive, key, reason) {
        Err(reply) => reply,
        Ok(false) => refusal(match reason {
            DeleteReason::Orphan => "a chave deixou de ser sobra",
            DeleteReason::Duplicate => "a chave deixou de ser duplicada",
        }),
        Ok(true) => {
            tracing::info!(?hive, ?reason, "bridge worker: deleting ARP key");
            match ops.delete_arp_key(hive, key) {
                Ok(()) => Reply::End { code: 0 },
                Err(e) => failure(&e),
            }
        }
    };
    if let Reply::Error { message, .. } = &reply {
        tracing::warn!(
            ?hive,
            ?reason,
            message,
            "bridge worker: ARP key not deleted"
        );
    }
    send(writer, &reply)
}
