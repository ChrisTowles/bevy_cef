//! Chromium's malloc memory-dump provider (`base::trace_event::MallocDumpProvider`) reads glibc's
//! deprecated `mallinfo()`, whose fields are `int`: past 2 GiB of heap they wrap negative, and the
//! provider's checked casts kill the process (`ud2` on the MemoryInfra thread, SIGILL) at the next
//! periodic memory dump. A host app (a game) easily holds that much.
//!
//! This `mallinfo` reads `mallinfo2()` (`size_t` fields) and saturates every field at a quarter of
//! `INT_MAX`, so the sums the provider forms stay positive. libcef resolves `mallinfo` through its
//! PLT, so this one wins once the executable exports it: link the app with
//! `-Wl,--export-dynamic-symbol=mallinfo`. The values only feed Chromium's memory metrics.

use std::os::raw::c_int;

#[repr(C)]
pub struct Mallinfo {
    arena: c_int,
    ordblks: c_int,
    smblks: c_int,
    hblks: c_int,
    hblkhd: c_int,
    usmblks: c_int,
    fsmblks: c_int,
    uordblks: c_int,
    fordblks: c_int,
    keepcost: c_int,
}

#[repr(C)]
struct Mallinfo2 {
    arena: usize,
    ordblks: usize,
    smblks: usize,
    hblks: usize,
    hblkhd: usize,
    usmblks: usize,
    fsmblks: usize,
    uordblks: usize,
    fordblks: usize,
    keepcost: usize,
}

unsafe extern "C" {
    fn mallinfo2() -> Mallinfo2;
}

/// glibc's `mallinfo`, saturating instead of wrapping.
#[unsafe(no_mangle)]
pub extern "C" fn mallinfo() -> Mallinfo {
    // SAFETY: glibc >= 2.33; no arguments, returns by value.
    let m = unsafe { mallinfo2() };
    let fit = |v: usize| v.min(c_int::MAX as usize / 4) as c_int;
    Mallinfo {
        arena: fit(m.arena),
        ordblks: fit(m.ordblks),
        smblks: fit(m.smblks),
        hblks: fit(m.hblks),
        hblkhd: fit(m.hblkhd),
        usmblks: fit(m.usmblks),
        fsmblks: fit(m.fsmblks),
        uordblks: fit(m.uordblks),
        fordblks: fit(m.fordblks),
        keepcost: fit(m.keepcost),
    }
}
