//! MSZIP container decoder (Windows Compression API format) used by `versionData.mszyml`.
use flate2::{Decompress, FlushDecompress};

const SIGNATURE: [u8; 4] = [0x0A, 0x51, 0xE5, 0xC0];
const HEADER_LEN: usize = 24;
const WINDOW: usize = 32 * 1024;

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum MszipError {
    #[error("assinatura MSZIP inválida")]
    BadSignature,
    #[error("dados MSZIP truncados")]
    Truncated,
    #[error("bloco MSZIP sem marcador CK")]
    MissingCk,
    #[error("deflate inválido: {0}")]
    Deflate(String),
    #[error("tamanho descomprimido diverge: esperado {expected}, obtido {actual}")]
    SizeMismatch { expected: usize, actual: usize },
}

/// Decodes an MSZIP stream. Every block inflates to at most 32 KiB and uses the
/// last 32 KiB already produced as its dictionary.
pub fn decode(input: &[u8]) -> Result<Vec<u8>, MszipError> {
    let header = input.get(..HEADER_LEN).ok_or(MszipError::Truncated)?;
    if header[..4] != SIGNATURE {
        return Err(MszipError::BadSignature);
    }
    let size_bytes: [u8; 8] = header[8..16]
        .try_into()
        .map_err(|_| MszipError::Truncated)?;
    let expected =
        usize::try_from(u64::from_le_bytes(size_bytes)).map_err(|_| MszipError::Truncated)?;

    let mut out: Vec<u8> = Vec::new();
    let mut pos = HEADER_LEN;
    while pos < input.len() {
        let len_bytes: [u8; 4] = input
            .get(pos..pos + 4)
            .and_then(|s| s.try_into().ok())
            .ok_or(MszipError::Truncated)?;
        let block_len = u32::from_le_bytes(len_bytes) as usize;
        pos += 4;
        let end = pos.checked_add(block_len).ok_or(MszipError::Truncated)?;
        let payload = input.get(pos..end).ok_or(MszipError::Truncated)?;
        pos = end;
        let deflate = payload.strip_prefix(b"CK").ok_or(MszipError::MissingCk)?;

        let mut dec = Decompress::new(false);
        if !out.is_empty() {
            let start = out.len().saturating_sub(WINDOW);
            dec.set_dictionary(&out[start..])
                .map_err(|e| MszipError::Deflate(e.to_string()))?;
        }
        let mut block = Vec::with_capacity(WINDOW + 1024);
        dec.decompress_vec(deflate, &mut block, FlushDecompress::Finish)
            .map_err(|e| MszipError::Deflate(e.to_string()))?;
        out.extend_from_slice(&block);
    }
    if out.len() != expected {
        return Err(MszipError::SizeMismatch {
            expected,
            actual: out.len(),
        });
    }
    Ok(out)
}
