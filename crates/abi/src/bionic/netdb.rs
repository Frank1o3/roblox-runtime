//! Bionic-compatible resolver exports for the Android engine on Linux hosts.
//!
//! Bionic and glibc disagree about `addrinfo`'s last two pointers, several
//! `AI_*` values, and `EAI_*` result numbers. The engine-facing list is copied
//! into bionic layout and allocated with libc so `freeaddrinfo` can release it.

use std::ffi::{c_char, c_int, c_void};
use std::ptr;

#[repr(C)]
struct BionicAddrinfo {
    ai_flags: c_int,
    ai_family: c_int,
    ai_socktype: c_int,
    ai_protocol: c_int,
    ai_addrlen: libc::socklen_t,
    ai_canonname: *mut c_char,
    ai_addr: *mut libc::sockaddr,
    ai_next: *mut BionicAddrinfo,
}

const B_AI_PASSIVE: c_int = 0x0001;
const B_AI_CANONNAME: c_int = 0x0002;
const B_AI_NUMERICHOST: c_int = 0x0004;
const B_AI_NUMERICSERV: c_int = 0x0008;
const B_AI_ALL: c_int = 0x0100;
const B_AI_V4MAPPED_CFG: c_int = 0x0200;
const B_AI_ADDRCONFIG: c_int = 0x0400;
const B_AI_V4MAPPED: c_int = 0x0800;

fn flags_to_host(flags: c_int) -> c_int {
    let mut host = 0;
    if flags & B_AI_PASSIVE != 0 {
        host |= libc::AI_PASSIVE;
    }
    if flags & B_AI_CANONNAME != 0 {
        host |= libc::AI_CANONNAME;
    }
    if flags & B_AI_NUMERICHOST != 0 {
        host |= libc::AI_NUMERICHOST;
    }
    if flags & B_AI_NUMERICSERV != 0 {
        host |= libc::AI_NUMERICSERV;
    }
    if flags & B_AI_ALL != 0 {
        host |= libc::AI_ALL;
    }
    if flags & B_AI_ADDRCONFIG != 0 {
        host |= libc::AI_ADDRCONFIG;
    }
    if flags & (B_AI_V4MAPPED | B_AI_V4MAPPED_CFG) != 0 {
        host |= libc::AI_V4MAPPED;
    }
    host
}

fn eai_to_bionic(code: c_int) -> c_int {
    match code {
        0 => 0,
        libc::EAI_BADFLAGS => 3,
        libc::EAI_NONAME => 8,
        libc::EAI_AGAIN => 2,
        libc::EAI_FAIL => 4,
        libc::EAI_FAMILY => 5,
        libc::EAI_SOCKTYPE => 10,
        libc::EAI_SERVICE => 9,
        libc::EAI_MEMORY => 6,
        libc::EAI_SYSTEM => 11,
        libc::EAI_OVERFLOW => 14,
        libc::EAI_NODATA => 7,
        // glibc/Linux value; libc does not expose this extension on every target.
        -9 => 1,
        _ => 4,
    }
}

unsafe fn free_bionic_list(mut node: *mut BionicAddrinfo) {
    while !node.is_null() {
        // SAFETY: callers pass only a list allocated by `getaddrinfo_bionic`.
        let next = unsafe { (*node).ai_next };
        // SAFETY: each field was allocated by libc in `getaddrinfo_bionic`.
        unsafe {
            libc::free((*node).ai_addr.cast());
            libc::free((*node).ai_canonname.cast());
            libc::free(node.cast());
        }
        node = next;
    }
}

unsafe fn copy_host_list(host: *mut libc::addrinfo) -> Result<*mut BionicAddrinfo, ()> {
    let mut head = ptr::null_mut();
    let mut tail: *mut *mut BionicAddrinfo = &mut head;
    let mut entry = host;
    while !entry.is_null() {
        // SAFETY: the host resolver returns a valid linked list until freed.
        let source = unsafe { &*entry };
        // SAFETY: zeroed allocation is large enough for this repr(C) structure.
        let node = unsafe {
            libc::calloc(1, std::mem::size_of::<BionicAddrinfo>()).cast::<BionicAddrinfo>()
        };
        if node.is_null() {
            // SAFETY: the partially built list follows our allocator contract.
            unsafe { free_bionic_list(head) };
            return Err(());
        }
        // SAFETY: node is writable and newly allocated.
        unsafe {
            (*node).ai_family = source.ai_family;
            (*node).ai_socktype = source.ai_socktype;
            (*node).ai_protocol = source.ai_protocol;
            (*node).ai_addrlen = source.ai_addrlen;
            if !source.ai_addr.is_null() && source.ai_addrlen > 0 {
                let addr = libc::malloc(source.ai_addrlen as usize).cast::<libc::sockaddr>();
                if addr.is_null() {
                    libc::free(node.cast());
                    free_bionic_list(head);
                    return Err(());
                }
                ptr::copy_nonoverlapping(
                    source.ai_addr.cast::<u8>(),
                    addr.cast::<u8>(),
                    source.ai_addrlen as usize,
                );
                (*node).ai_addr = addr;
            }
            if !source.ai_canonname.is_null() {
                let len = libc::strlen(source.ai_canonname) + 1;
                let name = libc::malloc(len).cast::<c_char>();
                if name.is_null() {
                    libc::free(node.cast());
                    free_bionic_list(head);
                    return Err(());
                }
                ptr::copy_nonoverlapping(source.ai_canonname, name, len);
                (*node).ai_canonname = name;
            }
            *tail = node;
            tail = &mut (*node).ai_next;
        }
        // SAFETY: source points at a valid host addrinfo entry.
        entry = source.ai_next;
    }
    Ok(head)
}

/// C ABI-compatible bionic `getaddrinfo` replacement.
///
/// # Safety
/// `node`, `service`, and `hints` must be valid for the duration of the call;
/// `result` must be writable when non-null. Any returned list must be released
/// with `freeaddrinfo_bionic`.
#[unsafe(no_mangle)]
unsafe extern "C" fn getaddrinfo_bionic(
    node: *const c_char,
    service: *const c_char,
    hints: *const BionicAddrinfo,
    result: *mut *mut BionicAddrinfo,
) -> c_int {
    if !result.is_null() {
        // SAFETY: a non-null result is writable by the function's C contract.
        unsafe { result.write(ptr::null_mut()) };
    }
    let mut host_hints: libc::addrinfo = unsafe { std::mem::zeroed() };
    if !hints.is_null() {
        // SAFETY: the C caller promises a readable bionic addrinfo hints struct.
        let input = unsafe { &*hints };
        host_hints.ai_flags = flags_to_host(input.ai_flags);
        host_hints.ai_family = input.ai_family;
        host_hints.ai_socktype = input.ai_socktype;
        host_hints.ai_protocol = input.ai_protocol;
    }
    let mut host_result = ptr::null_mut();
    // SAFETY: pointers follow libc getaddrinfo's contract; host_result is local.
    let status = unsafe {
        libc::getaddrinfo(
            node,
            service,
            if hints.is_null() {
                ptr::null()
            } else {
                &host_hints
            },
            &mut host_result,
        )
    };
    if status != 0 {
        return eai_to_bionic(status);
    }
    // SAFETY: host_result is a valid list returned by libc.
    let copied = unsafe { copy_host_list(host_result) };
    // SAFETY: host_result is owned by this function.
    unsafe { libc::freeaddrinfo(host_result) };
    match copied {
        Ok(list) => {
            if result.is_null() {
                // Preserve the C++ wrapper behavior for callers that discard output.
                // SAFETY: list was allocated by copy_host_list and is not otherwise used.
                unsafe { free_bionic_list(list) };
            } else {
                // SAFETY: result is writable and list is now owned by the caller.
                unsafe { result.write(list) };
            }
            0
        }
        Err(()) => 6,
    }
}

/// Release a bionic-layout list returned by `getaddrinfo_bionic`.
///
/// # Safety
/// `list` must be null or a list returned by `getaddrinfo_bionic` exactly once.
#[unsafe(no_mangle)]
unsafe extern "C" fn freeaddrinfo_bionic(list: *mut BionicAddrinfo) {
    // SAFETY: guaranteed by this function's C contract.
    unsafe { free_bionic_list(list) };
}

pub fn overrides() -> Vec<(&'static str, *mut c_void)> {
    vec![
        (
            "getaddrinfo",
            getaddrinfo_bionic as *const () as *mut c_void,
        ),
        (
            "freeaddrinfo",
            freeaddrinfo_bionic as *const () as *mut c_void,
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numeric_ipv4_result_uses_bionic_layout_and_free_pair() {
        let node = c"127.0.0.1";
        let service = c"80";
        let hints = BionicAddrinfo {
            ai_flags: B_AI_NUMERICHOST | B_AI_NUMERICSERV,
            ai_family: libc::AF_INET,
            ai_socktype: libc::SOCK_STREAM,
            ai_protocol: 0,
            ai_addrlen: 0,
            ai_canonname: ptr::null_mut(),
            ai_addr: ptr::null_mut(),
            ai_next: ptr::null_mut(),
        };
        let mut result = ptr::null_mut();
        // SAFETY: all pointers refer to valid storage or static C strings.
        let status =
            unsafe { getaddrinfo_bionic(node.as_ptr(), service.as_ptr(), &hints, &mut result) };
        assert_eq!(status, 0);
        assert!(!result.is_null());
        // SAFETY: successful resolver result follows our bionic layout.
        let first = unsafe { &*result };
        assert_eq!(first.ai_family, libc::AF_INET);
        assert!(!first.ai_addr.is_null());
        // SAFETY: returned ai_addr points to a sockaddr_in allocation.
        let address = unsafe { &*first.ai_addr.cast::<libc::sockaddr_in>() };
        assert_eq!(address.sin_addr.s_addr, u32::from_ne_bytes([127, 0, 0, 1]));
        // SAFETY: release exactly once using the matching exported deallocator.
        unsafe { freeaddrinfo_bionic(result) };
    }

    #[test]
    fn maps_host_resolver_failure_to_bionic_number() {
        let node = c"127.0.0.1";
        let mut result = ptr::null_mut();
        let hints = BionicAddrinfo {
            ai_flags: 0,
            ai_family: 12345,
            ai_socktype: 0,
            ai_protocol: 0,
            ai_addrlen: 0,
            ai_canonname: ptr::null_mut(),
            ai_addr: ptr::null_mut(),
            ai_next: ptr::null_mut(),
        };
        // SAFETY: all pointers refer to valid storage or static C strings.
        let status = unsafe { getaddrinfo_bionic(node.as_ptr(), ptr::null(), &hints, &mut result) };
        assert_eq!(status, 5);
        assert!(result.is_null());
    }

    #[test]
    fn accepts_a_null_result_and_releases_discarded_list() {
        let node = c"127.0.0.1";
        let service = c"80";
        // SAFETY: inputs are static C strings; a null output requests that the
        // resolver discard and free any successful list, matching the old ABI.
        let status = unsafe {
            getaddrinfo_bionic(
                node.as_ptr(),
                service.as_ptr(),
                ptr::null(),
                ptr::null_mut(),
            )
        };
        assert_eq!(status, 0);
    }

    #[test]
    fn registers_bionic_resolver_symbols() {
        let entries = overrides();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].0, "getaddrinfo");
        assert_eq!(entries[1].0, "freeaddrinfo");
        assert!(!entries[0].1.is_null());
        assert!(!entries[1].1.is_null());
    }
}
