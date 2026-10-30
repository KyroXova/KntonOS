//! Handheld Ambient Light Sensor (ALS) driver and lux EMA filtering.

// Ambient lighting categories and hardware interface.

/// Hardware interface through which the ALS communicates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlsHardwareInterface {
    /// ACPI HID Sensor Hub (Class 0x0020, Sensor Usage 0x0041 - Illuminance).
    AcpiHid { acpi_id: u32 },
    /// Direct I2C Bus controller (e.g. OPT3001, APDS-9960, TSL2591).
    I2c { bus: u8, address: u8 },
    /// Synthetic / Emulated sensor for platform testing.
    Emulated,
}

/// Six ambient lighting tiers matching handheld gaming environments.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LuxCategory {
    /// 0 - 10 Lux: Dark room, bedtime gaming.
    PitchDark,
    /// 11 - 50 Lux: Dim evening indoor living room.
    DimIndoor,
    /// 51 - 300 Lux: Normal indoor room or cafe.
    NormalIndoor,
    /// 301 - 1,000 Lux: Bright office, library, or shopping mall.
    Office,
    /// 1,001 - 10,000 Lux: Overcast outdoor daylight, shaded patio.
    BrightDaylight,
    /// > 10,000 Lux: Direct outdoor sunlight at noon.
    DirectSunlight,
}

impl LuxCategory {
    /// Classifies an illuminance value in Lux into a lighting tier.
    pub const fn from_lux(lux: u32) -> Self {
        match lux {
            0..=10 => Self::PitchDark,
            11..=50 => Self::DimIndoor,
            51..=300 => Self::NormalIndoor,
            301..=1_000 => Self::Office,
            1_001..=10_000 => Self::BrightDaylight,
            _ => Self::DirectSunlight,
        }
    }
}

// ALS driver and fixed-point exponential moving average (EMA) filter.

/// Ambient Light Sensor driver instance with noise filtering.
#[derive(Debug, Clone)]
pub struct AlsDriver {
    pub interface: AlsHardwareInterface,
    pub raw_lux: u32,
    pub filtered_lux_milli: u32, // Stored in milli-lux (lux * 1000) for fixed-point precision
    pub alpha_permille: u32,     // EMA weighting factor in permille (0..1000, e.g. 250 = 0.25)
    pub last_sample_time_ms: u64,
    pub sample_count: u64,
}

impl AlsDriver {
    /// Creates a new ALS driver with default EMA smoothing (alpha = 0.25).
    pub const fn new(interface: AlsHardwareInterface) -> Self {
        Self {
            interface,
            raw_lux: 0,
            filtered_lux_milli: 0,
            alpha_permille: 250, // 25% new sample weight, 75% history
            last_sample_time_ms: 0,
            sample_count: 0,
        }
    }

    /// Configures the EMA filter alpha weight (100 to 1000 permille).
    pub fn set_alpha(&mut self, permille: u32) {
        self.alpha_permille = permille.clamp(100, 1000);
    }

    /// Ingests a new raw sensor reading in Lux and updates the filtered estimate.
    pub fn update_reading(&mut self, raw_lux: u32, timestamp_ms: u64) -> u32 {
        self.raw_lux = raw_lux;
        let new_milli = raw_lux.saturating_mul(1000);

        if self.sample_count == 0 {
            // Seed the initial filter state with first valid sample
            self.filtered_lux_milli = new_milli;
        } else {
            // Fixed-point EMA calculation:
            // S_t = alpha * Y_t + (1 - alpha) * S_{t-1}
            let alpha = self.alpha_permille as u64;
            let one_minus_alpha = (1000 - self.alpha_permille) as u64;
            let filtered = (alpha * (new_milli as u64) + one_minus_alpha * (self.filtered_lux_milli as u64)) / 1000;
            self.filtered_lux_milli = filtered.min(u32::MAX as u64) as u32;
        }

        self.last_sample_time_ms = timestamp_ms;
        self.sample_count += 1;

        self.filtered_lux()
    }

    /// Returns the current smoothed ambient illuminance in integer Lux.
    #[inline(always)]
    pub fn filtered_lux(&self) -> u32 {
        self.filtered_lux_milli / 1000
    }

    /// Returns the current raw sensor reading in Lux.
    #[inline(always)]
    pub fn raw_lux(&self) -> u32 {
        self.raw_lux
    }

    /// Returns the active lighting category based on filtered illuminance.
    #[inline(always)]
    pub fn current_category(&self) -> LuxCategory {
        LuxCategory::from_lux(self.filtered_lux())
    }
}

// Unit tests and verification.

/// Unit test verifying ALS driver initialization, lux tier classification, and EMA smoothing.
pub fn verify_als_driver_and_lux_filter() -> bool {
    // 1. Verify Lux Category boundaries
    if LuxCategory::from_lux(0) != LuxCategory::PitchDark {
        return false;
    }
    if LuxCategory::from_lux(10) != LuxCategory::PitchDark {
        return false;
    }
    if LuxCategory::from_lux(11) != LuxCategory::DimIndoor {
        return false;
    }
    if LuxCategory::from_lux(50) != LuxCategory::DimIndoor {
        return false;
    }
    if LuxCategory::from_lux(150) != LuxCategory::NormalIndoor {
        return false;
    }
    if LuxCategory::from_lux(500) != LuxCategory::Office {
        return false;
    }
    if LuxCategory::from_lux(3000) != LuxCategory::BrightDaylight {
        return false;
    }
    if LuxCategory::from_lux(50000) != LuxCategory::DirectSunlight {
        return false;
    }

    // 2. Verify EMA smoothing on step response
    let mut als = AlsDriver::new(AlsHardwareInterface::Emulated);
    als.set_alpha(200); // 20% alpha

    // Initial reading at 100 Lux seeds filter exactly
    let r0 = als.update_reading(100, 0);
    if r0 != 100 || als.current_category() != LuxCategory::NormalIndoor {
        return false;
    }

    // Step change up to 600 Lux (office light turned on)
    // Sample 1: 0.2 * 600 + 0.8 * 100 = 120 + 80 = 200 Lux
    let r1 = als.update_reading(600, 100);
    if r1 != 200 {
        return false;
    }

    // Sample 2: 0.2 * 600 + 0.8 * 200 = 120 + 160 = 280 Lux
    let r2 = als.update_reading(600, 200);
    if r2 != 280 {
        return false;
    }

    // Sample 3: 0.2 * 600 + 0.8 * 280 = 120 + 224 = 344 Lux -> transitions into Office category
    let r3 = als.update_reading(600, 300);
    if r3 != 344 || als.current_category() != LuxCategory::Office {
        return false;
    }

    // 3. Transient shadow dip: a hand briefly occludes the sensor for 1 sample (e.g. 50 Lux)
    // S_4: 0.2 * 50 + 0.8 * 344 = 10 + 275.2 = 285 Lux (dampened, doesn't plunge to 50)
    let r4 = als.update_reading(50, 400);
    if r4 < 280 || r4 > 290 {
        return false;
    }

    true
}
