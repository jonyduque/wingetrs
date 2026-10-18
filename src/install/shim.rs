//! Shim files: kiennq launcher copies for executables, text wrappers for scripts.

use std::fs;
use std::path::{Path, PathBuf};

use crate::cache::DataDir;
use crate::http::Http;
use crate::install::download::verify;
use crate::install::extract::extract;
use crate::install::layout::Layout;
use crate::install::manifest::{HashAlgo, HashSpec};
use crate::install::plan::{ShimKind, ShimSpec};

#[derive(Debug, thiserror::Error)]
pub enum ShimError {
    #[error("lançador de shims indisponível: {0}")]
    Launcher(String),
    #[error("E/S: {0}")]
    Io(String),
}

fn io<E: std::fmt::Display>(e: E) -> ShimError {
    ShimError::Io(e.to_string())
}

const LAUNCHER_BASE: &str =
    "https://github.com/kiennq/scoop-better-shimexe/releases/download/v3.2.2/shimexe-";

/// (zip suffix, SHA-256 of the zip) for the machine architecture.
fn launcher_asset() -> (&'static str, &'static str) {
    if cfg!(target_arch = "aarch64") {
        (
            "aarch64",
            "a5896c99527ec8f9effe445c19a2d9cf2a482e6eb36d72660612b41b50d3a250",
        )
    } else if cfg!(target_arch = "x86") {
        (
            "x86",
            "05a1e35861ad8d7033e544e5ea058e22dd2208edd53dd9ed9da9867a24fe8375",
        )
    } else {
        (
            "x86_64",
            "27ee2cffa9d6f5f72f39a4f60d23b6ac3390db843a3b2eefb52d64daa80a8440",
        )
    }
}

/// Offset of the PE `Subsystem` field, if `bytes` look like a PE image.
fn subsystem_offset(bytes: &[u8]) -> Option<usize> {
    let lfanew = u32::from_le_bytes(bytes.get(0x3C..0x40)?.try_into().ok()?) as usize;
    let sig_end = lfanew.checked_add(4)?;
    if bytes.get(lfanew..sig_end)? != b"PE\0\0" {
        return None;
    }
    let off = lfanew.checked_add(24 + 68)?;
    bytes.get(off..off.checked_add(2)?)?;
    Some(off)
}

/// Reads the PE optional-header Subsystem (same offset for PE32 and PE32+).
pub fn pe_subsystem(bytes: &[u8]) -> Option<u16> {
    let off = subsystem_offset(bytes)?;
    Some(u16::from_le_bytes([bytes[off], bytes[off + 1]]))
}

/// Overwrites the Subsystem field; false (and untouched) if `bytes` is not a PE image.
pub fn set_pe_subsystem(bytes: &mut [u8], subsystem: u16) -> bool {
    match subsystem_offset(bytes) {
        Some(off) => {
            bytes[off..off + 2].copy_from_slice(&subsystem.to_le_bytes());
            true
        }
        None => false,
    }
}

/// Downloads the launcher for this architecture if `layout.launcher()` is missing.
pub async fn ensure_launcher(
    http: &Http,
    data: &DataDir,
    layout: &Layout,
) -> Result<(), ShimError> {
    let (arch, sha) = launcher_asset();
    let url = format!("{LAUNCHER_BASE}{arch}.zip");
    ensure_launcher_from(http, data, layout, &url, sha).await
}

/// Same as [`ensure_launcher`] with an explicit zip URL and SHA-256 (injectable for tests).
pub async fn ensure_launcher_from(
    http: &Http,
    _data: &DataDir,
    layout: &Layout,
    url: &str,
    sha256: &str,
) -> Result<(), ShimError> {
    let dest = layout.launcher();
    if dest.exists() {
        return Ok(());
    }
    let bytes = http
        .get(url)
        .await
        .map_err(|e| ShimError::Launcher(format!("{url}: {e}")))?;
    let spec = HashSpec {
        algo: HashAlgo::Sha256,
        hex: sha256.to_ascii_lowercase(),
    };
    if !verify(&spec, &bytes) {
        return Err(ShimError::Launcher(format!("SHA-256 de {url} não confere")));
    }
    let parent = dest
        .parent()
        .ok_or_else(|| ShimError::Io("launcher sem pasta".into()))?;
    fs::create_dir_all(parent).map_err(io)?;
    let zip_path = dest.with_extension("zip.part");
    let stage = dest.with_extension("stage");
    let _ = fs::remove_dir_all(&stage);
    let result = (|| {
        fs::write(&zip_path, &bytes).map_err(io)?;
        extract(&zip_path, "shim.zip", None, None, &stage)
            .map_err(|e| ShimError::Launcher(e.to_string()))?;
        let exe = stage.join("shim.exe");
        if !exe.is_file() {
            return Err(ShimError::Launcher("shim.exe ausente no zip".into()));
        }
        fs::rename(&exe, &dest).map_err(io)
    })();
    let _ = fs::remove_file(&zip_path);
    let _ = fs::remove_dir_all(&stage);
    result
}

/// First 4 KiB of `path` (enough for the PE headers of any sane image); `None` on any error.
fn read_header(path: &Path) -> Option<Vec<u8>> {
    use std::io::Read;
    let mut buf = Vec::new();
    fs::File::open(path)
        .ok()?
        .take(4096)
        .read_to_end(&mut buf)
        .ok()?;
    Some(buf)
}

/// A shim name must be exactly one safe path component (it comes from a manifest alias).
pub(crate) fn is_safe_shim_name(name: &str) -> bool {
    !name.contains(['/', '\\', '*', '?', '<', '>', '|', '"'])
        && !name.chars().any(char::is_control)
        && crate::install::safepath::safe_components(name).is_some_and(|c| c.len() == 1)
}

/// Shim arguments (manifest `bin` and the `shim` effect of a hook) go into the `.shim` and
/// `.cmd` text as is: only an allowlist passes (Task 191), so neither cmd nor PowerShell reads
/// syntax in them. ASCII letters and digits, space, `- _ . / \ : = , +`, and `"` in balanced
/// pairs (paths with spaces). Over the catalog of 2026-10-18 (4118 Scoop manifests, 332 `bin`
/// args, variables replaced) it refuses one: `extras/controlprinters` (`shell:::{GUID}`).
pub fn check_args(args: &str) -> Result<(), String> {
    let allowed = |c: char| c.is_ascii_alphanumeric() || " -_./\\:=,+".contains(c);
    let mut quoted = false;
    for c in args.chars() {
        if c == '"' {
            quoted = !quoted;
        } else if !allowed(c) {
            return Err(format!("{c:?} não é aceito nos argumentos de um comando"));
        }
    }
    if quoted {
        return Err("aspas sem fechar nos argumentos de um comando".into());
    }
    Ok(())
}

/// `args` split as cmd and the C runtime would (whitespace outside double quotes separates,
/// the quotes themselves go), for the `.ps1` shim's literal array.
pub fn split_args(args: &str) -> Vec<String> {
    split_args_quoted(args)
        .into_iter()
        .map(|(t, _)| t)
        .collect()
}

/// [`split_args`], each argument with whether it had double quotes.
pub fn split_args_quoted(args: &str) -> Vec<(String, bool)> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let (mut quoted, mut any) = (false, false);
    for c in args.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                any = true;
            }
            c if c.is_whitespace() && !quoted => {
                if any || !cur.is_empty() {
                    out.push((std::mem::take(&mut cur), any));
                }
                any = false;
            }
            c => cur.push(c),
        }
    }
    if any || !cur.is_empty() {
        out.push((cur, any));
    }
    out
}

/// `$version`/`$app` values a shim argument may take: `[A-Za-z0-9._+-]`, not empty (they come
/// from the manifest; a version `1.0&calc` would be cmd syntax once substituted).
pub fn is_plain_value(v: &str) -> bool {
    !v.is_empty()
        && v.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '+' | '-'))
}

/// `tokens` (one argument each, already substituted, with whether the manifest quoted it) as
/// cmd text: a quoted token, or one with anything but ASCII letters, digits and
/// `- _ . / \ : = , +`, goes inside `"…"`, where cmd reads `&`, `(`, `)` literally; `"`, `%`
/// and `!` cannot be made inert there and are refused (a user profile path holding one refuses
/// the shim).
pub fn cmd_args(tokens: &[(String, bool)]) -> Result<String, String> {
    let plain = |c: char| c.is_ascii_alphanumeric() || "-_./\\:=,+".contains(c);
    let mut out = Vec::new();
    for (t, quoted) in tokens {
        if let Some(c) = t
            .chars()
            .find(|&c| c.is_control() || matches!(c, '"' | '%' | '!'))
        {
            return Err(format!(
                "{c:?} em {t:?} não pode ir nos argumentos de um comando (nem entre aspas)"
            ));
        }
        if !quoted && !t.is_empty() && t.chars().all(plain) {
            out.push(t.clone());
        } else {
            out.push(format!("\"{t}\""));
        }
    }
    Ok(out.join(" "))
}

/// A PowerShell single-quoted literal: nothing inside it expands.
fn ps_literal(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// The `.ps1` shim: the target and the arguments as literals, never interpolated.
fn ps1_text(target: &str, args: Option<&str>) -> String {
    let list: Vec<String> = args
        .map(split_args)
        .unwrap_or_default()
        .iter()
        .map(|a| ps_literal(a))
        .collect();
    format!(
        "$path = {}\r\n$shimArgs = @({})\r\n& $path @shimArgs @args\r\n",
        ps_literal(target),
        list.join(", ")
    )
}

fn check_name(name: &str) -> Result<(), ShimError> {
    if is_safe_shim_name(name) {
        Ok(())
    } else {
        Err(ShimError::Io(format!("nome de shim inseguro: {name}")))
    }
}

fn write_text(path: &Path, text: &str) -> Result<(), ShimError> {
    fs::write(path, text.as_bytes()).map_err(io)
}

/// Creates the shim files for `spec`; returns every file created.
pub fn create_shim(layout: &Layout, spec: &ShimSpec) -> Result<Vec<PathBuf>, ShimError> {
    check_name(&spec.name)?;
    let dir = layout.shims();
    fs::create_dir_all(&dir).map_err(io)?;
    let target = spec.target.display().to_string();
    let args = spec.args.as_deref().filter(|a| !a.is_empty());
    let args_sp = args.map(|a| format!(" {a}")).unwrap_or_default();
    let mut created = Vec::new();
    match spec.kind {
        ShimKind::Exe => {
            let exe = dir.join(format!("{}.exe", spec.name));
            let shim = dir.join(format!("{}.shim", spec.name));
            let launcher = layout.launcher();
            match fs::remove_file(&exe) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(io(e)),
            }
            let gui = read_header(&spec.target)
                .and_then(|b| pe_subsystem(&b))
                .is_some_and(|s| s == 2);
            if gui {
                let mut bytes = fs::read(&launcher)
                    .map_err(|e| ShimError::Launcher(format!("{}: {e}", launcher.display())))?;
                if !set_pe_subsystem(&mut bytes, 2) {
                    return Err(ShimError::Launcher("lançador inválido".into()));
                }
                fs::write(&exe, bytes).map_err(io)?;
            } else if fs::hard_link(&launcher, &exe).is_err() {
                fs::copy(&launcher, &exe)
                    .map_err(|e| ShimError::Launcher(format!("{}: {e}", launcher.display())))?;
            }
            let mut text = format!("path = \"{target}\"\r\n");
            if let Some(a) = args {
                text.push_str(&format!("args = {a}\r\n"));
            }
            write_text(&shim, &text)?;
            created.push(exe);
            created.push(shim);
        }
        ShimKind::Cmd | ShimKind::Bat => {
            let cmd = dir.join(format!("{}.cmd", spec.name));
            write_text(
                &cmd,
                &format!("@rem {target}\r\n@\"{target}\"{args_sp} %*\r\n"),
            )?;
            created.push(cmd);
        }
        ShimKind::Ps1 => {
            let ps1 = dir.join(format!("{}.ps1", spec.name));
            let cmd = dir.join(format!("{}.cmd", spec.name));
            write_text(&ps1, &ps1_text(&target, args))?;
            let cmd_text = format!(
                "@rem {target}\r\n@echo off\r\nwhere /q pwsh.exe\r\nif %errorlevel% equ 0 (\r\n    pwsh.exe -noprofile -ex unrestricted -file \"{target}\"{args_sp} %*\r\n) else (\r\n    powershell.exe -noprofile -ex unrestricted -file \"{target}\"{args_sp} %*\r\n)\r\n"
            );
            write_text(&cmd, &cmd_text)?;
            created.push(ps1);
            created.push(cmd);
        }
        ShimKind::Jar => {
            let cmd = dir.join(format!("{}.cmd", spec.name));
            let folder = spec
                .target
                .parent()
                .map(|p| p.display().to_string())
                .unwrap_or_default();
            write_text(
                &cmd,
                &format!(
                    "@rem {target}\r\n@pushd {folder}\r\n@java -jar \"{target}\"{args_sp} %*\r\n@popd\r\n"
                ),
            )?;
            created.push(cmd);
        }
    }
    Ok(created)
}

/// Deletes every `shims\<name>.{exe,shim,cmd,ps1}` that exists.
pub fn remove_shim(layout: &Layout, name: &str) -> Result<(), ShimError> {
    check_name(name)?;
    let dir = layout.shims();
    for ext in ["exe", "shim", "cmd", "ps1"] {
        match fs::remove_file(dir.join(format!("{name}.{ext}"))) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(io(e)),
        }
    }
    Ok(())
}
