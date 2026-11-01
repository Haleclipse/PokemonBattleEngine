use core::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Generation {
    Gen1,
    Gen2,
    Gen3,
    Gen4,
    Gen5,
    Gen6,
    Gen7,
    Gen8,
    Gen9,
}

impl Generation {
    pub const fn number(self) -> u8 {
        match self {
            Self::Gen1 => 1,
            Self::Gen2 => 2,
            Self::Gen3 => 3,
            Self::Gen4 => 4,
            Self::Gen5 => 5,
            Self::Gen6 => 6,
            Self::Gen7 => 7,
            Self::Gen8 => 8,
            Self::Gen9 => 9,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BattleMode {
    Singles,
    Doubles,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FormatId(String);

impl FormatId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for FormatId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BattleSpec {
    pub generation: Generation,
    pub mode: BattleMode,
    pub format: FormatId,
}

impl BattleSpec {
    pub fn new(generation: Generation, mode: BattleMode, format: impl Into<String>) -> Self {
        Self {
            generation,
            mode,
            format: FormatId::new(format),
        }
    }
}
