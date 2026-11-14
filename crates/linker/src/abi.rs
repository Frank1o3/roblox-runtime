#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Abi {
    X86_64,
    Arm64V8a,
}

impl Abi {
    #[cfg(target_arch = "x86_64")]
    pub const HOST: Self = Self::X86_64;

    #[cfg(target_arch = "aarch64")]
    pub const HOST: Self = Self::Arm64V8a;

    pub const fn elf_machine(self) -> u16 {
        match self {
            Self::X86_64 => 62,
            Self::Arm64V8a => 183,
        }
    }

    pub fn from_elf_machine(machine: u16) -> Option<Self> {
        [Self::X86_64, Self::Arm64V8a]
            .into_iter()
            .find(|abi| abi.elf_machine() == machine)
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::X86_64 => "x86-64",
            Self::Arm64V8a => "ARM64",
        }
    }
}
