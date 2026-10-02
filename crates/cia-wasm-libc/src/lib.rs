//! Minimal libc shim for `wasm32-unknown-unknown`.
//!
//! The C codecs (mozjpeg, libwebp, libdeflate) are compiled against the
//! wasi-sdk headers but `wasm32-unknown-unknown` links no libc, so every libc
//! symbol they reference has to come from somewhere. This crate provides them,
//! backed by Rust's global allocator. On every other target the crate is empty.
//!
//! Symbols are gated behind features named after the library that needs them
//! (see `Cargo.toml`) so a final module links exactly the set it needs.
//! zstd-sys brings its own prefixed shim and needs nothing from here.
//!
//! Variadic C functions (`fprintf`, `snprintf`) are declared with a trailing
//! pointer argument: on wasm32 clang passes the `va_list` as a single pointer
//! to the argument area, so the wasm signature matches exactly.
//!
//! Depend on this crate with `use cia_wasm_libc as _;` so the linker keeps it.

#![cfg_attr(not(target_arch = "wasm32"), allow(unused))]
#![no_std]

#[cfg(all(target_arch = "wasm32", feature = "_alloc"))]
extern crate alloc;

#[cfg(all(target_arch = "wasm32", feature = "_alloc"))]
mod malloc {
    use alloc::alloc::{alloc, alloc_zeroed, dealloc, Layout};
    use core::ffi::c_void;
    use core::ptr;

    /// Header in front of each block: the usable size. 16 bytes keeps the
    /// payload aligned to `max_align_t` on wasm32.
    const HEADER: usize = 16;
    const ALIGN: usize = 16;

    fn layout_for(size: usize) -> Option<Layout> {
        let total = size.checked_add(HEADER)?;
        Layout::from_size_align(total, ALIGN).ok()
    }

    unsafe fn raw_alloc(size: usize, zeroed: bool) -> *mut c_void {
        let Some(layout) = layout_for(size) else {
            return ptr::null_mut();
        };
        let base = if zeroed {
            alloc_zeroed(layout)
        } else {
            alloc(layout)
        };
        if base.is_null() {
            return ptr::null_mut();
        }
        (base as *mut usize).write(size);
        base.add(HEADER) as *mut c_void
    }

    unsafe fn size_of(p: *mut c_void) -> usize {
        ((p as *mut u8).sub(HEADER) as *mut usize).read()
    }

    #[no_mangle]
    pub unsafe extern "C" fn malloc(size: usize) -> *mut c_void {
        raw_alloc(size, false)
    }

    #[no_mangle]
    pub unsafe extern "C" fn calloc(nmemb: usize, size: usize) -> *mut c_void {
        match nmemb.checked_mul(size) {
            Some(total) => raw_alloc(total, true),
            None => ptr::null_mut(),
        }
    }

    #[no_mangle]
    pub unsafe extern "C" fn free(p: *mut c_void) {
        if p.is_null() {
            return;
        }
        let size = size_of(p);
        let layout = layout_for(size).expect("block was allocated with a valid layout");
        dealloc((p as *mut u8).sub(HEADER), layout);
    }

    #[no_mangle]
    pub unsafe extern "C" fn realloc(p: *mut c_void, new_size: usize) -> *mut c_void {
        if p.is_null() {
            return raw_alloc(new_size, false);
        }
        if new_size == 0 {
            free(p);
            return ptr::null_mut();
        }
        let old_size = size_of(p);
        let new = raw_alloc(new_size, false);
        if !new.is_null() {
            ptr::copy_nonoverlapping(p as *const u8, new as *mut u8, old_size.min(new_size));
            free(p);
        }
        new
    }
}

#[cfg(all(target_arch = "wasm32", any(feature = "_alloc", feature = "_stdio")))]
mod misc {
    use core::ffi::{c_char, c_int};

    #[no_mangle]
    pub unsafe extern "C" fn strlen(s: *const c_char) -> usize {
        let mut n = 0usize;
        while s.add(n).read() != 0 {
            n += 1;
        }
        n
    }

    #[no_mangle]
    pub extern "C" fn abort() -> ! {
        core::arch::wasm32::unreachable()
    }

    #[no_mangle]
    pub extern "C" fn exit(_code: c_int) -> ! {
        core::arch::wasm32::unreachable()
    }

    #[no_mangle]
    pub extern "C" fn getenv(_name: *const c_char) -> *mut c_char {
        core::ptr::null_mut()
    }
}

#[cfg(all(target_arch = "wasm32", feature = "_stdio"))]
mod stdio {
    use core::ffi::{c_char, c_int, c_void};

    /// `FILE *stderr` as declared by the wasi headers. Only its address is
    /// taken; every stdio stub ignores the stream.
    #[no_mangle]
    pub static stderr: usize = 0;
    #[no_mangle]
    pub static stdout: usize = 0;

    #[no_mangle]
    pub extern "C" fn fprintf(_f: *mut c_void, _fmt: *const c_char, _va: *mut c_void) -> c_int {
        0
    }

    #[no_mangle]
    pub extern "C" fn fflush(_f: *mut c_void) -> c_int {
        0
    }

    #[no_mangle]
    pub extern "C" fn fputs(_s: *const c_char, _f: *mut c_void) -> c_int {
        0
    }

    #[no_mangle]
    pub extern "C" fn fputc(c: c_int, _f: *mut c_void) -> c_int {
        c
    }

    /// Only referenced by libjpeg's stdio destination, which we never install
    /// (`jpeg_mem_dest` is used). Reports that nothing was written.
    #[no_mangle]
    pub extern "C" fn fwrite(
        _ptr: *const c_void,
        _size: usize,
        _nmemb: usize,
        _f: *mut c_void,
    ) -> usize {
        0
    }

    /// Copies the format string, truncated, so an error message is at least
    /// recognisable. Format arguments are not expanded.
    #[no_mangle]
    pub unsafe extern "C" fn snprintf(
        buf: *mut c_char,
        n: usize,
        fmt: *const c_char,
        _va: *mut c_void,
    ) -> c_int {
        let len = super::misc::strlen(fmt);
        if n > 0 && !buf.is_null() {
            let copy = len.min(n - 1);
            core::ptr::copy_nonoverlapping(fmt, buf, copy);
            buf.add(copy).write(0);
        }
        len as c_int
    }
}

#[cfg(all(target_arch = "wasm32", feature = "_qsort"))]
mod qsort {
    use core::ffi::{c_int, c_void};
    use core::ptr;

    type Compar = unsafe extern "C" fn(*const c_void, *const c_void) -> c_int;

    /// In-place heapsort: O(n log n), no allocation, no recursion. libwebp
    /// sorts palettes (at most 256 entries) and Huffman symbol tables with it.
    #[no_mangle]
    pub unsafe extern "C" fn qsort(
        base: *mut c_void,
        nmemb: usize,
        size: usize,
        compar: Option<Compar>,
    ) {
        let Some(cmp) = compar else { return };
        if nmemb < 2 || size == 0 {
            return;
        }
        let base = base as *mut u8;
        let at = |i: usize| base.add(i * size);
        let swap = |a: usize, b: usize| ptr::swap_nonoverlapping(at(a), at(b), size);
        let less = |a: usize, b: usize| cmp(at(a).cast(), at(b).cast()) < 0;

        let sift_down = |mut root: usize, end: usize| loop {
            let left = 2 * root + 1;
            if left >= end {
                break;
            }
            let mut child = left;
            if left + 1 < end && less(left, left + 1) {
                child = left + 1;
            }
            if less(root, child) {
                swap(root, child);
                root = child;
            } else {
                break;
            }
        };

        for start in (0..nmemb / 2).rev() {
            sift_down(start, nmemb);
        }
        for end in (1..nmemb).rev() {
            swap(0, end);
            sift_down(0, end);
        }
    }
}
