//! PowerShell execution policy detection (Task 160).
//!
//! `-ExecutionPolicy Bypass` on the child beats every local policy except one set by group
//! policy (`MachinePolicy`/`UserPolicy`): those are read from the registry beforehand and refuse
//! the run with a clear message, instead of the child's `PSSecurityException`.

/// The group policy values of `...\Policies\Microsoft\Windows\PowerShell` (or
/// `...\Microsoft\PowerShellCore`) under `HKLM` (machine) and `HKCU` (user).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GpoValues {
    pub machine_enable_scripts: Option<u32>,
    pub machine_policy: Option<String>,
    pub user_enable_scripts: Option<u32>,
    pub user_policy: Option<String>,
}

/// Why group policy forbids scripts, machine before user; `None` = it does not.
pub fn gpo_blocks(v: &GpoValues) -> Option<String> {
    let scopes = [
        (v.machine_enable_scripts, v.machine_policy.as_deref()),
        (v.user_enable_scripts, v.user_policy.as_deref()),
    ];
    scopes
        .into_iter()
        .find_map(|(enable, policy)| scope_blocks(enable, policy))
        .map(|value| {
            format!(
                "a política de grupo (GPO) do PowerShell ({value}) impede rodar scripts; peça ao administrador"
            )
        })
}

/// The value that blocks within one scope: `EnableScripts = 0`, or a policy that a `-File`
/// script without a signature cannot pass.
fn scope_blocks(enable: Option<u32>, policy: Option<&str>) -> Option<String> {
    if enable == Some(0) {
        return Some("EnableScripts = 0".to_string());
    }
    let policy = policy?;
    match policy.trim().to_ascii_lowercase().as_str() {
        "allsigned" | "restricted" => Some(policy.trim().to_string()),
        _ => None,
    }
}

/// The child's output says the execution policy stopped the script.
pub fn policy_error(lines: &[String]) -> Option<String> {
    const MARKERS: [&str; 3] = [
        "UnauthorizedAccess",
        "PSSecurityException",
        "about_Execution_Policies",
    ];
    lines
        .iter()
        .any(|l| MARKERS.iter().any(|m| l.contains(m)))
        .then(|| {
            "a política de execução do PowerShell impediu o script; veja Get-ExecutionPolicy -List"
                .to_string()
        })
}

/// Manifest scripts never run with an elevated token.
pub fn refuse_if_elevated(elevated: bool) -> Result<(), String> {
    if elevated {
        Err("o wingetrs não roda scripts de manifesto com o processo elevado".to_string())
    } else {
        Ok(())
    }
}
