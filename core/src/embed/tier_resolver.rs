#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct HardwareProfile {
    pub ram_gb: u32,
    pub vram_gb: Option<u32>,
    pub platform: String,
    pub gpu_name: Option<String>,
}

pub fn stub_hardware_profile() -> HardwareProfile {
    HardwareProfile {
        ram_gb: 16,
        vram_gb: None,
        platform: std::env::consts::OS.to_string(),
        gpu_name: None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Tier {
    Light,
    Standard,
    Quality,
}

pub fn resolve_embedding_tier(profile: &HardwareProfile, chat_vram_mb: u32) -> Tier {
    // Hard RAM limit: systems with less than 16 GB RAM are always restricted to Light.
    if profile.ram_gb < 16 {
        return Tier::Light;
    }

    // Quality check: RAM >= 32 OR remaining VRAM >= 12 GB (12288 MB)
    if profile.ram_gb >= 32 {
        return Tier::Quality;
    }

    if let Some(vram_gb) = profile.vram_gb {
        let vram_mb = vram_gb * 1024;
        let remaining_vram = vram_mb.saturating_sub(chat_vram_mb);
        if remaining_vram >= 12288 {
            return Tier::Quality;
        }
    }

    // Light check: total VRAM < 8 GB OR CPU-only (VRAM is None)
    if profile.vram_gb.is_none() {
        return Tier::Light;
    }

    if let Some(vram_gb) = profile.vram_gb {
        if vram_gb < 8 {
            return Tier::Light;
        }
    }

    // Default
    Tier::Standard
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_light_low_ram() {
        let profile = HardwareProfile {
            ram_gb: 8,
            vram_gb: Some(16),
            platform: "windows".to_string(),
            gpu_name: None,
        };
        assert_eq!(resolve_embedding_tier(&profile, 0), Tier::Light);
    }

    #[test]
    fn test_resolve_light_cpu_only() {
        let profile = HardwareProfile {
            ram_gb: 16,
            vram_gb: None,
            platform: "windows".to_string(),
            gpu_name: None,
        };
        assert_eq!(resolve_embedding_tier(&profile, 0), Tier::Light);
    }

    #[test]
    fn test_resolve_light_low_total_vram() {
        let profile = HardwareProfile {
            ram_gb: 16,
            vram_gb: Some(4),
            platform: "windows".to_string(),
            gpu_name: None,
        };
        assert_eq!(resolve_embedding_tier(&profile, 0), Tier::Light);
    }

    #[test]
    fn test_resolve_quality_via_ram() {
        let profile = HardwareProfile {
            ram_gb: 32,
            vram_gb: None,
            platform: "windows".to_string(),
            gpu_name: None,
        };
        assert_eq!(resolve_embedding_tier(&profile, 0), Tier::Quality);
    }

    #[test]
    fn test_resolve_quality_via_vram() {
        let profile = HardwareProfile {
            ram_gb: 16,
            vram_gb: Some(16),
            platform: "windows".to_string(),
            gpu_name: None,
        };
        // 16 GB VRAM - 4096 MB (4 GB) chat headroom = 12 GB remaining >= 12 GB -> Quality
        assert_eq!(resolve_embedding_tier(&profile, 4096), Tier::Quality);
    }

    #[test]
    fn test_resolve_quality_via_both() {
        let profile = HardwareProfile {
            ram_gb: 32,
            vram_gb: Some(16),
            platform: "windows".to_string(),
            gpu_name: None,
        };
        assert_eq!(resolve_embedding_tier(&profile, 4096), Tier::Quality);
    }

    #[test]
    fn test_resolve_standard_default() {
        let profile = HardwareProfile {
            ram_gb: 16,
            vram_gb: Some(10),
            platform: "windows".to_string(),
            gpu_name: None,
        };
        assert_eq!(resolve_embedding_tier(&profile, 0), Tier::Standard);
    }

    #[test]
    fn test_resolve_standard_due_to_chat_headroom() {
        let profile = HardwareProfile {
            ram_gb: 24,
            vram_gb: Some(16),
            platform: "windows".to_string(),
            gpu_name: None,
        };
        // 16 GB VRAM - 8192 MB (8 GB) chat headroom = 8 GB remaining < 12 GB.
        // RAM = 24 < 32.
        // Total VRAM = 16 >= 8, RAM = 24 >= 16.
        // So neither Quality nor Light matches -> Standard.
        assert_eq!(resolve_embedding_tier(&profile, 8192), Tier::Standard);
    }

    #[test]
    fn test_resolve_standard_true_negative() {
        let profile = HardwareProfile {
            ram_gb: 24,
            vram_gb: Some(8),
            platform: "windows".to_string(),
            gpu_name: None,
        };
        // 8 GB VRAM - 8192 MB (8 GB) chat headroom = 0 GB remaining < 12 GB.
        // RAM = 24 < 32.
        // Total VRAM = 8 >= 8, RAM = 24 >= 16.
        // So neither Quality nor Light matches -> Standard.
        assert_eq!(resolve_embedding_tier(&profile, 8192), Tier::Standard);
    }
}
