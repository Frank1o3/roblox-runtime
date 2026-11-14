#![forbid(unsafe_code)]

use std::collections::HashMap;
use std::fmt;

use super::elf::{DynSym, TlsSegment};
use super::reloc::SymbolResolver;

const STT_TLS: u8 = 6;

const SHN_UNDEF: u16 = 0;

fn round_up(value: u64, align: u64) -> Option<u64> {
    if align <= 1 {
        return Some(value);
    }
    let mask = align - 1;
    value.checked_add(mask).map(|v| v & !mask)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TlsError {
    BadAlign(u64),

    FileLargerThanMem(u64, u64),

    Overflow(&'static str),

    TdataOutOfFile(u64),
}

impl fmt::Display for TlsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadAlign(a) => write!(f, "PT_TLS p_align {a} is not a power of two"),
            Self::FileLargerThanMem(fz, mz) => {
                write!(f, "PT_TLS p_filesz {fz} exceeds p_memsz {mz}")
            }
            Self::Overflow(what) => write!(f, "static-TLS layout arithmetic overflowed: {what}"),
            Self::TdataOutOfFile(off) => {
                write!(
                    f,
                    "PT_TLS .tdata at file offset {off:#x} is past the file bytes"
                )
            }
        }
    }
}

impl std::error::Error for TlsError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TlsModule {
    pub tp_offset: i64,

    pub block_offset: usize,

    pub size: usize,
}

#[derive(Debug, Clone, Default)]
pub struct TlsLayout {
    init_block: Vec<u8>,

    modules: Vec<TlsModule>,

    accumulated: u64,

    tls_defs: HashMap<String, (i64, u64)>,
}

impl TlsLayout {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_module(
        &mut self,
        tls: &TlsSegment,
        file: &[u8],
        tdata_off: u64,
        dynsyms: &[DynSym],
    ) -> Result<TlsModule, TlsError> {
        if tls.align > 1 && !tls.align.is_power_of_two() {
            return Err(TlsError::BadAlign(tls.align));
        }
        if tls.file_size > tls.mem_size {
            return Err(TlsError::FileLargerThanMem(tls.file_size, tls.mem_size));
        }

        let aligned_acc = round_up(self.accumulated, tls.align)
            .ok_or(TlsError::Overflow("accumulated alignment"))?;
        let aligned_size =
            round_up(tls.mem_size, tls.align).ok_or(TlsError::Overflow("module size alignment"))?;
        let offset_i = aligned_acc
            .checked_add(aligned_size)
            .ok_or(TlsError::Overflow("offset accumulation"))?;
        let tp_offset =
            -(i64::try_from(offset_i).map_err(|_| TlsError::Overflow("offset as i64"))?);

        let block_offset = round_up(self.init_block.len() as u64, tls.align)
            .ok_or(TlsError::Overflow("block_offset alignment"))?;
        let block_offset = usize::try_from(block_offset)
            .map_err(|_| TlsError::Overflow("block_offset as usize"))?;
        let mem_size =
            usize::try_from(tls.mem_size).map_err(|_| TlsError::Overflow("mem_size as usize"))?;
        let block_end = block_offset
            .checked_add(mem_size)
            .ok_or(TlsError::Overflow("block_offset + mem_size"))?;
        if self.init_block.len() < block_end {
            self.init_block.resize(block_end, 0);
        }

        let file_size =
            usize::try_from(tls.file_size).map_err(|_| TlsError::Overflow("file_size as usize"))?;
        if file_size > 0 {
            let file_off = usize::try_from(tdata_off)
                .map_err(|_| TlsError::Overflow("tdata file offset as usize"))?;
            let file_end = file_off
                .checked_add(file_size)
                .ok_or(TlsError::Overflow("tdata file end"))?;
            let src = file
                .get(file_off..file_end)
                .ok_or(TlsError::TdataOutOfFile(tdata_off))?;
            self.init_block[block_offset..block_offset + file_size].copy_from_slice(src);
        }

        for sym in dynsyms {
            if sym.sym_type == STT_TLS && sym.shndx != SHN_UNDEF && !sym.name.is_empty() {
                self.tls_defs
                    .entry(sym.name.clone())
                    .or_insert((tp_offset, sym.value));
            }
        }

        self.accumulated = offset_i;
        let module = TlsModule {
            tp_offset,
            block_offset,
            size: mem_size,
        };
        self.modules.push(module);
        Ok(module)
    }

    pub fn init_block(&self) -> &[u8] {
        &self.init_block
    }

    pub fn modules(&self) -> &[TlsModule] {
        &self.modules
    }

    pub fn tp_offset_of(&self, name: &str) -> Option<i64> {
        self.tls_defs
            .get(name)
            .map(|&(tp_offset, value)| tp_offset.wrapping_add(value as i64))
    }
}

pub struct TlsResolver<'a, R: SymbolResolver> {
    inner: &'a R,
    dynsyms: &'a [DynSym],
    layout: &'a TlsLayout,

    own_tp_offset: Option<i64>,
}

impl<'a, R: SymbolResolver> TlsResolver<'a, R> {
    pub fn new(
        inner: &'a R,
        dynsyms: &'a [DynSym],
        layout: &'a TlsLayout,
        own_tp_offset: Option<i64>,
    ) -> Self {
        Self {
            inner,
            dynsyms,
            layout,
            own_tp_offset,
        }
    }
}

impl<R: SymbolResolver> SymbolResolver for TlsResolver<'_, R> {
    fn resolve_symbol(&self, sym_index: u32) -> Option<u64> {
        self.inner.resolve_symbol(sym_index)
    }

    fn resolve_tls_offset(&self, sym_index: u32) -> Option<u64> {
        if sym_index == 0 {
            return self.own_tp_offset.map(|v| v as u64);
        }
        let sym = self.dynsyms.get(sym_index as usize)?;

        self.layout
            .tp_offset_of(&sym.name)
            .map(|v| v as u64)
            .or_else(|| {
                self.own_tp_offset
                    .map(|tp| tp.wrapping_add(sym.value as i64) as u64)
            })
    }
}
