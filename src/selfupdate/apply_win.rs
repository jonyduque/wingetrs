//! Windows side of `self update` (Task 213): which scope the executable's folder is in, the
//! real UAC launch of the elevated copy, and that copy's entry point.
use super::{Outcome, SelfUpdateError};
use std::path::Path;
use windows::Win32::Security::Cryptography::{
    CERT_OPEN_STORE_FLAGS, CERT_QUERY_ENCODING_TYPE, CERT_STORE_OPEN_EXISTING_FLAG,
    CERT_STORE_PROV_SYSTEM_W, CERT_STORE_READONLY_FLAG, CERT_SYSTEM_STORE_LOCAL_MACHINE,
    CertCloseStore, CertEnumCertificatesInStore, CertOpenStore,
};
use windows::core::HSTRING;

/// `CERT_SYSTEM_STORE_LOCAL_MACHINE`: the machine's stores, which only an administrator writes.
pub const LOCAL_MACHINE: u32 = CERT_SYSTEM_STORE_LOCAL_MACHINE;
/// The machine stores the elevated client trusts: the roots and the third-party roots Windows
/// adds on demand. Never a `CurrentUser` store (the user writes those without elevation).
pub const MACHINE_ROOT_STORES: [&str; 2] = ["Root", "AuthRoot"];

/// Reads the DER certificates of one system store: `(location flag, store name)`.
pub type StoreReader<'a> = &'a dyn Fn(u32, &str) -> std::io::Result<Vec<Vec<u8>>>;

/// The trusted roots of the elevated client: every certificate of
/// [`MACHINE_ROOT_STORES`] in [`LOCAL_MACHINE`] read through `read`, without duplicates. A
/// store that cannot be opened is skipped; none read is an error.
pub fn machine_roots_with(read: StoreReader<'_>) -> std::io::Result<Vec<Vec<u8>>> {
    let mut roots: Vec<Vec<u8>> = Vec::new();
    let mut last_err = None;
    for name in MACHINE_ROOT_STORES {
        match read(LOCAL_MACHINE, name) {
            Ok(certs) => {
                for c in certs {
                    if !roots.contains(&c) {
                        roots.push(c);
                    }
                }
            }
            Err(e) => last_err = Some(e),
        }
    }
    match (roots.is_empty(), last_err) {
        (true, Some(e)) => Err(e),
        (true, None) => Err(std::io::Error::other(
            "o repositório de certificados da máquina está vazio",
        )),
        (false, _) => Ok(roots),
    }
}

/// One system store's certificates (read-only, never created).
fn read_store(location: u32, name: &str) -> std::io::Result<Vec<Vec<u8>>> {
    let wide = HSTRING::from(name);
    // SAFETY: `wide` outlives the call; the store is closed below.
    let store = unsafe {
        CertOpenStore(
            CERT_STORE_PROV_SYSTEM_W,
            CERT_QUERY_ENCODING_TYPE(0),
            None,
            CERT_OPEN_STORE_FLAGS(location)
                | CERT_STORE_READONLY_FLAG
                | CERT_STORE_OPEN_EXISTING_FLAG,
            Some(wide.as_ptr().cast()),
        )
    }
    .map_err(|e| std::io::Error::other(e.to_string()))?;
    let mut out = Vec::new();
    let mut prev = None;
    loop {
        // SAFETY: `prev` is the context this enumeration returned last (freed by this call).
        let ctx = unsafe { CertEnumCertificatesInStore(store, prev) };
        if ctx.is_null() {
            break;
        }
        // SAFETY: a non-null context of the open store, valid until the next call.
        let c = unsafe { &*ctx };
        if !c.pbCertEncoded.is_null() && c.cbCertEncoded > 0 {
            // SAFETY: `cbCertEncoded` bytes at `pbCertEncoded`, owned by the context.
            let der =
                unsafe { std::slice::from_raw_parts(c.pbCertEncoded, c.cbCertEncoded as usize) };
            out.push(der.to_vec());
        }
        prev = Some(ctx.cast_const());
    }
    // SAFETY: the enumeration ended (no context left to free); closed once.
    let _ = unsafe { CertCloseStore(Some(store), 0) };
    Ok(out)
}

/// The machine's trusted roots ([`machine_roots_with`] over the real stores).
pub fn machine_roots() -> std::io::Result<Vec<Vec<u8>>> {
    machine_roots_with(&read_store)
}

/// Whether `exe` and every folder above it are administrator-only by owner and DACL (and none
/// a reparse point): then only an elevated copy may replace it.
pub fn admin_scope(exe: &Path) -> bool {
    crate::bridge::arp_ops_win::hold_admin_chain(exe).is_ok()
}

/// [`super::elevated_apply_started`] through the shell's `runas` (one UAC prompt).
pub fn elevated_apply(
    exe: &Path,
    notice: &mut dyn FnMut(),
    started: &mut dyn FnMut(),
) -> Result<Outcome, SelfUpdateError> {
    let launcher = crate::winget_install::launch_win::ShellLauncher;
    super::elevated_apply_started(&launcher, exe, notice, started)
}

/// `self apply-elevated`: runs elevated, from its own executable (never a path received), only
/// when that executable's chain is administrator-only and it is not a development build. It
/// writes nothing outside the executable's folder (no data dir, no log, no output) and answers
/// by its exit code (`super::exit_code`).
pub fn run_elevated() -> i32 {
    let r = elevated_main();
    super::exit_code(&r)
}

fn elevated_main() -> Result<Outcome, SelfUpdateError> {
    let exe = std::env::current_exe()?;
    super::refuse_dev_build(&exe)?;
    if !admin_scope(&exe) {
        return Err(SelfUpdateError::Other(
            "pasta do executável não é só de administrador".into(),
        ));
    }
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    rt.block_on(async {
        // Never `Http::new`: no proxy of the user and only the machine's roots (Task 213 review).
        let roots = machine_roots()?;
        let http = crate::http::Http::for_elevated(&roots).map_err(|e| match e {
            crate::http::HttpError::UntrustedCertificate(m) => SelfUpdateError::UntrustedRoot(m),
            other => SelfUpdateError::Other(other.to_string()),
        })?;
        super::elevated_run(
            &http,
            super::WEB_BASE,
            super::DL_BASE,
            &exe,
            env!("CARGO_PKG_VERSION"),
        )
        .await
    })
}
