//! Uninstall command from an ARP entry (Task 172).
use crate::installed::Hive;
use crate::installed::arp::split_command;
use crate::installed::guid::pack_guid;
use serde::{Deserialize, Serialize};

/// What the ARP key says about an installed product (all values read raw).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArpDetails {
    pub hive: Hive,
    pub key: String,
    pub display_name: Option<String>,
    pub publisher: Option<String>,
    pub display_version: Option<String>,
    pub windows_installer: bool,
    pub system_component: bool,
    pub quiet_uninstall_string: Option<String>,
    pub uninstall_string: Option<String>,
    pub inno_app_path: Option<String>,
    pub bundle_cache_path: Option<String>,
    pub bundle_upgrade_code: Option<String>,
    pub install_location: Option<String>,
}

/// Which rule produced the command (travels over the bridge pipe).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Rule {
    Msi,
    Quiet,
    Inno,
    Burn,
    Nsis,
    Interactive,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UninstallCommand {
    pub rule: Rule,
    /// "msiexec" or the program as written in the registry (not expanded).
    pub program: String,
    pub args: String,
    pub interactive: bool,
}

/// A key name that is safe to use as one registry sub-key component.
pub fn valid_arp_key(key: &str) -> bool {
    !key.is_empty()
        && !key.contains(['\\', '/'])
        && key.chars().count() <= 255
        && key != "."
        && key != ".."
}

fn non_blank(s: &Option<String>) -> Option<&str> {
    s.as_deref().map(str::trim).filter(|v| !v.is_empty())
}

/// Appends the switches missing from `rest` (token compare, case-insensitive).
fn with_switches(rest: &str, switches: &[&str]) -> String {
    let mut out = rest.trim().to_string();
    for sw in switches {
        if !rest.split_whitespace().any(|t| t.eq_ignore_ascii_case(sw)) {
            if !out.is_empty() {
                out.push(' ');
            }
            out.push_str(sw);
        }
    }
    out
}

fn split_or_whole(cmd: &str) -> (String, String) {
    split_command(cmd).unwrap_or_else(|| (cmd.trim().to_string(), String::new()))
}

/// Spec P10/D4 order: msi, quiet, inno, burn, nsis (manifest hint), interactive.
pub fn uninstall_command(d: &ArpDetails, nsis_from_manifest: bool) -> Option<UninstallCommand> {
    if d.windows_installer && pack_guid(&d.key).is_some() {
        return Some(UninstallCommand {
            rule: Rule::Msi,
            program: "msiexec".into(),
            args: format!("/x{} /quiet /norestart", d.key),
            interactive: false,
        });
    }
    if let Some(q) = non_blank(&d.quiet_uninstall_string) {
        let (program, args) = split_or_whole(q);
        return Some(UninstallCommand {
            rule: Rule::Quiet,
            program,
            args,
            interactive: false,
        });
    }
    let uninstall = non_blank(&d.uninstall_string);
    let built = |rule: Rule, switches: &[&str]| {
        uninstall.map(|u| {
            let (program, rest) = split_or_whole(u);
            UninstallCommand {
                rule,
                program,
                args: with_switches(&rest, switches),
                interactive: false,
            }
        })
    };
    if d.key.to_ascii_lowercase().ends_with("_is1") && non_blank(&d.inno_app_path).is_some() {
        return built(
            Rule::Inno,
            &["/VERYSILENT", "/SUPPRESSMSGBOXES", "/NORESTART"],
        );
    }
    if non_blank(&d.bundle_cache_path).is_some() || non_blank(&d.bundle_upgrade_code).is_some() {
        return built(Rule::Burn, &["/quiet", "/norestart"]);
    }
    if nsis_from_manifest {
        return built(Rule::Nsis, &["/S"]);
    }
    uninstall.map(|u| {
        let (program, args) = split_or_whole(u);
        UninstallCommand {
            rule: Rule::Interactive,
            program,
            args,
            interactive: true,
        }
    })
}
