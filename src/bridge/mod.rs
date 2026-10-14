//! Upgrade bridge: apps installed by winget or scoop are upgraded through their own CLI.
//! Everything here is pure (rules, command lines, exit codes, output splitting); only
//! `spawn_win` starts a process.
use crate::installed::correlate::{Correlation, ListRow, UpdateState};
use std::collections::{BTreeSet, VecDeque};
use std::path::{Path, PathBuf};

#[cfg(windows)]
pub mod spawn_win;

pub const PINNED_NOTE: &str = "versão fixada";
pub const NOT_OURS_NOTE: &str = "instalado fora do wingetrs";

/// Lines kept for the error log.
pub const TAIL_LINES: usize = 20;
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
    Ok(BridgeJob {
        tool,
        target,
        name: r.name.clone(),
    })
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
            let ps1 = scoop_ps1(scoop_root)
                .display()
                .to_string()
                .replace('\'', "''");
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

/// winget's exit code (an HRESULT) as a message (task 105 probe, §4.4).
pub fn winget_message(code: i32) -> String {
    let text = match code as u32 {
        0 => "Atualizado.",
        0x8A15_002B => "Nenhuma atualização aplicável para este pacote.",
        0x8A15_0014 => "O winget não encontrou este pacote instalado.",
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
/// redrawn in place); blank lines are dropped; bytes are decoded lossily and cut at
/// `LINE_MAX`. An unterminated line longer than `PARTIAL_MAX` is cut and the rest of it
/// dropped until the next `\n`, so memory stays bounded.
#[derive(Debug, Default)]
pub struct Tail {
    lines: VecDeque<String>,
    partial: Vec<u8>,
    /// The current line passed `PARTIAL_MAX`: its bytes are dropped until `\n`.
    skipping: bool,
}

impl Tail {
    pub fn new() -> Tail {
        Tail::default()
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
        let mut line = String::from_utf8_lossy(seg).trim_end().to_string();
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
