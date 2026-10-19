//! Exit code classification (Task 175).

use crate::sources::winget::manifest::{ExpectedReturnCode, InstallerKind};

/// Shown when the user declines the UAC prompt.
pub const UAC_REFUSED: &str = crate::install::progress::ELEVATION_REFUSED;

/// Port of `ExpectedReturnCodeEnum` (ManifestCommon.cpp) plus `Unknown`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Category {
    PackageInUse,
    PackageInUseByApplication,
    InstallInProgress,
    FileInUse,
    MissingDependency,
    DiskFull,
    InsufficientMemory,
    InvalidParameter,
    NoNetwork,
    ContactSupport,
    RebootRequiredToFinish,
    RebootRequiredForInstall,
    RebootInitiated,
    CancelledByUser,
    AlreadyInstalled,
    Downgrade,
    BlockedByPolicy,
    SystemNotSupported,
    Custom,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Ok,
    OkReboot {
        initiated: bool,
    },
    Failed {
        category: Category,
        code: u32,
        message: String,
    },
}

use Category as C;

// Port of GetDefaultKnownReturnCodes, Burn/Wix/Msi branch
// (docs/reference/winget-cli/AppInstallerCommonCore_Manifest_ManifestCommon.cpp:1094-1116).
const MSI_DEFAULTS: &[(u32, Category)] = &[
    (1618, C::InstallInProgress),      // ERROR_INSTALL_ALREADY_RUNNING
    (112, C::DiskFull),                // ERROR_DISK_FULL
    (1601, C::ContactSupport),         // ERROR_INSTALL_SERVICE_FAILURE
    (3010, C::RebootRequiredToFinish), // ERROR_SUCCESS_REBOOT_REQUIRED
    (1641, C::RebootInitiated),        // ERROR_SUCCESS_REBOOT_INITIATED
    (1602, C::CancelledByUser),        // ERROR_INSTALL_USEREXIT
    (1638, C::AlreadyInstalled),       // ERROR_PRODUCT_VERSION
    (1654, C::SystemNotSupported),     // ERROR_INSTALL_REJECTED
    (1625, C::BlockedByPolicy),        // ERROR_INSTALL_PACKAGE_REJECTED
    (1624, C::BlockedByPolicy),        // ERROR_INSTALL_TRANSFORM_REJECTED
    (1636, C::BlockedByPolicy),        // ERROR_PATCH_PACKAGE_REJECTED
    (1649, C::BlockedByPolicy),        // ERROR_PATCH_REMOVAL_DISALLOWED
    (1640, C::BlockedByPolicy),        // ERROR_INSTALL_REMOTE_DISALLOWED
    (87, C::InvalidParameter),         // ERROR_INVALID_PARAMETER
    (1628, C::InvalidParameter),       // ERROR_INVALID_TABLE
    (1639, C::InvalidParameter),       // ERROR_INVALID_COMMAND_LINE
    (1650, C::InvalidParameter),       // ERROR_INVALID_PATCH_XML
    (1623, C::SystemNotSupported),     // ERROR_INSTALL_LANGUAGE_UNSUPPORTED
    (1633, C::SystemNotSupported),     // ERROR_INSTALL_PLATFORM_UNSUPPORTED
];

// Port of GetDefaultKnownReturnCodes, Inno branch (same file, :1118-1125).
const INNO_DEFAULTS: &[(u32, Category)] = &[
    (2, C::CancelledByUser),
    (5, C::CancelledByUser),
    (8, C::RebootRequiredForInstall),
];

// Port of GetDefaultKnownReturnCodes, Msix branch (same file, :1126-1144);
// each value is HRESULT_FROM_WIN32(code) = 0x80070000 | code.
const MSIX_DEFAULTS: &[(u32, Category)] = &[
    (0x8007_0000 | 15613, C::MissingDependency), // ERROR_INSTALL_PREREQUISITE_FAILED
    (0x8007_0000 | 15603, C::MissingDependency), // ERROR_INSTALL_RESOLVE_DEPENDENCY_FAILED
    (0x8007_0000 | 15634, C::MissingDependency), // ..._OPTIONAL_PACKAGE_REQUIRES_MAIN_PACKAGE
    (0x8007_0000 | 15604, C::DiskFull),          // ERROR_INSTALL_OUT_OF_DISK_SPACE
    (0x8007_0000 | 15608, C::CancelledByUser),   // ERROR_INSTALL_CANCEL
    (0x8007_0000 | 15611, C::AlreadyInstalled),  // ERROR_PACKAGE_ALREADY_EXISTS
    (0x8007_0000 | 15622, C::Downgrade),         // ERROR_INSTALL_PACKAGE_DOWNGRADE
    (0x8007_0000 | 15617, C::BlockedByPolicy),   // ERROR_DEPLOYMENT_BLOCKED_BY_POLICY
    (0x8007_0000 | 15615, C::BlockedByPolicy),   // ERROR_INSTALL_POLICY_FAILURE
    (0x8007_0000 | 15618, C::PackageInUse),      // ERROR_PACKAGES_IN_USE
    (0x8007_0000 | 15632, C::SystemNotSupported), // ..._WRONG_PROCESSOR_ARCHITECTURE
    (0x8007_0000 | 15635, C::SystemNotSupported), // ..._NOT_SUPPORTED_ON_FILESYSTEM
    (0x8007_0000 | 15645, C::SystemNotSupported), // ERROR_DEPLOYMENT_OPTION_NOT_SUPPORTED
    (0x8007_0000 | 15658, C::SystemNotSupported), // ..._LACKS_CAPABILITY_TO_DEPLOY_ON_HOST
];

fn defaults_for(kind: &InstallerKind) -> &'static [(u32, Category)] {
    match kind {
        InstallerKind::Msi | InstallerKind::Wix | InstallerKind::Burn => MSI_DEFAULTS,
        InstallerKind::Inno => INNO_DEFAULTS,
        InstallerKind::Msix | InstallerKind::Appx => MSIX_DEFAULTS,
        _ => &[],
    }
}

fn from_category(category: Category, code: u32) -> Outcome {
    match category {
        C::RebootRequiredToFinish => Outcome::OkReboot { initiated: false },
        C::RebootInitiated => Outcome::OkReboot { initiated: true },
        _ => Outcome::Failed {
            category,
            code,
            message: message(category).to_string(),
        },
    }
}

fn unknown(code: u32) -> Outcome {
    Outcome::Failed {
        category: C::Unknown,
        code,
        message: format!("o instalador terminou com o código {code} (0x{code:x})"),
    }
}

/// Precedence (InstallFlow.cpp): success codes, manifest expected codes, per-type defaults.
pub fn classify_install(
    kind: &InstallerKind,
    code: i32,
    success: &[i64],
    expected: &[ExpectedReturnCode],
) -> Outcome {
    let code = code as u32;
    if code == 0 || success.iter().any(|s| *s as u32 == code) {
        return Outcome::Ok;
    }
    if let Some(e) = expected
        .iter()
        .find(|e| e.installer_return_code as u32 == code)
    {
        return from_category(category_of(&e.return_response), code);
    }
    match defaults_for(kind).iter().find(|(c, _)| *c == code) {
        Some((_, cat)) => from_category(*cat, code),
        None => unknown(code),
    }
}

/// 3010 (reboot required) and 1641 (reboot initiated): the Windows Installer codes, also
/// returned by Burn bundles; the caller confirms the removal by the ARP key.
pub fn reboot_code(code: i32) -> Option<Outcome> {
    match code as u32 {
        3010 => Some(Outcome::OkReboot { initiated: false }),
        1641 => Some(Outcome::OkReboot { initiated: true }),
        _ => None,
    }
}

pub fn classify_uninstall(msiexec: bool, code: i32) -> Outcome {
    if msiexec && let Some(o) = reboot_code(code) {
        return o;
    }
    let code = code as u32;
    if code == 0 {
        return Outcome::Ok;
    }
    if msiexec {
        match code {
            1618 => return from_category(C::InstallInProgress, code),
            1602 => return from_category(C::CancelledByUser, code),
            _ => {}
        }
    }
    unknown(code)
}

pub fn classify_msix(hresult: i32) -> Outcome {
    let code = hresult as u32;
    if hresult >= 0 {
        return Outcome::Ok;
    }
    match MSIX_DEFAULTS.iter().find(|(c, _)| *c == code) {
        Some((_, cat)) => from_category(*cat, code),
        None => unknown(code),
    }
}

/// Manifest `ReturnResponse` names (ConvertToExpectedReturnCodeEnum); unknown names are `Custom`.
pub fn category_of(response: &str) -> Category {
    match response.to_ascii_lowercase().as_str() {
        "packageinuse" => C::PackageInUse,
        "packageinusebyapplication" => C::PackageInUseByApplication,
        "installinprogress" => C::InstallInProgress,
        "fileinuse" => C::FileInUse,
        "missingdependency" => C::MissingDependency,
        "diskfull" => C::DiskFull,
        "insufficientmemory" => C::InsufficientMemory,
        "invalidparameter" => C::InvalidParameter,
        "nonetwork" => C::NoNetwork,
        "contactsupport" => C::ContactSupport,
        "rebootrequiredtofinish" => C::RebootRequiredToFinish,
        "rebootrequiredforinstall" => C::RebootRequiredForInstall,
        "rebootinitiated" => C::RebootInitiated,
        "cancelledbyuser" => C::CancelledByUser,
        "alreadyinstalled" => C::AlreadyInstalled,
        "downgrade" => C::Downgrade,
        "blockedbypolicy" => C::BlockedByPolicy,
        "systemnotsupported" => C::SystemNotSupported,
        _ => C::Custom,
    }
}

pub fn message(c: Category) -> &'static str {
    match c {
        C::PackageInUse => "o pacote está em uso; feche-o e tente de novo",
        C::PackageInUseByApplication => {
            "o pacote está em uso por um aplicativo; feche-o e tente de novo"
        }
        C::InstallInProgress => "outro instalador está rodando; tente de novo em instantes",
        C::FileInUse => "um arquivo necessário está em uso",
        C::MissingDependency => "falta uma dependência do pacote",
        C::DiskFull => "não há espaço em disco suficiente",
        C::InsufficientMemory => "memória insuficiente",
        C::InvalidParameter => "o instalador recusou os parâmetros recebidos",
        C::NoNetwork => "o instalador precisa de rede e não a encontrou",
        C::ContactSupport => "o instalador pede para contatar o suporte do pacote",
        C::RebootRequiredToFinish => "reinicie o computador para concluir",
        C::RebootRequiredForInstall => "reinicie o computador e rode de novo para instalar",
        C::RebootInitiated => "o instalador iniciou a reinicialização do computador",
        C::CancelledByUser => "a instalação foi cancelada pelo usuário",
        C::AlreadyInstalled => "o pacote já está instalado",
        C::Downgrade => "já há uma versão mais nova instalada",
        C::BlockedByPolicy => "uma política do sistema bloqueou a instalação",
        C::SystemNotSupported => "o sistema não é compatível com este pacote",
        C::Custom => "o instalador terminou com um código esperado pelo manifesto",
        C::Unknown => "o instalador terminou com um código desconhecido",
    }
}
