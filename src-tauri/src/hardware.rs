//! Hardware detection module.
//!
//! Probes the system at startup to determine GPU capabilities.
//! Uses `nvidia-smi` to detect NVIDIA GPUs and their VRAM.
//! Falls back gracefully to CPU-only mode when no GPU is found.

use std::process::Command;

/// Minimum VRAM (in MB) required to enable CUDA acceleration.
const MIN_CUDA_VRAM_MB: u64 = 4096;

/// System hardware profile detected at startup.
#[derive(Debug, Clone, serde::Serialize)]
pub struct HardwareInfo {
    /// Whether an NVIDIA GPU was detected.
    pub has_nvidia: bool,
    /// GPU product name (e.g. "NVIDIA GeForce RTX 4070").
    pub gpu_name: Option<String>,
    /// Total VRAM in megabytes.
    pub vram_mb: Option<u64>,
    /// Whether CUDA should be used for processing.
    /// True only when an NVIDIA GPU with >= 4096 MB VRAM is present.
    pub use_cuda: bool,
}

impl HardwareInfo {
    /// Returns a CPU-only profile. Used as a fallback when CUDA fails.
    pub fn cpu_only() -> Self {
        Self {
            has_nvidia: false,
            gpu_name: None,
            vram_mb: None,
            use_cuda: false,
        }
    }
}

/// Detect available hardware by querying `nvidia-smi`.
///
/// This function never panics. If `nvidia-smi` is missing, fails to run,
/// or returns unparseable output, it returns a CPU-only profile.
pub fn detect_hardware() -> HardwareInfo {
    // Query GPU name and total memory in one call using csv format.
    // --query-gpu=name,memory.total returns e.g. "NVIDIA GeForce RTX 4070, 12282 MiB"
    let mut smi_cmd = Command::new("nvidia-smi");
    smi_cmd.args(["--query-gpu=name,memory.total", "--format=csv,noheader,nounits"]);
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        smi_cmd.creation_flags(0x08000000);
    }
    let output = match smi_cmd.output()
    {
        Ok(output) if output.status.success() => output,
        Ok(_) => {
            // nvidia-smi exists but returned a non-zero exit code.
            log::warn!("nvidia-smi exited with non-zero status; assuming CPU-only");
            return HardwareInfo::cpu_only();
        }
        Err(e) => {
            // nvidia-smi not found or could not be executed.
            log::info!("nvidia-smi not available ({e}); assuming CPU-only");
            return HardwareInfo::cpu_only();
        }
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    // Take the first GPU line (index 0) if multiple GPUs are present.
    let line = match stdout.lines().next() {
        Some(l) if !l.trim().is_empty() => l.trim(),
        _ => {
            log::warn!("nvidia-smi returned empty output; assuming CPU-only");
            return HardwareInfo::cpu_only();
        }
    };

    // Expected format: "GPU Name, VRAM_MB"
    // e.g. "NVIDIA GeForce RTX 4070, 12282"
    let parts: Vec<&str> = line.splitn(2, ',').collect();
    if parts.len() < 2 {
        log::warn!("nvidia-smi output in unexpected format: {line}");
        return HardwareInfo::cpu_only();
    }

    let gpu_name = parts[0].trim().to_string();
    let vram_mb: u64 = match parts[1].trim().parse() {
        Ok(v) => v,
        Err(e) => {
            log::warn!("Could not parse VRAM value '{}': {e}", parts[1].trim());
            return HardwareInfo {
                has_nvidia: true,
                gpu_name: Some(gpu_name),
                vram_mb: None,
                use_cuda: false,
            };
        }
    };

    let use_cuda = vram_mb >= MIN_CUDA_VRAM_MB;

    log::info!(
        "Detected GPU: {gpu_name} with {vram_mb} MB VRAM (CUDA {})",
        if use_cuda { "enabled" } else { "disabled — below threshold" }
    );

    HardwareInfo {
        has_nvidia: true,
        gpu_name: Some(gpu_name),
        vram_mb: Some(vram_mb),
        use_cuda,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_only_profile_is_correct() {
        let info = HardwareInfo::cpu_only();
        assert!(!info.has_nvidia);
        assert!(info.gpu_name.is_none());
        assert!(info.vram_mb.is_none());
        assert!(!info.use_cuda);
    }

    #[test]
    fn detect_hardware_does_not_panic() {
        // Should return a valid struct regardless of whether nvidia-smi exists.
        let info = detect_hardware();
        if info.has_nvidia {
            assert!(info.gpu_name.is_some());
        }
        // use_cuda must be false when VRAM is below threshold or absent.
        if let Some(vram) = info.vram_mb {
            assert_eq!(info.use_cuda, vram >= MIN_CUDA_VRAM_MB);
        } else {
            assert!(!info.use_cuda);
        }
    }
}
