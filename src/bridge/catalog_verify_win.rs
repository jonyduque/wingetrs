//! Signature of the winget catalog on the elevated side (Task 202): `WinVerifyTrust`
//! (`WINTRUST_ACTION_GENERIC_VERIFY_V2`, which for an msix checks every file of the package,
//! the block map and the signature, with its timestamp) on the copy in the administrator-only
//! run folder; then the signer certificate, taken from the package's `AppxSignature.p7x` (read
//! through the zip, decoded by `CryptQueryObject`), must chain to a Microsoft root
//! (`CERT_CHAIN_POLICY_MICROSOFT_ROOT`, application roots included). Its name and age are
//! checked by `signed_install::check_signer`.
//!
//! Nothing here reaches the network: no revocation check and cache-only URL retrieval, since a
//! CryptoAPI fetch would follow the user's proxy settings (the elevated side's network goes only
//! through `Http::for_elevated`).
use super::signed_install::{CatalogVerifier, Signer, Verified};
use std::io::{Read, Seek};
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::AsRawHandle;
use std::path::Path;
use std::time::{Duration, SystemTime};
use windows::Win32::Foundation::{FILETIME, HANDLE, HWND, INVALID_HANDLE_VALUE};
use windows::Win32::Security::Cryptography::{
    CERT_CHAIN_CACHE_ONLY_URL_RETRIEVAL, CERT_CHAIN_CONTEXT, CERT_CHAIN_PARA,
    CERT_CHAIN_POLICY_MICROSOFT_ROOT, CERT_CHAIN_POLICY_PARA, CERT_CHAIN_POLICY_STATUS,
    CERT_CONTEXT, CERT_FIND_SUBJECT_CERT, CERT_INFO, CERT_NAME_ATTR_TYPE,
    CERT_QUERY_CONTENT_FLAG_PKCS7_SIGNED, CERT_QUERY_FORMAT_FLAG_BINARY, CERT_QUERY_OBJECT_BLOB,
    CMSG_SIGNER_CERT_INFO_PARAM, CRYPT_INTEGER_BLOB, CertCloseStore, CertFindCertificateInStore,
    CertFreeCertificateChain, CertFreeCertificateContext, CertGetCertificateChain,
    CertGetNameStringW, CertVerifyCertificateChainPolicy, CryptMsgClose, CryptMsgGetParam,
    CryptQueryObject, HCERTSTORE, MICROSOFT_ROOT_CERT_CHAIN_POLICY_CHECK_APPLICATION_ROOT_FLAG,
    PKCS_7_ASN_ENCODING, X509_ASN_ENCODING, szOID_COMMON_NAME, szOID_ORGANIZATION_NAME,
};
use windows::Win32::Security::WinTrust::{
    WINTRUST_ACTION_GENERIC_VERIFY_V2, WINTRUST_DATA, WINTRUST_DATA_0, WINTRUST_FILE_INFO,
    WTD_CACHE_ONLY_URL_RETRIEVAL, WTD_CHOICE_FILE, WTD_REVOCATION_CHECK_NONE, WTD_REVOKE_NONE,
    WTD_STATEACTION_IGNORE, WTD_UI_NONE, WinVerifyTrust,
};
use windows::Win32::Storage::FileSystem::FILE_SHARE_READ;
use windows::core::{HSTRING, PCWSTR};

/// The package's signature inside the msix.
const SIGNATURE_ENTRY: &str = "AppxSignature.p7x";
/// Largest signature read (the catalog's is about 10 KB).
const SIGNATURE_MAX: u64 = 1 << 20;
/// Largest `CERT_INFO` of the signer accepted from `CryptMsgGetParam`.
const SIGNER_INFO_MAX: u32 = 64 * 1024;
/// 100 ns intervals between 1601-01-01 and 1970-01-01.
const FILETIME_UNIX: u64 = 116_444_736_000_000_000;

/// The production verifier.
pub struct WinTrustVerifier;

/// Largest catalog read back (as `signed_install::MSIX_MAX`).
const CATALOG_MAX: u64 = super::signed_install::MSIX_MAX as u64;

impl CatalogVerifier for WinTrustVerifier {
    /// One handle for everything: opened denying write and delete, `WinVerifyTrust` on that
    /// handle, then the signature and the bytes read through it, so what is parsed is what was
    /// verified even if the path could be swapped.
    fn verify(&self, msix: &Path) -> Result<Verified, String> {
        let mut file = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ.0)
            .open(msix)
            .map_err(|e| e.to_string())?;
        verify_trust_handle(msix, &file)?;
        file.rewind().map_err(|e| e.to_string())?;
        let mut bytes = Vec::new();
        (&mut file)
            .take(CATALOG_MAX + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        if bytes.len() as u64 > CATALOG_MAX {
            return Err("catálogo grande demais".into());
        }
        let blob = signature_blob(&bytes)?;
        let signer = signer_of(&blob)?;
        Ok(Verified { signer, bytes })
    }
}

/// `WinVerifyTrust` with the generic Authenticode action on the open `handle` of the file at
/// `path` (the path only names it; the content read is the handle's), no UI, no revocation, no
/// URL retrieval.
fn verify_trust_handle(path: &Path, handle: &std::fs::File) -> Result<(), String> {
    let wide = HSTRING::from(path.as_os_str());
    let mut file = WINTRUST_FILE_INFO {
        cbStruct: std::mem::size_of::<WINTRUST_FILE_INFO>() as u32,
        pcwszFilePath: PCWSTR(wide.as_ptr()),
        hFile: HANDLE(handle.as_raw_handle()),
        ..Default::default()
    };
    let mut data = WINTRUST_DATA {
        cbStruct: std::mem::size_of::<WINTRUST_DATA>() as u32,
        dwUIChoice: WTD_UI_NONE,
        fdwRevocationChecks: WTD_REVOKE_NONE,
        dwUnionChoice: WTD_CHOICE_FILE,
        Anonymous: WINTRUST_DATA_0 { pFile: &mut file },
        // No state kept: nothing to close afterwards.
        dwStateAction: WTD_STATEACTION_IGNORE,
        dwProvFlags: WTD_REVOCATION_CHECK_NONE | WTD_CACHE_ONLY_URL_RETRIEVAL,
        ..Default::default()
    };
    let mut action = WINTRUST_ACTION_GENERIC_VERIFY_V2;
    // SAFETY: `data` and the `file` and `wide` it points to outlive the call; no UI window.
    let rc = unsafe {
        WinVerifyTrust(
            HWND(INVALID_HANDLE_VALUE.0),
            &mut action,
            (&mut data as *mut WINTRUST_DATA).cast(),
        )
    };
    if rc != 0 {
        return Err(format!(
            "WinVerifyTrust recusou o pacote (0x{:08X})",
            rc as u32
        ));
    }
    Ok(())
}

/// The PKCS #7 signature of the msix `bytes`: its `AppxSignature.p7x` without the `PKCX`
/// marker.
fn signature_blob(bytes: &[u8]) -> Result<Vec<u8>, String> {
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|e| e.to_string())?;
    let entry = zip
        .by_name(SIGNATURE_ENTRY)
        .map_err(|e| format!("{SIGNATURE_ENTRY}: {e}"))?;
    let mut p7x = Vec::new();
    entry
        .take(SIGNATURE_MAX + 1)
        .read_to_end(&mut p7x)
        .map_err(|e| format!("{SIGNATURE_ENTRY}: {e}"))?;
    if p7x.len() as u64 > SIGNATURE_MAX {
        return Err(format!("{SIGNATURE_ENTRY} grande demais"));
    }
    p7x.strip_prefix(b"PKCX")
        .map(<[u8]>::to_vec)
        .ok_or_else(|| format!("{SIGNATURE_ENTRY} sem o marcador PKCX"))
}

/// Closes a message and a store from `CryptQueryObject` on drop.
struct Query {
    store: HCERTSTORE,
    msg: *mut core::ffi::c_void,
}

impl Drop for Query {
    fn drop(&mut self) {
        // SAFETY: each was returned by CryptQueryObject (or is null) and is closed once.
        unsafe {
            if !self.msg.is_null() {
                let _ = CryptMsgClose(Some(self.msg));
            }
            if !self.store.0.is_null() {
                let _ = CertCloseStore(Some(self.store), 0);
            }
        }
    }
}

/// Frees a certificate context on drop.
struct Cert(*mut CERT_CONTEXT);

impl Drop for Cert {
    fn drop(&mut self) {
        // SAFETY: a context from CertFindCertificateInStore, freed once.
        unsafe {
            let _ = CertFreeCertificateContext(Some(self.0));
        }
    }
}

/// Frees a chain context on drop.
struct Chain(*mut CERT_CHAIN_CONTEXT);

impl Drop for Chain {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: a chain from CertGetCertificateChain, freed once.
            unsafe { CertFreeCertificateChain(self.0) };
        }
    }
}

/// `FILETIME` as `SystemTime` (before 1970 is 1970).
fn system_time(ft: FILETIME) -> SystemTime {
    let ticks = (u64::from(ft.dwHighDateTime) << 32) | u64::from(ft.dwLowDateTime);
    let since_unix = ticks.saturating_sub(FILETIME_UNIX);
    SystemTime::UNIX_EPOCH + Duration::from_nanos(since_unix.saturating_mul(100))
}

/// The attribute `oid` of the certificate's subject, empty when absent.
fn name_attr(cert: *const CERT_CONTEXT, oid: windows::core::PCSTR) -> String {
    let mut buf = [0u16; 256];
    // SAFETY: `cert` is a valid context; `oid` a static NUL-terminated OID string; the call
    // writes at most `buf.len()` characters, NUL included, and returns how many it wrote.
    let n = unsafe {
        CertGetNameStringW(
            cert,
            CERT_NAME_ATTR_TYPE,
            0,
            Some(oid.0.cast()),
            Some(&mut buf),
        )
    } as usize;
    String::from_utf16_lossy(&buf[..n.saturating_sub(1).min(buf.len())])
}

/// The signer certificate of the PKCS #7 `blob`, chained (with the signature's own
/// certificates) to a Microsoft root: its common name, organization and start of validity.
fn signer_of(blob: &[u8]) -> Result<Signer, String> {
    let os = |e: windows::core::Error| e.message();
    let crypt = CRYPT_INTEGER_BLOB {
        cbData: u32::try_from(blob.len()).map_err(|e| e.to_string())?,
        pbData: blob.as_ptr().cast_mut(),
    };
    let mut q = Query {
        store: HCERTSTORE::default(),
        msg: std::ptr::null_mut(),
    };
    // SAFETY: `crypt` points into `blob`, alive for the call; the store and message handles
    // land in `q`, which closes them.
    unsafe {
        CryptQueryObject(
            CERT_QUERY_OBJECT_BLOB,
            (&crypt as *const CRYPT_INTEGER_BLOB).cast(),
            CERT_QUERY_CONTENT_FLAG_PKCS7_SIGNED,
            CERT_QUERY_FORMAT_FLAG_BINARY,
            0,
            None,
            None,
            None,
            Some(&mut q.store),
            Some(&mut q.msg),
            None,
        )
    }
    .map_err(os)?;
    let mut size = 0u32;
    // SAFETY: valid message; first call only asks the size.
    unsafe { CryptMsgGetParam(q.msg, CMSG_SIGNER_CERT_INFO_PARAM, 0, None, &mut size) }
        .map_err(os)?;
    if size == 0 || size > SIGNER_INFO_MAX {
        return Err(format!("informação do signatário com {size} bytes"));
    }
    // u64 cells: CERT_INFO needs 8-byte alignment.
    let mut info = vec![0u64; (size as usize).div_ceil(8)];
    // SAFETY: `info` holds at least `size` bytes, aligned for CERT_INFO.
    unsafe {
        CryptMsgGetParam(
            q.msg,
            CMSG_SIGNER_CERT_INFO_PARAM,
            0,
            Some(info.as_mut_ptr().cast()),
            &mut size,
        )
    }
    .map_err(os)?;
    // SAFETY: the store is valid; the find parameter is the CERT_INFO just written.
    let cert = unsafe {
        CertFindCertificateInStore(
            q.store,
            X509_ASN_ENCODING | PKCS_7_ASN_ENCODING,
            0,
            CERT_FIND_SUBJECT_CERT,
            Some(info.as_ptr().cast::<CERT_INFO>().cast()),
            None,
        )
    };
    if cert.is_null() {
        return Err("certificado do signatário ausente na assinatura".into());
    }
    let cert = Cert(cert);
    // SAFETY: a valid context points to its CERT_INFO.
    let not_before = unsafe { (*(*cert.0).pCertInfo).NotBefore };
    let para = CERT_CHAIN_PARA {
        cbSize: std::mem::size_of::<CERT_CHAIN_PARA>() as u32,
        ..Default::default()
    };
    let mut chain = Chain(std::ptr::null_mut());
    // The chain as of the certificate's start (it is valid only 3 days; the signature's
    // timestamp, checked by WinVerifyTrust, is what keeps it valid later); intermediates come
    // from the signature's own store, nothing is fetched.
    // SAFETY: valid context and store; `chain.0` receives a chain freed by `Chain`.
    unsafe {
        CertGetCertificateChain(
            None,
            cert.0,
            Some(&not_before),
            Some(q.store),
            &para,
            CERT_CHAIN_CACHE_ONLY_URL_RETRIEVAL,
            None,
            &mut chain.0,
        )
    }
    .map_err(os)?;
    let policy = CERT_CHAIN_POLICY_PARA {
        cbSize: std::mem::size_of::<CERT_CHAIN_POLICY_PARA>() as u32,
        dwFlags: MICROSOFT_ROOT_CERT_CHAIN_POLICY_CHECK_APPLICATION_ROOT_FLAG,
        pvExtraPolicyPara: std::ptr::null_mut(),
    };
    let mut status = CERT_CHAIN_POLICY_STATUS {
        cbSize: std::mem::size_of::<CERT_CHAIN_POLICY_STATUS>() as u32,
        ..Default::default()
    };
    // SAFETY: valid chain; `policy` and `status` outlive the call.
    let ran = unsafe {
        CertVerifyCertificateChainPolicy(
            CERT_CHAIN_POLICY_MICROSOFT_ROOT,
            chain.0,
            &policy,
            &mut status,
        )
    };
    if !ran.as_bool() || status.dwError != 0 {
        return Err(format!(
            "a cadeia do signatário não termina numa raiz da Microsoft (0x{:08X})",
            status.dwError
        ));
    }
    Ok(Signer {
        common_name: name_attr(cert.0, szOID_COMMON_NAME),
        organization: name_attr(cert.0, szOID_ORGANIZATION_NAME),
        not_before: system_time(not_before),
    })
}
