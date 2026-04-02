//! Lightweight encryption helpers for sensitive DB fields.
//!
//! On Windows, uses DPAPI (CryptProtectData / CryptUnprotectData) which
//! ties the ciphertext to the current Windows user account — other users
//! or machines cannot decrypt the data.
//!
//! On non-Windows platforms, falls back to base64 encoding (obfuscation
//! only — suitable for dev/CI but not production secret storage).
//!
//! Encrypted values are prefixed with "dpapi:" so we can detect and
//! transparently migrate legacy plaintext values.

/// Prefix that marks a value as DPAPI-encrypted.
const DPAPI_PREFIX: &str = "dpapi:";

/// Encrypt a plaintext string for storage.
///
/// - Windows: DPAPI → base64 with "dpapi:" prefix
/// - Other:   base64 only (obfuscation fallback)
pub fn encrypt_sensitive(plaintext: &str) -> Result<String, String> {
    if plaintext.is_empty() {
        return Ok(String::new());
    }

    #[cfg(target_os = "windows")]
    {
        encrypt_dpapi(plaintext)
    }

    #[cfg(not(target_os = "windows"))]
    {
        // Fallback: base64 obfuscation (not real encryption)
        use base64::Engine;
        let encoded = base64::engine::general_purpose::STANDARD.encode(plaintext.as_bytes());
        Ok(format!("{}{}", DPAPI_PREFIX, encoded))
    }
}

/// Decrypt a stored value back to plaintext.
///
/// If the value does not start with "dpapi:", it is returned as-is
/// (legacy migration: old plaintext values work transparently).
pub fn decrypt_sensitive(encrypted: &str) -> Result<String, String> {
    if encrypted.is_empty() {
        return Ok(String::new());
    }

    // Legacy plaintext — return as-is for transparent migration
    if !encrypted.starts_with(DPAPI_PREFIX) {
        return Ok(encrypted.to_string());
    }

    let payload = &encrypted[DPAPI_PREFIX.len()..];

    #[cfg(target_os = "windows")]
    {
        decrypt_dpapi(payload)
    }

    #[cfg(not(target_os = "windows"))]
    {
        // Fallback: decode base64
        use base64::Engine;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(payload)
            .map_err(|e| format!("base64 decode failed: {}", e))?;
        String::from_utf8(bytes).map_err(|e| format!("UTF-8 decode failed: {}", e))
    }
}

// ═══════════════════════════════════════════════════════════════════
//  Windows DPAPI implementation
// ═══════════════════════════════════════════════════════════════════

#[cfg(target_os = "windows")]
fn encrypt_dpapi(plaintext: &str) -> Result<String, String> {
    use base64::Engine;
    use windows::core::PCWSTR;
    use windows::Win32::Security::Cryptography::{
        CryptProtectData, CRYPT_INTEGER_BLOB,
    };

    let plain_bytes = plaintext.as_bytes();
    let input_blob = CRYPT_INTEGER_BLOB {
        cbData: plain_bytes.len() as u32,
        pbData: plain_bytes.as_ptr() as *mut u8,
    };
    let mut output_blob = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: std::ptr::null_mut(),
    };

    unsafe {
        CryptProtectData(
            &input_blob,
            PCWSTR::null(),     // description
            None,               // optional entropy
            None,               // reserved
            None,               // prompt struct
            0,                  // flags
            &mut output_blob,
        )
        .map_err(|e| format!("DPAPI CryptProtectData failed: {}", e))?;
    }

    let encrypted_bytes = unsafe {
        std::slice::from_raw_parts(output_blob.pbData, output_blob.cbData as usize)
    };
    let encoded = base64::engine::general_purpose::STANDARD.encode(encrypted_bytes);

    // Free the DPAPI-allocated buffer
    unsafe {
        let _ = windows::Win32::Foundation::LocalFree(windows::Win32::Foundation::HLOCAL(output_blob.pbData as *mut core::ffi::c_void));
    }

    Ok(format!("{}{}", DPAPI_PREFIX, encoded))
}

#[cfg(target_os = "windows")]
fn decrypt_dpapi(base64_payload: &str) -> Result<String, String> {
    use base64::Engine;
    use windows::Win32::Security::Cryptography::{
        CryptUnprotectData, CRYPT_INTEGER_BLOB,
    };

    let cipher_bytes = base64::engine::general_purpose::STANDARD
        .decode(base64_payload)
        .map_err(|e| format!("base64 decode failed: {}", e))?;

    let input_blob = CRYPT_INTEGER_BLOB {
        cbData: cipher_bytes.len() as u32,
        pbData: cipher_bytes.as_ptr() as *mut u8,
    };
    let mut output_blob = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: std::ptr::null_mut(),
    };

    unsafe {
        CryptUnprotectData(
            &input_blob,
            Some(std::ptr::null_mut()), // description out
            None,               // optional entropy
            None,               // reserved
            None,               // prompt struct
            0,                  // flags
            &mut output_blob,
        )
        .map_err(|e| format!("DPAPI CryptUnprotectData failed: {}", e))?;
    }

    let decrypted_bytes = unsafe {
        std::slice::from_raw_parts(output_blob.pbData, output_blob.cbData as usize)
    };
    let result = String::from_utf8(decrypted_bytes.to_vec())
        .map_err(|e| format!("UTF-8 decode failed: {}", e))?;

    // Free the DPAPI-allocated buffer
    unsafe {
        let _ = windows::Win32::Foundation::LocalFree(windows::Win32::Foundation::HLOCAL(output_blob.pbData as *mut core::ffi::c_void));
    }

    Ok(result)
}
