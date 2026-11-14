use std::collections::BTreeMap;

use crate::elf::DynSym;
use crate::reloc::SymbolResolver;

const STB_LOCAL: u8 = 0;
const STB_GLOBAL: u8 = 1;
const STB_WEAK: u8 = 2;
const SHN_UNDEF: u16 = 0;
const SHN_ABS: u16 = 0xfff1;

pub trait SymbolProvider {
    fn resolve(&self, name: &str) -> Option<u64>;
}

#[derive(Default)]
pub struct Scope {
    providers: Vec<Box<dyn SymbolProvider>>,
}

impl Scope {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, provider: Box<dyn SymbolProvider>) -> &mut Self {
        self.providers.push(provider);
        self
    }

    pub fn resolve(&self, name: &str) -> Option<u64> {
        self.providers.iter().find_map(|p| p.resolve(name))
    }
}

pub struct AddressProvider(pub BTreeMap<String, u64>);

impl SymbolProvider for AddressProvider {
    fn resolve(&self, name: &str) -> Option<u64> {
        self.0.get(name).copied()
    }
}

pub struct ObjectProvider {
    symbols: BTreeMap<String, u64>,
}

impl ObjectProvider {
    pub fn new(base: u64, symbols: &[DynSym]) -> Self {
        let mut definitions = BTreeMap::new();
        for sym in symbols {
            if sym.name.is_empty()
                || sym.shndx == SHN_UNDEF
                || !matches!(sym.bind, STB_GLOBAL | STB_WEAK)
                || !matches!(sym.sym_type, 0 | 1 | 2 | 10)
            {
                continue;
            }
            let address = if sym.shndx == SHN_ABS {
                sym.value
            } else {
                base.wrapping_add(sym.value)
            };
            match definitions.get(&sym.name) {
                Some(_) if sym.bind == STB_WEAK => {}
                _ => {
                    definitions.insert(sym.name.clone(), address);
                }
            }
        }
        Self {
            symbols: definitions,
        }
    }
}

impl SymbolProvider for ObjectProvider {
    fn resolve(&self, name: &str) -> Option<u64> {
        self.symbols.get(name).copied()
    }
}

pub struct ScopedResolver<'a> {
    scope: &'a Scope,
    dynsyms: &'a [DynSym],
}

impl<'a> ScopedResolver<'a> {
    pub fn new(scope: &'a Scope, dynsyms: &'a [DynSym]) -> Self {
        Self { scope, dynsyms }
    }
}

impl SymbolResolver for ScopedResolver<'_> {
    fn resolve_symbol(&self, sym_index: u32) -> Option<u64> {
        let sym = self.dynsyms.get(sym_index as usize)?;
        if sym.bind == STB_LOCAL {
            return None;
        }
        self.scope
            .resolve(&sym.name)
            .or_else(|| (sym.bind == STB_WEAK).then_some(0))
    }

    fn resolve_tls_offset(&self, _sym_index: u32) -> Option<u64> {
        None
    }
}
