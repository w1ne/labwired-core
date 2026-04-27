pub enum CpuInterruptLevel {
    Level1,
    Level2,
    Level3,
    Level4,
    Level5,
    Level6,
    Level7,
}

impl CpuInterruptLevel {
    pub const fn mask(&self) -> u32 {
        match &self {
            CpuInterruptLevel::Level1 => 407551u32,
            CpuInterruptLevel::Level2 => 3670016u32,
            CpuInterruptLevel::Level3 => 683706368u32,
            CpuInterruptLevel::Level4 => 1392508928u32,
            CpuInterruptLevel::Level5 => 2214658048u32,
            CpuInterruptLevel::Level6 => 0u32,
            CpuInterruptLevel::Level7 => 16384u32,
        }
    }
}
