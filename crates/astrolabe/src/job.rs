//! Tools must not outlive a session on Windows: the adapter runs in a Job Object that
//! kills everything in it when its last handle closes, including when the core dies.
//! Declared by hand, for two calls and a struct, rather than adding a dependency.
//! `CREATE_NEW_PROCESS_GROUP` isn't needed: a job doesn't go by console groups. Elsewhere
//! this does nothing, and on the Mac a crash leaves a tool that left the adapter's group.
use std::process::Child;

#[cfg_attr(not(windows), allow(dead_code))]
pub struct Job {
    #[cfg(windows)]
    _handle: windows::Handle,
}

impl Job {
    /// None if the OS refused. Then `kill_tree` is all there is, as before. A process the
    /// adapter starts in the moment before this is out of the job.
    #[cfg(windows)]
    pub fn assign(child: &Child) -> Option<Self> {
        use std::os::windows::io::AsRawHandle;
        let handle = windows::create_kill_on_close()?;
        windows::assign(&handle, child.as_raw_handle()).then_some(Self { _handle: handle })
    }

    #[cfg(not(windows))]
    pub fn assign(_child: &Child) -> Option<Self> {
        None
    }
}

#[cfg(windows)]
mod windows {
    use std::ffi::c_void;
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle, RawHandle};

    pub type Handle = OwnedHandle;

    const JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE: u32 = 0x2000;
    const JOB_OBJECT_EXTENDED_LIMIT_INFORMATION: i32 = 9;

    #[repr(C)]
    #[derive(Default)]
    #[allow(dead_code)] // The OS reads these.
    struct BasicLimit {
        per_process_user_time: i64,
        per_job_user_time: i64,
        limit_flags: u32,
        minimum_working_set: usize,
        maximum_working_set: usize,
        active_process_limit: u32,
        affinity: usize,
        priority_class: u32,
        scheduling_class: u32,
    }

    #[repr(C)]
    #[derive(Default)]
    #[allow(dead_code)] // The OS reads these.
    struct IoCounters {
        counts: [u64; 6],
    }

    #[repr(C)]
    #[derive(Default)]
    #[allow(dead_code)] // The OS reads these.
    struct ExtendedLimit {
        basic: BasicLimit,
        io: IoCounters,
        process_memory_limit: usize,
        job_memory_limit: usize,
        peak_process_memory_used: usize,
        peak_job_memory_used: usize,
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn CreateJobObjectW(attributes: *mut c_void, name: *const u16) -> *mut c_void;
        fn SetInformationJobObject(
            job: *mut c_void,
            class: i32,
            info: *const c_void,
            size: u32,
        ) -> i32;
        fn AssignProcessToJobObject(job: *mut c_void, process: *mut c_void) -> i32;
    }

    pub fn create_kill_on_close() -> Option<OwnedHandle> {
        // SAFETY: both pointers may be null; a non-null result is a handle we now own.
        let raw = unsafe { CreateJobObjectW(std::ptr::null_mut(), std::ptr::null()) };
        if raw.is_null() {
            return None;
        }
        // SAFETY: `raw` was just returned by the OS and nothing else holds it.
        let handle = unsafe { OwnedHandle::from_raw_handle(raw) };
        let mut limit = ExtendedLimit::default();
        limit.basic.limit_flags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        // SAFETY: the pointer and size describe a live struct of the class's layout.
        let set = unsafe {
            SetInformationJobObject(
                handle.as_raw_handle(),
                JOB_OBJECT_EXTENDED_LIMIT_INFORMATION,
                (&raw const limit).cast(),
                size_of::<ExtendedLimit>() as u32,
            )
        };
        (set != 0).then_some(handle)
    }

    pub fn assign(job: &OwnedHandle, process: RawHandle) -> bool {
        // SAFETY: both handles are live for this call.
        unsafe { AssignProcessToJobObject(job.as_raw_handle(), process) != 0 }
    }
}
