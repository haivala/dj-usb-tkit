//! How the process gets memory from the system and gives it back.
//!
//! Analyzing one track allocates and frees several gigabytes in small
//! blocks. An allocator that returns each freed block to the system asks for
//! it again a moment later, and with several tracks analyzed at once the
//! workers end up waiting on the system's memory management instead of
//! analyzing. So freed memory is kept for reuse while an analysis job runs,
//! and given back in one go when it is done. Outside a job the allocator
//! returns memory as it normally does.
//!
//! Linux (glibc) does this with its own allocator. Windows uses mimalloc: its
//! system heap has no such setting. Other systems are left as they are.

#[cfg(windows)]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

/// mimalloc's `mi_option_purge_delay`, which the bindings do not name: the
/// option just before `mi_option_use_numa_nodes` in mimalloc v2.
#[cfg(windows)]
const MI_OPTION_PURGE_DELAY: libmimalloc_sys::mi_option_t =
    libmimalloc_sys::mi_option_use_numa_nodes - 1;

/// How long mimalloc keeps freed memory before returning it, in
/// milliseconds: during a job, and its own default outside one.
#[cfg(windows)]
const JOB_PURGE_DELAY_MS: std::ffi::c_long = 1000;
#[cfg(windows)]
const DEFAULT_PURGE_DELAY_MS: std::ffi::c_long = 10;

/// Above the 64 MiB a glibc thread arena grows in, so that a freed arena
/// block is kept instead of unmapped.
#[cfg(all(target_os = "linux", target_env = "gnu"))]
const JOB_TOP_PAD_BYTES: libc::c_int = 256 * 1024 * 1024;
/// glibc's own trim threshold and top pad.
#[cfg(all(target_os = "linux", target_env = "gnu"))]
const DEFAULT_TRIM_AND_PAD_BYTES: libc::c_int = 128 * 1024;

/// Keep freed memory for reuse, until [`stop_keeping_freed_memory`]. Called
/// when an analysis job starts.
pub fn keep_freed_memory() {
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    // SAFETY: `mallopt` only changes the allocator's own tuning values.
    unsafe {
        libc::mallopt(libc::M_TRIM_THRESHOLD, libc::c_int::MAX);
        libc::mallopt(libc::M_TOP_PAD, JOB_TOP_PAD_BYTES);
    }
    #[cfg(windows)]
    // SAFETY: sets one of mimalloc's own options to a value it documents.
    unsafe {
        libmimalloc_sys::mi_option_set(MI_OPTION_PURGE_DELAY, JOB_PURGE_DELAY_MS);
    }
}

/// Go back to returning freed memory as usual, and give what was kept back
/// to the system. Called when an analysis job ends.
pub fn stop_keeping_freed_memory() {
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    // SAFETY: `mallopt` only changes the allocator's own tuning values.
    unsafe {
        libc::mallopt(libc::M_TRIM_THRESHOLD, DEFAULT_TRIM_AND_PAD_BYTES);
        libc::mallopt(libc::M_TOP_PAD, DEFAULT_TRIM_AND_PAD_BYTES);
    }
    #[cfg(windows)]
    // SAFETY: sets one of mimalloc's own options back to its default.
    unsafe {
        libmimalloc_sys::mi_option_set(MI_OPTION_PURGE_DELAY, DEFAULT_PURGE_DELAY_MS);
    }
    release_freed_memory();
}

/// Give the memory kept by [`keep_freed_memory`] back to the system now.
pub fn release_freed_memory() {
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    // SAFETY: `malloc_trim` only returns memory the allocator holds unused.
    unsafe {
        libc::malloc_trim(0);
    }
    #[cfg(windows)]
    // SAFETY: `mi_collect` only returns memory mimalloc holds unused.
    unsafe {
        libmimalloc_sys::mi_collect(true);
    }
}
