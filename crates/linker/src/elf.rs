#![forbid(unsafe_code)]

use std::fmt;

use super::reloc::{Rela, RelocKind};
use crate::abi::Abi;

const ELF_MAGIC: [u8; 4] = [0x7f, b'E', b'L', b'F'];

const ELFCLASS64: u8 = 2;

const ELFDATA2LSB: u8 = 1;

const EI_CLASS: usize = 4;

const EI_DATA: usize = 5;

const ET_DYN: u16 = 3;

const EHDR_SIZE: usize = 64;

const PHDR_SIZE: usize = 56;

const DYN_SIZE: usize = 16;

const SYM_SIZE: usize = 24;

const GNU_HASH_BLOOM_WORD_SIZE: usize = 8;

const PT_LOAD: u32 = 1;

const PT_DYNAMIC: u32 = 2;

const PT_TLS: u32 = 7;

const PT_GNU_RELRO: u32 = 0x6474_e552;

pub const PF_X: u32 = 1;

pub const PF_W: u32 = 2;

pub const PF_R: u32 = 4;

const DT_NULL: i64 = 0;
const DT_NEEDED: i64 = 1;
const DT_PLTRELSZ: i64 = 2;
const DT_HASH: i64 = 4;
const DT_STRTAB: i64 = 5;
const DT_SYMTAB: i64 = 6;
const DT_RELA: i64 = 7;
const DT_RELASZ: i64 = 8;
const DT_RELAENT: i64 = 9;
const DT_STRSZ: i64 = 10;
const DT_SYMENT: i64 = 11;
const DT_INIT: i64 = 12;
const DT_SONAME: i64 = 14;
const DT_BIND_NOW: i64 = 24;
const DT_INIT_ARRAY: i64 = 25;
const DT_FINI_ARRAY: i64 = 26;
const DT_INIT_ARRAYSZ: i64 = 27;
const DT_FINI_ARRAYSZ: i64 = 28;
const DT_FLAGS: i64 = 30;
const DT_PLTREL: i64 = 20;
const DT_JMPREL: i64 = 23;

const DT_RELR: i64 = 36;

const DT_RELRSZ: i64 = 35;

const DT_RELRENT: i64 = 37;

const DT_ANDROID_RELA: i64 = 0x6000_0011;

const DT_ANDROID_RELASZ: i64 = 0x6000_0012;

const DT_ANDROID_RELR: i64 = 0x6fff_e000;

const DT_ANDROID_RELRSZ: i64 = 0x6fff_e001;

const DT_ANDROID_RELRENT: i64 = 0x6fff_e003;

const APS2_MAGIC: [u8; 4] = *b"APS2";

const RELOCATION_GROUPED_BY_INFO_FLAG: i64 = 1;

const RELOCATION_GROUPED_BY_OFFSET_DELTA_FLAG: i64 = 2;

const RELOCATION_GROUPED_BY_ADDEND_FLAG: i64 = 4;

const RELOCATION_GROUP_HAS_ADDEND_FLAG: i64 = 8;

const DT_GNU_HASH: i64 = 0x6fff_fef5;

const DT_FLAGS_1: i64 = 0x6fff_fffb;

const DF_BIND_NOW: u64 = 0x8;

const DF_1_NOW: u64 = 0x1;

fn rela_sym(info: u64) -> u32 {
    (info >> 32) as u32
}

fn rela_type(info: u64) -> u32 {
    (info & 0xffff_ffff) as u32
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ElfError {
    Truncated { offset: usize, need: usize },

    BadMagic,

    NotElf64(u8),

    NotLittleEndian(u8),

    NotSharedObject(u16),

    UnsupportedMachine(u16),

    BadPhEntSize(u16),

    BadEntSize(i64, u64, u64),

    UnmappedVaddr(u64),

    MissingDynamic(&'static str),

    BadAndroidMagic(usize),

    BadSleb128(usize),

    BadAndroidReloc(u64, u64),

    BadRelocationType(u32),
}

impl fmt::Display for ElfError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated { offset, need } => {
                write!(
                    f,
                    "ELF image truncated: need {need} bytes at offset {offset}"
                )
            }
            Self::BadMagic => write!(f, "not an ELF file (bad magic)"),
            Self::NotElf64(c) => write!(f, "not ELFCLASS64 (e_ident[EI_CLASS]={c})"),
            Self::NotLittleEndian(d) => write!(f, "not ELFDATA2LSB (e_ident[EI_DATA]={d})"),
            Self::NotSharedObject(t) => write!(f, "not ET_DYN (e_type={t})"),
            Self::UnsupportedMachine(m) => write!(
                f,
                "e_machine {m} is neither x86-64 nor ARM64, the two machines Eclipse loads"
            ),
            Self::BadPhEntSize(s) => write!(f, "unexpected e_phentsize {s} (want {PHDR_SIZE})"),
            Self::BadEntSize(tag, found, want) => {
                write!(f, "dynamic tag {tag:#x} entry size {found} (want {want})")
            }
            Self::UnmappedVaddr(v) => {
                write!(
                    f,
                    "virtual address {v:#x} is not inside any PT_LOAD segment"
                )
            }
            Self::MissingDynamic(what) => write!(f, "missing required dynamic entry: {what}"),
            Self::BadAndroidMagic(off) => {
                write!(
                    f,
                    "Android-packed relocations: bad APS2 magic at offset {off}"
                )
            }
            Self::BadSleb128(off) => {
                write!(
                    f,
                    "Android-packed relocations: malformed SLEB128 at offset {off}"
                )
            }
            Self::BadAndroidReloc(declared, produced) => {
                write!(
                    f,
                    "Android-packed relocations: declared {declared} relocs, decoded {produced}"
                )
            }
            Self::BadRelocationType(raw) => write!(
                f,
                "relocation type {raw} is outside the 16-bit numbering of every supported psABI"
            ),
        }
    }
}

impl std::error::Error for ElfError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoadSegment {
    pub file_offset: u64,

    pub vaddr: u64,

    pub file_size: u64,

    pub mem_size: u64,

    pub flags: u32,

    pub align: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TlsSegment {
    pub vaddr: u64,

    pub file_size: u64,

    pub mem_size: u64,

    pub align: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RelroSegment {
    pub vaddr: u64,

    pub mem_size: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DynInfo {
    pub rela: Option<(u64, u64)>,

    pub jmprel: Option<(u64, u64)>,

    pub pltrel: Option<u64>,

    pub relr: Option<(u64, u64)>,

    pub android_rela: Option<(u64, u64)>,

    pub android_relr: Option<(u64, u64)>,

    pub symtab: Option<u64>,

    pub strtab: Option<(u64, u64)>,

    pub hash: Option<u64>,

    pub gnu_hash: Option<u64>,

    pub soname_off: Option<u64>,

    pub needed_offs: Vec<u64>,

    pub init: Option<u64>,

    pub init_array: Option<(u64, u64)>,

    pub fini_array: Option<(u64, u64)>,

    pub flags: Option<u64>,

    pub flags_1: Option<u64>,

    pub bind_now_tag: bool,
}

impl DynInfo {
    pub fn bind_now(&self) -> bool {
        self.bind_now_tag
            || self.flags.is_some_and(|f| f & DF_BIND_NOW != 0)
            || self.flags_1.is_some_and(|f| f & DF_1_NOW != 0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DynSym {
    pub name: String,

    pub value: u64,

    pub size: u64,

    pub bind: u8,

    pub sym_type: u8,

    pub shndx: u16,
}

#[derive(Debug, Clone)]
pub struct ElfImage<'a> {
    bytes: &'a [u8],

    pub abi: Abi,

    pub loads: Vec<LoadSegment>,

    pub dynamic: Option<(u64, u64)>,

    pub tls: Option<TlsSegment>,

    pub relro: Option<RelroSegment>,

    pub dyn_info: DynInfo,

    pub dynsyms: Vec<DynSym>,
}

fn read_u16(bytes: &[u8], off: usize) -> Result<u16, ElfError> {
    let end = off.checked_add(2).ok_or(ElfError::Truncated {
        offset: off,
        need: 2,
    })?;
    let s = bytes.get(off..end).ok_or(ElfError::Truncated {
        offset: off,
        need: 2,
    })?;
    Ok(u16::from_le_bytes([s[0], s[1]]))
}

fn read_u32(bytes: &[u8], off: usize) -> Result<u32, ElfError> {
    let end = off.checked_add(4).ok_or(ElfError::Truncated {
        offset: off,
        need: 4,
    })?;
    let s = bytes.get(off..end).ok_or(ElfError::Truncated {
        offset: off,
        need: 4,
    })?;
    Ok(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

fn read_u64(bytes: &[u8], off: usize) -> Result<u64, ElfError> {
    let end = off.checked_add(8).ok_or(ElfError::Truncated {
        offset: off,
        need: 8,
    })?;
    let s = bytes.get(off..end).ok_or(ElfError::Truncated {
        offset: off,
        need: 8,
    })?;
    Ok(u64::from_le_bytes(s.try_into().expect("8-byte slice")))
}

fn read_sleb128(bytes: &[u8], cursor: &mut usize) -> Result<i64, ElfError> {
    let start = *cursor;
    let mut result: i64 = 0;
    let mut shift: u32 = 0;
    loop {
        let byte = *bytes.get(*cursor).ok_or(ElfError::Truncated {
            offset: *cursor,
            need: 1,
        })?;
        *cursor += 1;

        if shift >= 64 {
            return Err(ElfError::BadSleb128(start));
        }
        result |= i64::from(byte & 0x7f) << shift;
        shift += 7;
        if byte & 0x80 == 0 {
            if shift < 64 && byte & 0x40 != 0 {
                result |= -1i64 << shift;
            }
            return Ok(result);
        }
    }
}

impl<'a> ElfImage<'a> {
    pub fn parse(bytes: &'a [u8]) -> Result<Self, ElfError> {
        let abi = Self::parse_header(bytes)?;

        let e_phoff = read_u64(bytes, 32)? as usize;
        let e_phentsize = read_u16(bytes, 54)?;
        let e_phnum = read_u16(bytes, 56)? as usize;
        if e_phentsize as usize != PHDR_SIZE {
            return Err(ElfError::BadPhEntSize(e_phentsize));
        }

        let mut loads = Vec::new();
        let mut dynamic = None;
        let mut tls = None;
        let mut relro = None;

        for i in 0..e_phnum {
            let ph = e_phoff
                .checked_add(i.checked_mul(PHDR_SIZE).ok_or(ElfError::Truncated {
                    offset: e_phoff,
                    need: PHDR_SIZE,
                })?)
                .ok_or(ElfError::Truncated {
                    offset: e_phoff,
                    need: PHDR_SIZE,
                })?;

            let p_type = read_u32(bytes, ph)?;
            let p_flags = read_u32(bytes, ph + 4)?;
            let p_offset = read_u64(bytes, ph + 8)?;
            let p_vaddr = read_u64(bytes, ph + 16)?;
            let p_filesz = read_u64(bytes, ph + 32)?;
            let p_memsz = read_u64(bytes, ph + 40)?;
            let p_align = read_u64(bytes, ph + 48)?;

            match p_type {
                PT_LOAD => loads.push(LoadSegment {
                    file_offset: p_offset,
                    vaddr: p_vaddr,
                    file_size: p_filesz,
                    mem_size: p_memsz,
                    flags: p_flags,
                    align: p_align,
                }),
                PT_DYNAMIC => dynamic = Some((p_vaddr, p_filesz)),
                PT_TLS => {
                    tls = Some(TlsSegment {
                        vaddr: p_vaddr,
                        file_size: p_filesz,
                        mem_size: p_memsz,
                        align: p_align,
                    })
                }
                PT_GNU_RELRO => {
                    relro = Some(RelroSegment {
                        vaddr: p_vaddr,
                        mem_size: p_memsz,
                    })
                }
                _ => {}
            }
        }

        let mut img = ElfImage {
            bytes,
            abi,
            loads,
            dynamic,
            tls,
            relro,
            dyn_info: DynInfo::default(),
            dynsyms: Vec::new(),
        };

        if let Some((dyn_vaddr, dyn_size)) = img.dynamic {
            img.dyn_info = img.parse_dynamic(dyn_vaddr, dyn_size)?;
            img.dynsyms = img.parse_dynsyms()?;
        }

        Ok(img)
    }

    fn parse_header(bytes: &[u8]) -> Result<Abi, ElfError> {
        if bytes.len() < EHDR_SIZE {
            return Err(ElfError::Truncated {
                offset: 0,
                need: EHDR_SIZE,
            });
        }
        if bytes[0..4] != ELF_MAGIC {
            return Err(ElfError::BadMagic);
        }
        if bytes[EI_CLASS] != ELFCLASS64 {
            return Err(ElfError::NotElf64(bytes[EI_CLASS]));
        }
        if bytes[EI_DATA] != ELFDATA2LSB {
            return Err(ElfError::NotLittleEndian(bytes[EI_DATA]));
        }
        let e_type = read_u16(bytes, 16)?;
        if e_type != ET_DYN {
            return Err(ElfError::NotSharedObject(e_type));
        }
        let e_machine = read_u16(bytes, 18)?;
        Abi::from_elf_machine(e_machine).ok_or(ElfError::UnsupportedMachine(e_machine))
    }

    pub fn vaddr_to_off(&self, vaddr: u64) -> Result<usize, ElfError> {
        for seg in &self.loads {
            let end = seg.vaddr.saturating_add(seg.file_size);
            if vaddr >= seg.vaddr && vaddr < end {
                let delta = vaddr - seg.vaddr;
                let off = seg
                    .file_offset
                    .checked_add(delta)
                    .ok_or(ElfError::UnmappedVaddr(vaddr))?;
                return usize::try_from(off).map_err(|_| ElfError::UnmappedVaddr(vaddr));
            }
        }
        Err(ElfError::UnmappedVaddr(vaddr))
    }

    fn parse_dynamic(&self, dyn_vaddr: u64, dyn_size: u64) -> Result<DynInfo, ElfError> {
        let base = self.vaddr_to_off(dyn_vaddr)?;
        let count = (dyn_size as usize) / DYN_SIZE;
        let mut info = DynInfo::default();

        for i in 0..count {
            let off = base + i * DYN_SIZE;
            let tag = read_u64(self.bytes, off)? as i64;
            let val = read_u64(self.bytes, off + 8)?;
            match tag {
                DT_NULL => break,
                DT_RELA => info.rela.get_or_insert((0, 0)).0 = val,
                DT_RELASZ => info.rela.get_or_insert((0, 0)).1 = val,

                DT_RELAENT if val != reloc_ent_size() => {
                    return Err(ElfError::BadEntSize(tag, val, reloc_ent_size()));
                }
                DT_RELRENT if val != 8 => return Err(ElfError::BadEntSize(tag, val, 8)),
                DT_ANDROID_RELRENT if val != 8 => return Err(ElfError::BadEntSize(tag, val, 8)),
                DT_SYMENT if val != SYM_SIZE as u64 => {
                    return Err(ElfError::BadEntSize(tag, val, SYM_SIZE as u64));
                }
                DT_JMPREL => info.jmprel.get_or_insert((0, 0)).0 = val,
                DT_PLTRELSZ => info.jmprel.get_or_insert((0, 0)).1 = val,
                DT_PLTREL => info.pltrel = Some(val),
                DT_RELR => info.relr.get_or_insert((0, 0)).0 = val,
                DT_RELRSZ => info.relr.get_or_insert((0, 0)).1 = val,
                DT_ANDROID_RELA => info.android_rela.get_or_insert((0, 0)).0 = val,
                DT_ANDROID_RELASZ => info.android_rela.get_or_insert((0, 0)).1 = val,
                DT_ANDROID_RELR => info.android_relr.get_or_insert((0, 0)).0 = val,
                DT_ANDROID_RELRSZ => info.android_relr.get_or_insert((0, 0)).1 = val,
                DT_SYMTAB => info.symtab = Some(val),
                DT_STRTAB => info.strtab.get_or_insert((0, 0)).0 = val,
                DT_STRSZ => info.strtab.get_or_insert((0, 0)).1 = val,
                DT_HASH => info.hash = Some(val),
                DT_GNU_HASH => info.gnu_hash = Some(val),
                DT_SONAME => info.soname_off = Some(val),
                DT_NEEDED => info.needed_offs.push(val),
                DT_INIT => info.init = Some(val),
                DT_INIT_ARRAY => info.init_array.get_or_insert((0, 0)).0 = val,
                DT_INIT_ARRAYSZ => info.init_array.get_or_insert((0, 0)).1 = val,
                DT_FINI_ARRAY => info.fini_array.get_or_insert((0, 0)).0 = val,
                DT_FINI_ARRAYSZ => info.fini_array.get_or_insert((0, 0)).1 = val,
                DT_FLAGS => info.flags = Some(val),
                DT_FLAGS_1 => info.flags_1 = Some(val),
                DT_BIND_NOW => info.bind_now_tag = true,
                _ => {}
            }
        }
        Ok(info)
    }

    fn str_at(&self, str_off: u64) -> Result<String, ElfError> {
        let (str_vaddr, str_sz) = self
            .dyn_info
            .strtab
            .ok_or(ElfError::MissingDynamic("DT_STRTAB for a string lookup"))?;
        let base = self.vaddr_to_off(str_vaddr)?;
        let idx = usize::try_from(str_off).map_err(|_| ElfError::UnmappedVaddr(str_off))?;
        let limit = usize::try_from(str_sz).unwrap_or(usize::MAX);
        let start = base.checked_add(idx).ok_or(ElfError::Truncated {
            offset: base,
            need: idx,
        })?;

        let table_end = base.saturating_add(limit).min(self.bytes.len());
        let region = self
            .bytes
            .get(start..table_end)
            .ok_or(ElfError::Truncated {
                offset: start,
                need: 1,
            })?;
        let nul = region
            .iter()
            .position(|&b| b == 0)
            .ok_or(ElfError::Truncated {
                offset: start,
                need: 1,
            })?;
        Ok(String::from_utf8_lossy(&region[..nul]).into_owned())
    }

    pub fn soname(&self) -> Result<Option<String>, ElfError> {
        match self.dyn_info.soname_off {
            Some(off) => Ok(Some(self.str_at(off)?)),
            None => Ok(None),
        }
    }

    pub fn needed(&self) -> Result<Vec<String>, ElfError> {
        self.dyn_info
            .needed_offs
            .iter()
            .map(|&off| self.str_at(off))
            .collect()
    }

    fn parse_dynsyms(&self) -> Result<Vec<DynSym>, ElfError> {
        let Some(sym_vaddr) = self.dyn_info.symtab else {
            if self.dyn_info.rela.is_some() || self.dyn_info.jmprel.is_some() {
                return Err(ElfError::MissingDynamic("DT_SYMTAB"));
            }
            return Ok(Vec::new());
        };
        let base = self.vaddr_to_off(sym_vaddr)?;
        let count = self.dynsym_count()?;
        let table_len = count.checked_mul(SYM_SIZE).ok_or(ElfError::Truncated {
            offset: base,
            need: usize::MAX,
        })?;
        if base
            .checked_add(table_len)
            .is_none_or(|end| end > self.bytes.len())
        {
            return Err(ElfError::Truncated {
                offset: base,
                need: table_len,
            });
        }

        let mut syms = Vec::with_capacity(count);
        for index in 0..count {
            let off = base + index * SYM_SIZE;
            let st_name = read_u32(self.bytes, off)?;
            let st_info = self.bytes[off + 4];
            let st_shndx = read_u16(self.bytes, off + 6)?;
            let st_value = read_u64(self.bytes, off + 8)?;
            let st_size = read_u64(self.bytes, off + 16)?;
            let name = self.str_at(st_name as u64)?;
            syms.push(DynSym {
                name,
                value: st_value,
                size: st_size,
                bind: st_info >> 4,
                sym_type: st_info & 0xf,
                shndx: st_shndx,
            });
        }
        Ok(syms)
    }

    fn dynsym_count(&self) -> Result<usize, ElfError> {
        if let Some(hash) = self.dyn_info.hash {
            let off = self.vaddr_to_off(hash)?;
            let nchain = read_u32(self.bytes, offset_after(off, 4)?)?;
            return Ok(nchain as usize);
        }
        let gnu_hash = self
            .dyn_info
            .gnu_hash
            .ok_or(ElfError::MissingDynamic("DT_HASH or DT_GNU_HASH"))?;
        let off = self.vaddr_to_off(gnu_hash)?;
        let nbuckets = read_u32(self.bytes, off)? as usize;
        let symoffset = read_u32(self.bytes, offset_after(off, 4)?)?;
        let bloom_size = read_u32(self.bytes, offset_after(off, 8)?)? as usize;
        let buckets = offset_after(
            off,
            bloom_size
                .checked_mul(GNU_HASH_BLOOM_WORD_SIZE)
                .and_then(|bloom| bloom.checked_add(16))
                .ok_or(ElfError::Truncated {
                    offset: off,
                    need: usize::MAX,
                })?,
        )?;
        let chains = offset_after(
            buckets,
            nbuckets.checked_mul(4).ok_or(ElfError::Truncated {
                offset: buckets,
                need: usize::MAX,
            })?,
        )?;

        let mut last = 0u32;
        for bucket in 0..nbuckets {
            last = last.max(read_u32(self.bytes, offset_after(buckets, bucket * 4)?)?);
        }
        if last < symoffset {
            return Ok(symoffset as usize);
        }
        loop {
            let chain_index = (last - symoffset) as usize;
            let entry = read_u32(self.bytes, offset_after(chains, chain_index * 4)?)?;
            last = last.checked_add(1).ok_or(ElfError::Truncated {
                offset: chains,
                need: usize::MAX,
            })?;
            if entry & 1 != 0 {
                return Ok(last as usize);
            }
        }
    }

    pub fn relocations(&self) -> Result<Vec<Rela>, ElfError> {
        let mut out = Vec::new();
        if let Some((vaddr, size)) = self.dyn_info.rela {
            self.read_rela_table(vaddr, size, &mut out)?;
        }
        if let Some((vaddr, size)) = self.dyn_info.android_rela {
            self.decode_android_packed_rela(vaddr, size, &mut out)?;
        }
        if let Some((vaddr, size)) = self.dyn_info.jmprel {
            self.read_rela_table(vaddr, size, &mut out)?;
        }
        Ok(out)
    }

    fn decode_android_packed_rela(
        &self,
        vaddr: u64,
        size: u64,
        out: &mut Vec<Rela>,
    ) -> Result<usize, ElfError> {
        let base = self.vaddr_to_off(vaddr)?;

        let end = base
            .saturating_add(usize::try_from(size).unwrap_or(usize::MAX))
            .min(self.bytes.len());
        let section = self.bytes.get(base..end).ok_or(ElfError::Truncated {
            offset: base,
            need: usize::try_from(size).unwrap_or(usize::MAX),
        })?;

        if section.get(0..4) != Some(&APS2_MAGIC[..]) {
            return Err(ElfError::BadAndroidMagic(base));
        }

        let mut cur = 4usize;
        let reloc_count = read_sleb128(section, &mut cur)?;
        let reloc_count =
            u64::try_from(reloc_count).map_err(|_| ElfError::BadAndroidReloc(0, 0))?;

        let mut offset = read_sleb128(section, &mut cur)?;
        let mut addend: i64 = 0;
        let mut produced: u64 = 0;

        while produced < reloc_count {
            let group_size = read_sleb128(section, &mut cur)?;
            let group_size = u64::try_from(group_size)
                .map_err(|_| ElfError::BadAndroidReloc(reloc_count, produced))?;
            let group_flags = read_sleb128(section, &mut cur)?;

            let grouped_by_offset = group_flags & RELOCATION_GROUPED_BY_OFFSET_DELTA_FLAG != 0;
            let grouped_by_info = group_flags & RELOCATION_GROUPED_BY_INFO_FLAG != 0;
            let grouped_by_addend = group_flags & RELOCATION_GROUPED_BY_ADDEND_FLAG != 0;
            let group_has_addend = group_flags & RELOCATION_GROUP_HAS_ADDEND_FLAG != 0;

            if produced
                .checked_add(group_size)
                .is_none_or(|t| t > reloc_count)
            {
                return Err(ElfError::BadAndroidReloc(reloc_count, produced));
            }

            let group_offset_delta = if grouped_by_offset {
                read_sleb128(section, &mut cur)?
            } else {
                0
            };
            let group_info = if grouped_by_info {
                Some(read_sleb128(section, &mut cur)?)
            } else {
                None
            };
            if group_has_addend && grouped_by_addend {
                addend = addend.wrapping_add(read_sleb128(section, &mut cur)?);
            }

            for _ in 0..group_size {
                offset = if grouped_by_offset {
                    offset.wrapping_add(group_offset_delta)
                } else {
                    offset.wrapping_add(read_sleb128(section, &mut cur)?)
                };
                let info = match group_info {
                    Some(i) => i,
                    None => read_sleb128(section, &mut cur)?,
                };
                if group_has_addend {
                    if !grouped_by_addend {
                        addend = addend.wrapping_add(read_sleb128(section, &mut cur)?);
                    }
                } else {
                    addend = 0;
                }
                let info = info as u64;
                out.push(Rela {
                    offset: offset as u64,
                    sym_index: rela_sym(info),
                    kind: self.reloc_kind(info)?,
                    addend,
                });
            }
            produced += group_size;
        }
        Ok(cur)
    }

    fn read_rela_table(&self, vaddr: u64, size: u64, out: &mut Vec<Rela>) -> Result<(), ElfError> {
        let base = self.vaddr_to_off(vaddr)?;
        let count = (size as usize) / reloc_ent_size() as usize;
        for i in 0..count {
            let off = base + i * reloc_ent_size() as usize;
            let r_offset = read_u64(self.bytes, off)?;
            let r_info = read_u64(self.bytes, off + 8)?;
            let r_addend = read_u64(self.bytes, off + 16)? as i64;
            out.push(Rela {
                offset: r_offset,
                sym_index: rela_sym(r_info),
                kind: self.reloc_kind(r_info)?,
                addend: r_addend,
            });
        }
        Ok(())
    }

    fn reloc_kind(&self, info: u64) -> Result<RelocKind, ElfError> {
        let raw = rela_type(info);
        RelocKind::decode(self.abi, raw).ok_or(ElfError::BadRelocationType(raw))
    }

    pub fn relr(&self) -> Result<Vec<u64>, ElfError> {
        let Some((vaddr, size)) = self.dyn_info.relr.or(self.dyn_info.android_relr) else {
            return Ok(Vec::new());
        };
        let base = self.vaddr_to_off(vaddr)?;
        let count = (size as usize) / 8;
        let mut out = Vec::with_capacity(count);
        for i in 0..count {
            out.push(read_u64(self.bytes, base + i * 8)?);
        }
        Ok(out)
    }
}

fn reloc_ent_size() -> u64 {
    24
}

fn offset_after(base: usize, delta: usize) -> Result<usize, ElfError> {
    base.checked_add(delta).ok_or(ElfError::Truncated {
        offset: base,
        need: delta,
    })
}
