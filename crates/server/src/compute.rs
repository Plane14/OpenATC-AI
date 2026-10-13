//! Device choices shared by the three inference workers.
use clap::Args;
use serde::Serialize;

#[derive(Args, Clone, Debug, Default)]
#[allow(clippy::struct_excessive_bools)] // Independent command-line overrides.
pub struct CpuOptions {
    /// Run all AI inference on CPU.
    #[arg(long)]
    pub force_cpu: bool,
    /// Run the language model on CPU.
    #[arg(long)]
    pub force_llm_cpu: bool,
    /// Run speech recognition on CPU.
    #[arg(long)]
    pub force_stt_cpu: bool,
    /// Run speech synthesis on CPU.
    #[arg(long)]
    pub force_tts_cpu: bool,
}
impl CpuOptions {
    pub fn llm(&self) -> bool {
        self.force_cpu || self.force_llm_cpu
    }
    pub fn stt(&self) -> bool {
        self.force_cpu || self.force_stt_cpu
    }
    pub fn tts(&self) -> bool {
        self.force_cpu || self.force_tts_cpu
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct DeviceStatus {
    pub device: String,
    pub reason: String,
}
impl DeviceStatus {
    pub fn new(device: &str, reason: impl Into<String>) -> Self {
        Self {
            device: device.into(),
            reason: reason.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn global_override_wins_and_individual_overrides_are_independent() {
        for bits in 0..16 {
            let options = CpuOptions {
                force_cpu: bits & 1 != 0,
                force_llm_cpu: bits & 2 != 0,
                force_stt_cpu: bits & 4 != 0,
                force_tts_cpu: bits & 8 != 0,
            };
            assert_eq!(options.llm(), bits & 3 != 0);
            assert_eq!(options.stt(), bits & 5 != 0);
            assert_eq!(options.tts(), bits & 9 != 0);
        }
    }
}
