use std::ffi::c_void;
use std::ptr::NonNull;
use std::time::{Duration, Instant};

use rustix::mm::{
    Advice, MapFlags, MprotectFlags, ProtFlags, madvise, mmap_anonymous, mprotect, munmap,
};

use super::elf::{ElfError, ElfImage, LoadSegment, PF_R, PF_W, PF_X, RelroSegment};
use super::reloc::{self, RelocError, RelocImage, RelocKind, SymbolResolver};
use super::resolve::{Scope, ScopedResolver};
use super::tls::{TlsLayout, TlsResolver};
use crate::abi::Abi;

#[cfg(not(target_arch = "x86_64"))]
compile_error!("the experimental Rust linker currently supports x86_64 hosts only");

// x86-64 has a coherent instruction/data cache, so publishing executable
// bytes needs no cache maintenance. Keep the call site shared with the ELF
// mapper, where instruction-cache invalidation is required on other hosts.
unsafe fn eclipse_sync_icache(_begin: *mut c_void, _end: *mut c_void) {}

#[derive(Debug)]
pub enum MapError {
    NoLoadSegments,

    SpanOverflow(&'static str),

    SegmentOutOfFile(u64),

    FileSizeExceedsMemSize(u64, u64),

    Os(rustix::io::Errno),

    Reloc(RelocError),

    UnsupportedRelocation {
        abi: Abi,
        kind: RelocKind,
        offset: u64,
    },

    SegmentsSharePage {
        page_size: u64,
        first: u64,
        second: u64,
    },

    RelocationDecode {
        table: &'static str,
        error: ElfError,
    },
}

impl std::fmt::Display for MapError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoLoadSegments => write!(f, "ELF image has no PT_LOAD segments to map"),
            Self::SpanOverflow(what) => write!(f, "PT_LOAD layout arithmetic overflowed: {what}"),
            Self::SegmentOutOfFile(off) => {
                write!(
                    f,
                    "PT_LOAD at file offset {off:#x} extends past the file bytes"
                )
            }
            Self::FileSizeExceedsMemSize(fz, mz) => {
                write!(f, "PT_LOAD p_filesz {fz:#x} exceeds p_memsz {mz:#x}")
            }
            Self::Os(e) => write!(f, "memory-mapping syscall failed: {e}"),
            Self::Reloc(e) => write!(f, "base relocation failed: {e}"),
            Self::UnsupportedRelocation { abi, kind, offset } => write!(
                f,
                "{} relocation {kind} at {offset:#x} is not supported by the engine loader",
                abi.label()
            ),
            Self::SegmentsSharePage {
                page_size,
                first,
                second,
            } => write!(
                f,
                "the PT_LOAD segments at {first:#x} and {second:#x} have different permissions \
                 but share one {} KiB page of this kernel",
                page_size / 1024
            ),
            Self::RelocationDecode { table, error } => {
                write!(f, "decoding the {table} relocation table failed: {error}")
            }
        }
    }
}

impl std::error::Error for MapError {}

impl From<RelocError> for MapError {
    fn from(e: RelocError) -> Self {
        Self::Reloc(e)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MapStats {
    pub segments_mapped: usize,

    pub relative_applied: usize,

    pub relr_applied: usize,

    pub skipped_by_type: usize,

    pub icache_sync: Duration,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SymbolRelocStats {
    pub glob_dat_applied: usize,

    pub jump_slot_applied: usize,

    pub abs64_applied: usize,

    pub resolved_nonnull: usize,

    pub deferred: usize,
}

impl SymbolRelocStats {
    pub fn total_applied(&self) -> usize {
        self.glob_dat_applied + self.jump_slot_applied + self.abs64_applied
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TlsRelocStats {
    pub tpoff64_applied: usize,

    pub deferred: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PartialSymbolStats {
    pub applied_nonnull: usize,

    pub applied_weak_zero: usize,

    pub unresolved_strong: usize,

    pub unresolved: Vec<String>,
}

fn page_floor(addr: u64, page: u64) -> u64 {
    addr & !(page - 1)
}

fn page_ceil(addr: u64, page: u64) -> Option<u64> {
    addr.checked_add(page - 1).map(|a| a & !(page - 1))
}

fn page_span(seg: &LoadSegment, page: u64) -> Result<(u64, u64), MapError> {
    let end = seg
        .vaddr
        .checked_add(seg.mem_size)
        .ok_or(MapError::SpanOverflow("vaddr + memsz"))?;
    let end = page_ceil(end, page).ok_or(MapError::SpanOverflow("segment end ceil"))?;
    Ok((page_floor(seg.vaddr, page), end))
}

fn reject_shared_pages(loads: &[LoadSegment], page: u64) -> Result<(), MapError> {
    for (index, first) in loads.iter().enumerate() {
        let (first_start, first_end) = page_span(first, page)?;
        for second in &loads[index + 1..] {
            let (second_start, second_end) = page_span(second, page)?;
            if first.flags != second.flags && first_start < second_end && second_start < first_end {
                return Err(MapError::SegmentsSharePage {
                    page_size: page,
                    first: first.vaddr,
                    second: second.vaddr,
                });
            }
        }
    }
    Ok(())
}

fn prot_of(flags: u32) -> ProtFlags {
    let mut p = ProtFlags::empty();
    if flags & PF_R != 0 {
        p |= ProtFlags::READ;
    }
    if flags & PF_W != 0 {
        p |= ProtFlags::WRITE;
    }
    if flags & PF_X != 0 {
        p |= ProtFlags::EXEC;
    }
    p
}

pub struct MappedObject {
    base: NonNull<u8>,

    span: usize,

    static_tls_offset: i64,
}

unsafe impl Send for MappedObject {}
unsafe impl Sync for MappedObject {}

impl MappedObject {
    pub fn map_and_relocate(
        img: &ElfImage<'_>,
        file: &[u8],
        page_size: u64,
    ) -> Result<(Self, MapStats), MapError> {
        if img.loads.is_empty() {
            return Err(MapError::NoLoadSegments);
        }

        let mut max_end: u64 = 0;
        for s in &img.loads {
            if s.file_size > s.mem_size {
                return Err(MapError::FileSizeExceedsMemSize(s.file_size, s.mem_size));
            }
            let end = s
                .vaddr
                .checked_add(s.mem_size)
                .ok_or(MapError::SpanOverflow("vaddr + memsz"))?;
            max_end = max_end.max(end);
        }
        reject_shared_pages(&img.loads, page_size)?;
        let span_u64 = page_ceil(max_end, page_size).ok_or(MapError::SpanOverflow("memsz ceil"))?;
        let span =
            usize::try_from(span_u64).map_err(|_| MapError::SpanOverflow("span as usize"))?;
        if span == 0 {
            return Err(MapError::NoLoadSegments);
        }

        let ptr = unsafe {
            mmap_anonymous(
                std::ptr::null_mut(),
                span,
                ProtFlags::empty(),
                MapFlags::PRIVATE,
            )
        }
        .map_err(MapError::Os)?;

        let base = NonNull::new(ptr.cast::<u8>()).ok_or(MapError::Os(rustix::io::Errno::NOMEM))?;

        const HUGE_PAGE_THRESHOLD: usize = 2 * 1024 * 1024;
        if span >= HUGE_PAGE_THRESHOLD {
            let _ = unsafe { madvise(ptr, span, Advice::LinuxHugepage) };
        }

        let load_base = base.as_ptr() as u64;

        let mut obj = MappedObject {
            base,
            span,
            static_tls_offset: 0,
        };

        for seg in &img.loads {
            obj.populate_segment(file, seg, page_size)?;
        }

        let relas = img
            .relocations()
            .map_err(|error| MapError::RelocationDecode {
                table: "RELA",
                error,
            })?;
        let relr = img.relr().map_err(|error| MapError::RelocationDecode {
            table: "RELR",
            error,
        })?;

        let mut stats = MapStats {
            segments_mapped: img.loads.len(),
            ..Default::default()
        };

        let mut relative: Vec<reloc::Rela> = Vec::new();
        for r in &relas {
            if r.kind == RelocKind::Relative {
                relative.push(*r);
            } else {
                stats.skipped_by_type += 1;
            }
        }
        stats.relative_applied = relative.len();

        stats.relr_applied = count_relr_targets(&relr);

        let relr_runtime: Vec<u64> = relr
            .iter()
            .map(|&w| {
                if w & 1 == 0 {
                    w.wrapping_add(load_base)
                } else {
                    w
                }
            })
            .collect();

        {
            let base_addr = load_base;
            let tls_off = obj.static_tls_offset;

            let bytes = unsafe { obj.image_bytes() };
            let mut image = reloc::SliceImage::new(img.abi, base_addr, tls_off, bytes);
            reloc::apply_rela(&mut image, &NullResolver, &relative)?;
            reloc::apply_relr(&mut image, &relr_runtime)?;
        }

        let icache_start = Instant::now();
        for seg in img.loads.iter().filter(|seg| seg.flags & PF_X != 0) {
            obj.sync_instruction_cache(seg)?;
        }
        stats.icache_sync = icache_start.elapsed();

        for seg in &img.loads {
            obj.protect_segment(seg, page_size)?;
        }

        Ok((obj, stats))
    }

    pub fn relocate_symbols(
        &mut self,
        img: &ElfImage<'_>,
        scope: &Scope,
        page_size: u64,
    ) -> Result<SymbolRelocStats, MapError> {
        let relas = img
            .relocations()
            .map_err(|error| MapError::RelocationDecode {
                table: "RELA",
                error,
            })?;
        let mut symbol_relas: Vec<reloc::Rela> = Vec::new();
        let mut stats = SymbolRelocStats::default();
        for r in &relas {
            match r.kind {
                RelocKind::GlobDat => {
                    stats.glob_dat_applied += 1;
                    symbol_relas.push(*r);
                }
                RelocKind::JumpSlot => {
                    stats.jump_slot_applied += 1;
                    symbol_relas.push(*r);
                }
                RelocKind::Abs64 => {
                    stats.abs64_applied += 1;
                    symbol_relas.push(*r);
                }
                RelocKind::TpOff64 | RelocKind::IRelative => stats.deferred += 1,

                RelocKind::None | RelocKind::Relative => {}

                kind @ RelocKind::Unsupported(_) => {
                    return Err(MapError::UnsupportedRelocation {
                        abi: img.abi,
                        kind,
                        offset: r.offset,
                    });
                }
            }
        }

        let resolver = ScopedResolver::new(scope, &img.dynsyms);
        let load_base = self.load_base();
        let tls_off = self.static_tls_offset;

        for seg in &img.loads {
            self.mprotect_segment_pages(seg, page_size, ProtFlags::READ | ProtFlags::WRITE)?;
        }

        let resolved_nonnull = {
            let bytes = unsafe { self.image_bytes() };
            let mut image = reloc::SliceImage::new(img.abi, load_base, tls_off, bytes);
            reloc::apply_rela(&mut image, &resolver, &symbol_relas)?;

            let mut nonnull = 0usize;
            for r in &symbol_relas {
                let off = usize::try_from(r.offset)
                    .map_err(|_| MapError::SpanOverflow("reloc offset as usize"))?;
                if image.read_u64(off)? != 0 {
                    nonnull += 1;
                }
            }
            nonnull
        };
        stats.resolved_nonnull = resolved_nonnull;

        self.restore_protection(img, page_size)?;

        Ok(stats)
    }

    pub fn relocate_symbols_partial(
        &mut self,
        img: &ElfImage<'_>,
        scope: &Scope,
        page_size: u64,
    ) -> Result<PartialSymbolStats, MapError> {
        let relas = img
            .relocations()
            .map_err(|error| MapError::RelocationDecode {
                table: "RELA",
                error,
            })?;

        let resolver = ScopedResolver::new(scope, &img.dynsyms);
        let mut to_apply: Vec<reloc::Rela> = Vec::new();
        let mut stats = PartialSymbolStats::default();
        let mut unresolved_names: std::collections::BTreeSet<String> =
            std::collections::BTreeSet::new();
        for r in &relas {
            match r.kind {
                RelocKind::GlobDat | RelocKind::JumpSlot | RelocKind::Abs64 => {
                    match resolver.resolve_symbol(r.sym_index) {
                        Some(0) => {
                            stats.applied_weak_zero += 1;
                            to_apply.push(*r);
                        }
                        Some(_) => {
                            stats.applied_nonnull += 1;
                            to_apply.push(*r);
                        }
                        None => {
                            stats.unresolved_strong += 1;
                            let name = img
                                .dynsyms
                                .get(r.sym_index as usize)
                                .map(|s| s.name.clone())
                                .unwrap_or_default();
                            if !name.is_empty() {
                                unresolved_names.insert(name);
                            }
                        }
                    }
                }
                RelocKind::None | RelocKind::Relative => {}
                kind @ (RelocKind::TpOff64 | RelocKind::IRelative | RelocKind::Unsupported(_)) => {
                    return Err(MapError::UnsupportedRelocation {
                        abi: img.abi,
                        kind,
                        offset: r.offset,
                    });
                }
            }
        }
        stats.unresolved = unresolved_names.into_iter().collect();

        let load_base = self.load_base();
        let tls_off = self.static_tls_offset;

        for seg in &img.loads {
            self.mprotect_segment_pages(seg, page_size, ProtFlags::READ | ProtFlags::WRITE)?;
        }
        {
            let bytes = unsafe { self.image_bytes() };
            let mut image = reloc::SliceImage::new(img.abi, load_base, tls_off, bytes);
            for r in &to_apply {
                reloc::apply_one(&mut image, &resolver, r)?;
            }
        }
        self.restore_protection(img, page_size)?;

        Ok(stats)
    }

    pub fn relocate_tls<R: reloc::SymbolResolver>(
        &mut self,
        img: &ElfImage<'_>,
        inner: &R,
        layout: &TlsLayout,
        own_tp_offset: Option<i64>,
        page_size: u64,
    ) -> Result<TlsRelocStats, MapError> {
        let relas = img
            .relocations()
            .map_err(|error| MapError::RelocationDecode {
                table: "RELA",
                error,
            })?;
        let mut tls_relas: Vec<reloc::Rela> = Vec::new();
        let mut stats = TlsRelocStats::default();
        for r in &relas {
            match r.kind {
                RelocKind::TpOff64 => {
                    stats.tpoff64_applied += 1;
                    tls_relas.push(*r);
                }
                RelocKind::IRelative => stats.deferred += 1,

                RelocKind::None
                | RelocKind::Relative
                | RelocKind::GlobDat
                | RelocKind::JumpSlot
                | RelocKind::Abs64 => {}

                kind @ RelocKind::Unsupported(_) => {
                    return Err(MapError::UnsupportedRelocation {
                        abi: img.abi,
                        kind,
                        offset: r.offset,
                    });
                }
            }
        }

        if tls_relas.is_empty() {
            return Ok(stats);
        }

        let resolver = TlsResolver::new(inner, &img.dynsyms, layout, own_tp_offset);
        let load_base = self.load_base();

        for seg in &img.loads {
            self.mprotect_segment_pages(seg, page_size, ProtFlags::READ | ProtFlags::WRITE)?;
        }

        {
            let bytes = unsafe { self.image_bytes() };
            let mut image = reloc::SliceImage::new(img.abi, load_base, 0, bytes);
            reloc::apply_rela(&mut image, &resolver, &tls_relas)?;
        }

        self.restore_protection(img, page_size)?;

        Ok(stats)
    }

    fn populate_segment(
        &mut self,
        file: &[u8],
        seg: &LoadSegment,
        page_size: u64,
    ) -> Result<(), MapError> {
        let seg_off = usize::try_from(seg.vaddr)
            .map_err(|_| MapError::SpanOverflow("segment offset as usize"))?;

        self.mprotect_segment_pages(seg, page_size, ProtFlags::READ | ProtFlags::WRITE)?;

        let filesz = usize::try_from(seg.file_size)
            .map_err(|_| MapError::SpanOverflow("filesz as usize"))?;
        if filesz == 0 {
            return Ok(());
        }
        let file_off = usize::try_from(seg.file_offset)
            .map_err(|_| MapError::SpanOverflow("file_offset as usize"))?;
        let file_end = file_off
            .checked_add(filesz)
            .ok_or(MapError::SpanOverflow("file_offset + filesz"))?;
        let src = file
            .get(file_off..file_end)
            .ok_or(MapError::SegmentOutOfFile(seg.file_offset))?;

        let dst = unsafe {
            let end = seg_off
                .checked_add(filesz)
                .ok_or(MapError::SpanOverflow("seg_off + filesz"))?;
            if end > self.span {
                return Err(MapError::SpanOverflow("segment past span"));
            }
            std::slice::from_raw_parts_mut(self.base.as_ptr().add(seg_off), filesz)
        };
        dst.copy_from_slice(src);
        Ok(())
    }

    fn sync_instruction_cache(&self, seg: &LoadSegment) -> Result<(), MapError> {
        let start = usize::try_from(seg.vaddr)
            .map_err(|_| MapError::SpanOverflow("text offset as usize"))?;
        let end = usize::try_from(seg.file_size)
            .ok()
            .and_then(|len| start.checked_add(len))
            .filter(|&end| end <= self.span)
            .ok_or(MapError::SpanOverflow("text range past span"))?;
        unsafe {
            eclipse_sync_icache(
                self.base.as_ptr().add(start).cast(),
                self.base.as_ptr().add(end).cast(),
            );
        }
        Ok(())
    }

    fn restore_protection(&self, img: &ElfImage<'_>, page_size: u64) -> Result<(), MapError> {
        for seg in &img.loads {
            self.protect_segment(seg, page_size)?;
        }
        if let Some(relro) = img.relro {
            self.apply_relro(&relro, page_size)?;
        }
        Ok(())
    }

    fn protect_segment(&self, seg: &LoadSegment, page_size: u64) -> Result<(), MapError> {
        let prot = prot_of(seg.flags);
        self.mprotect_segment_pages(seg, page_size, prot)
    }

    fn mprotect_segment_pages(
        &self,
        seg: &LoadSegment,
        page_size: u64,
        prot: ProtFlags,
    ) -> Result<(), MapError> {
        let (seg_start, seg_end) = page_span(seg, page_size)?;

        if seg_end <= seg_start {
            return Ok(());
        }
        let off = usize::try_from(seg_start)
            .map_err(|_| MapError::SpanOverflow("protect offset as usize"))?;
        let len = usize::try_from(seg_end - seg_start)
            .map_err(|_| MapError::SpanOverflow("protect len as usize"))?;
        if off.checked_add(len).map(|e| e > self.span).unwrap_or(true) {
            return Err(MapError::SpanOverflow("protect range past span"));
        }

        unsafe {
            mprotect(
                self.base.as_ptr().add(off).cast(),
                len,
                mprotect_flags(prot),
            )
        }
        .map_err(MapError::Os)
    }

    pub fn apply_relro(&self, relro: &RelroSegment, page_size: u64) -> Result<(), MapError> {
        let prot_start = page_floor(relro.vaddr, page_size);
        let end_unaligned = relro
            .vaddr
            .checked_add(relro.mem_size)
            .ok_or(MapError::SpanOverflow("relro vaddr + memsz"))?;

        let prot_end = page_floor(end_unaligned, page_size);
        if prot_end <= prot_start {
            return Ok(());
        }
        let off = usize::try_from(prot_start)
            .map_err(|_| MapError::SpanOverflow("relro offset as usize"))?;
        let len = usize::try_from(prot_end - prot_start)
            .map_err(|_| MapError::SpanOverflow("relro len as usize"))?;
        if off.checked_add(len).map(|e| e > self.span).unwrap_or(true) {
            return Err(MapError::SpanOverflow("relro range past span"));
        }

        unsafe {
            mprotect(
                self.base.as_ptr().add(off).cast(),
                len,
                mprotect_flags(ProtFlags::READ),
            )
        }
        .map_err(MapError::Os)
    }

    pub fn load_base(&self) -> u64 {
        self.base.as_ptr() as u64
    }

    pub fn span(&self) -> usize {
        self.span
    }

    pub fn read_u64(&self, off: usize) -> Result<u64, MapError> {
        let end = off
            .checked_add(8)
            .ok_or(MapError::SpanOverflow("read offset + 8"))?;
        if end > self.span {
            return Err(MapError::SpanOverflow("read past span"));
        }

        let bytes = unsafe { std::slice::from_raw_parts(self.base.as_ptr().add(off), 8) };
        Ok(u64::from_le_bytes(bytes.try_into().expect("8-byte slice")))
    }

    pub(crate) unsafe fn image_bytes(&mut self) -> &mut [u8] {
        unsafe { std::slice::from_raw_parts_mut(self.base.as_ptr(), self.span) }
    }
}

impl Drop for MappedObject {
    fn drop(&mut self) {
        let _ = unsafe { munmap(self.base.as_ptr().cast(), self.span) };
    }
}

fn mprotect_flags(prot: ProtFlags) -> MprotectFlags {
    let mut m = MprotectFlags::empty();
    if prot.contains(ProtFlags::READ) {
        m |= MprotectFlags::READ;
    }
    if prot.contains(ProtFlags::WRITE) {
        m |= MprotectFlags::WRITE;
    }
    if prot.contains(ProtFlags::EXEC) {
        m |= MprotectFlags::EXEC;
    }
    m
}

struct NullResolver;
impl reloc::SymbolResolver for NullResolver {
    fn resolve_symbol(&self, _i: u32) -> Option<u64> {
        None
    }
    fn resolve_tls_offset(&self, _i: u32) -> Option<u64> {
        None
    }
}

fn count_relr_targets(entries: &[u64]) -> usize {
    let mut n = 0usize;
    for &entry in entries {
        if entry & 1 == 0 {
            n += 1;
        } else {
            n += (entry >> 1).count_ones() as usize;
        }
    }
    n
}

pub fn host_page_size() -> u64 {
    rustix::param::page_size() as u64
}
