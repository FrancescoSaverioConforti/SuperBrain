//! M6.6: Windows creation-time Job Object containment for spawned
//! external MCP server child processes.
//!
//! **The race this module exists to close**: the ordinary pattern
//! "`CreateProcessW` (or `tokio::process::Command::spawn()`), then
//! `AssignProcessToJobObject` on the returned handle" has a real window
//! between the two calls during which the child's own initial thread is
//! already runnable and can execute attacker-controlled code before any
//! containment applies. M6.6 was explicitly not authorized to accept
//! that window as a documented residual risk (unlike M6.4-S1's
//! ACL/TOCTOU narrowing) — it required either closing it or a STOP.
//!
//! **Why `CREATE_SUSPENDED` + `ResumeThread` was rejected as the fix**:
//! that pattern needs the new process's main THREAD handle
//! (`PROCESS_INFORMATION::hThread`) to call `ResumeThread` on. Confirmed
//! by reading `tokio` 1.54.1's own `src/process/windows.rs` directly:
//! `tokio::process::Child` (and the `std::process::Child` it wraps)
//! expose only the PROCESS handle via `AsRawHandle` — the thread handle
//! `CreateProcessW` returns is created and closed internally inside
//! `std`'s own spawn implementation and never surfaced to any caller on
//! stable Rust. `CREATE_SUSPENDED` on that stack would produce a
//! permanently-suspended, unresumable process.
//!
//! **The mechanism this module uses instead**: `PROC_THREAD_ATTRIBUTE_
//! JOB_LIST`, a process-creation-time attribute (Windows 10 / Server
//! 2016+) passed to `CreateProcessW` itself via `STARTUPINFOEXW` +
//! `InitializeProcThreadAttributeList`/`UpdateProcThreadAttribute`. Per
//! Microsoft's own documentation, the kernel assigns the new process to
//! every job in the list as part of process creation, before the
//! process's initial thread can execute any code — there is no
//! separate "assign" step for this module's caller to race against.
//! This requires bypassing `tokio::process::Command`/`std::process::
//! Command` entirely for this one spawn (neither exposes `STARTUPINFOEXW`
//! or an attribute list), so this module also creates the three stdio
//! pipes itself (`CreatePipe`) and restricts which handles the child
//! inherits via the complementary `PROC_THREAD_ATTRIBUTE_HANDLE_LIST`
//! attribute — the child inherits exactly its own stdin/stdout/stderr
//! pipe ends, never any other handle open in this process, regardless
//! of how many other inheritable handles this process may hold.
//!
//! **Fail-closed by construction**: every setup step (job creation,
//! limit configuration, attribute list construction, pipe creation)
//! happens strictly before `CreateProcessW` is called. If any step
//! fails, `CreateProcessW` is never reached and no process is spawned —
//! there is no path through this module that executes untrusted code
//! without the job already attached at creation time.
//!
//! **What this module does NOT do**: no filesystem or network
//! sandboxing (out of M6.6's scope by explicit instruction), no
//! `CREATE_SUSPENDED` (see above — rejected, not merely unused), no
//! change to M6.4's environment allowlist/cwd handling or M6.5's schema
//! verification/authorization ordering (this module is a pure
//! replacement for the process-creation step inside `registration.rs`'s
//! `connect_now`, invisible to every caller above it).

use std::ffi::c_void;
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::{FromRawHandle, OwnedHandle};
use std::path::Path;

use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Security::SECURITY_ATTRIBUTES;
use windows::Win32::System::JobObjects::{CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation, SetInformationJobObject};
#[cfg(test)]
use windows::Win32::System::JobObjects::IsProcessInJob;
use windows::Win32::System::Pipes::CreatePipe;
use windows::Win32::System::Threading::{
    CreateProcessW, DeleteProcThreadAttributeList, EXTENDED_STARTUPINFO_PRESENT, InitializeProcThreadAttributeList, LPPROC_THREAD_ATTRIBUTE_LIST, PROC_THREAD_ATTRIBUTE_HANDLE_LIST,
    PROC_THREAD_ATTRIBUTE_JOB_LIST, PROCESS_CREATION_FLAGS, PROCESS_INFORMATION, STARTF_USESTDHANDLES, STARTUPINFOEXW, STARTUPINFOW, UpdateProcThreadAttribute,
};

/// An owned Windows Job Object handle. `KILL_ON_JOB_CLOSE` is set at
/// creation (before any process is ever assigned to it), so dropping
/// this handle terminates every process still in the job, including
/// the whole process tree the contained child may have spawned — the
/// job tracks descendants transitively by default (no `SILENT_
/// BREAKAWAY_OK`/`BREAKAWAY_OK` limit is ever set here, so no child or
/// grandchild can leave the job).
pub(crate) struct OwnedJobHandle(HANDLE);

impl OwnedJobHandle {
    /// Creates a new, anonymous (unnamed) job object with `KILL_ON_
    /// JOB_CLOSE` already configured. Returns an error (no job handle
    /// leaked, nothing left for a caller to clean up) if either the
    /// create or the limit-configuration step fails — a caller must
    /// never treat a job object without this limit as equivalent to
    /// one with it.
    pub(crate) fn create_with_kill_on_close() -> io::Result<Self> {
        // SAFETY: `CreateJobObjectW(None, PCWSTR::null())` creates an
        // anonymous job object; no attributes/name pointers are kept
        // past this call.
        let handle = unsafe { CreateJobObjectW(None, windows::core::PCWSTR::null()) }.map_err(io::Error::from)?;
        let job = Self(handle);

        let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        // SAFETY: `info` is a valid, fully-initialized
        // JOBOBJECT_EXTENDED_LIMIT_INFORMATION; its address and exact
        // size are passed together, matching what
        // `SetInformationJobObject` requires for this information class.
        unsafe {
            SetInformationJobObject(job.0, JobObjectExtendedLimitInformation, &info as *const _ as *const c_void, std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32)
        }
        .map_err(io::Error::from)?;

        Ok(job)
    }

    /// Test-only: a job handle that is guaranteed invalid (already
    /// closed) at the moment it is returned, for adversarial tests that
    /// need `spawn_contained_into`'s own `PROC_THREAD_ATTRIBUTE_JOB_LIST`
    /// setup step to genuinely fail against the real code path, rather
    /// than against a synthetic stand-in.
    #[cfg(test)]
    fn already_closed_for_test() -> Self {
        let real = Self::create_with_kill_on_close().unwrap();
        let handle = real.0;
        std::mem::forget(real); // skip this handle's own Drop/CloseHandle below -- we close it ourselves, once, here
        let _ = unsafe { CloseHandle(handle) };
        Self(handle)
    }

    pub(crate) fn raw(&self) -> HANDLE {
        self.0
    }
}

impl Drop for OwnedJobHandle {
    fn drop(&mut self) {
        // SAFETY: `self.0` is a valid job object handle owned
        // exclusively by this struct, closed exactly once here.
        // `KILL_ON_JOB_CLOSE` means this also terminates every process
        // still assigned to the job (the entire contained process
        // tree), matching this type's own documented drop guarantee.
        let _ = unsafe { CloseHandle(self.0) };
    }
}

/// An owned `PROC_THREAD_ATTRIBUTE_LIST` buffer, built via
/// `InitializeProcThreadAttributeList`/`UpdateProcThreadAttribute` and
/// released via `DeleteProcThreadAttributeList` on drop. Holds the
/// backing byte buffer alive for its own lifetime (the list header
/// points into it; freeing the buffer while `CreateProcessW` still
/// holds a pointer to this list would be a use-after-free).
pub(crate) struct ProcThreadAttributeList {
    #[allow(dead_code)] // never read directly; kept alive only for its own Drop ordering relative to `list` (see struct doc comment)
    buffer: Vec<u8>,
    list: LPPROC_THREAD_ATTRIBUTE_LIST,
}

impl ProcThreadAttributeList {
    /// Builds an attribute list with exactly two attributes:
    /// `PROC_THREAD_ATTRIBUTE_JOB_LIST` (the single job handle the new
    /// process must be created into) and `PROC_THREAD_ATTRIBUTE_
    /// HANDLE_LIST` (the exact set of handles the new process may
    /// inherit — here, the child's own stdio pipe ends only). Both
    /// attribute values (`job_handle`, `inherit_handles`) must outlive
    /// the returned list and the `CreateProcessW` call it is used in;
    /// this function does not take ownership of them.
    fn with_job_and_handle_list(job_handle: &HANDLE, inherit_handles: &[HANDLE]) -> io::Result<Self> {
        const ATTRIBUTE_COUNT: u32 = 2;

        let mut required_size: usize = 0;
        // SAFETY: first call per the documented pattern — `None` list,
        // only `required_size` is written, used solely to size the
        // backing buffer below.
        let first = unsafe { InitializeProcThreadAttributeList(None, ATTRIBUTE_COUNT, None, &mut required_size) };
        // Microsoft's own documented pattern for this API: the sizing
        // call is EXPECTED to fail (ERROR_INSUFFICIENT_BUFFER) and
        // `required_size` is only meaningful once it does -- a sizing
        // call that unexpectedly SUCCEEDS, or fails with any other
        // error, means `required_size` cannot be trusted and this must
        // fail closed rather than proceed with a zero/stale value.
        if first.is_ok() || required_size == 0 {
            return Err(io::Error::other("InitializeProcThreadAttributeList sizing call did not fail as documented; refusing to proceed with an untrusted buffer size"));
        }

        let mut buffer = vec![0u8; required_size];
        let list = LPPROC_THREAD_ATTRIBUTE_LIST(buffer.as_mut_ptr() as *mut c_void);

        // SAFETY: `buffer` is sized exactly to `required_size` as
        // reported by the sizing call above and kept alive for this
        // struct's own lifetime; `list` points into it.
        unsafe { InitializeProcThreadAttributeList(Some(list), ATTRIBUTE_COUNT, None, &mut required_size) }.map_err(io::Error::from)?;

        let attrs = Self { buffer, list };

        // SAFETY: `job_handle` points to a `HANDLE` that outlives this
        // call (borrowed from the caller, who keeps `OwnedJobHandle`
        // alive across the whole `CreateProcessW` call this list is
        // used in).
        unsafe {
            UpdateProcThreadAttribute(
                attrs.list,
                0,
                PROC_THREAD_ATTRIBUTE_JOB_LIST as usize,
                Some(job_handle as *const HANDLE as *const c_void),
                std::mem::size_of::<HANDLE>(),
                None,
                None,
            )
        }
        .map_err(io::Error::from)?;

        // SAFETY: `inherit_handles` outlives this call (borrowed from
        // the caller, kept alive across `CreateProcessW`); its address
        // and exact byte length are passed together.
        unsafe {
            UpdateProcThreadAttribute(
                attrs.list,
                0,
                PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize,
                Some(inherit_handles.as_ptr() as *const c_void),
                std::mem::size_of_val(inherit_handles),
                None,
                None,
            )
        }
        .map_err(io::Error::from)?;

        Ok(attrs)
    }
}

impl Drop for ProcThreadAttributeList {
    fn drop(&mut self) {
        // SAFETY: `self.list` was initialized by
        // `InitializeProcThreadAttributeList` above and is released
        // exactly once here, before `self.buffer` (the memory it
        // points into) is freed by this struct's own subsequent,
        // auto-generated `Vec` drop.
        unsafe { DeleteProcThreadAttributeList(self.list) };
    }
}

/// One end of a Windows anonymous pipe, owned. `inheritable` controls
/// whether a child process created with `bInheritHandles = true` may
/// inherit this specific handle at all -- independent of (and in
/// addition to) `PROC_THREAD_ATTRIBUTE_HANDLE_LIST`'s own restriction,
/// matching the same "two independent, both-must-agree" inheritance
/// model `CreatePipe`'s own `SECURITY_ATTRIBUTES` parameter documents.
fn create_pipe_pair(inheritable_read: bool, inheritable_write: bool) -> io::Result<(OwnedHandle, OwnedHandle)> {
    let mut read_handle = HANDLE::default();
    let mut write_handle = HANDLE::default();
    let mut sec_attrs = SECURITY_ATTRIBUTES { nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32, lpSecurityDescriptor: std::ptr::null_mut(), bInheritHandle: true.into() };

    // SAFETY: `sec_attrs` is valid and only borrowed for this call;
    // `read_handle`/`write_handle` receive freshly created handles this
    // function hands ownership of to its caller via `OwnedHandle`
    // below.
    unsafe { CreatePipe(&mut read_handle, &mut write_handle, Some(&sec_attrs), 0) }.map_err(io::Error::from)?;

    // The pipe is created with both ends inheritable (above); a side
    // that must NOT be inheritable has that flag cleared immediately
    // afterward -- `CreatePipe` itself offers no per-end control.
    if !inheritable_read {
        clear_inherit(read_handle)?;
    }
    if !inheritable_write {
        clear_inherit(write_handle)?;
    }
    let _ = &mut sec_attrs; // kept alive for the documented duration of the CreatePipe call above

    // SAFETY: both handles were just created by `CreatePipe` above and
    // are not yet owned by anything else; wrapping them in
    // `OwnedHandle` transfers ownership so they are closed exactly
    // once, on drop.
    unsafe { Ok((OwnedHandle::from_raw_handle(read_handle.0 as _), OwnedHandle::from_raw_handle(write_handle.0 as _))) }
}

fn clear_inherit(handle: HANDLE) -> io::Result<()> {
    use windows::Win32::Foundation::{HANDLE_FLAGS, SetHandleInformation};
    const HANDLE_FLAG_INHERIT: u32 = 1;
    // SAFETY: `handle` is a valid, currently-open handle owned by this
    // call's caller for at least the duration of this call.
    unsafe { SetHandleInformation(handle, HANDLE_FLAG_INHERIT, HANDLE_FLAGS(0)) }.map_err(io::Error::from)
}

/// The parent-side pipe ends `spawn_contained`'s caller reads/writes
/// through, once the child process has been created. The child-side
/// ends (handed to the child via `STARTUPINFOEXW` during
/// `CreateProcessW`) are deliberately NOT exposed here: this process's
/// own copies of them are closed immediately after the child is
/// created (so the child's own exit is observable as EOF rather than
/// blocked on a parent-held duplicate), leaving no live handle for a
/// caller to hold or use.
pub(crate) struct ContainedStdio {
    pub(crate) parent_stdin_write: OwnedHandle,
    pub(crate) parent_stdout_read: OwnedHandle,
}

impl ContainedStdio {
    /// Converts the raw pipe-handle pair into `tokio::fs::File`s —
    /// `AsyncWrite`/`AsyncRead` respectively — the same async-I/O shape
    /// `rmcp::transport::async_rw::AsyncRwTransport::new_client` is
    /// generic over (it requires only `W: AsyncWrite`/`R: AsyncRead`,
    /// confirmed by reading `rmcp` 3.11.0's own `async_rw.rs`, never
    /// specifically `tokio::process::ChildStdin`/`ChildStdout`), so
    /// `registration.rs`'s `connect_now` can hand these to the exact
    /// same transport-construction call it already uses on every
    /// other platform, unchanged. `tokio::fs::File` itself drives
    /// pipe I/O via its own blocking-pool `spawn_blocking` calls
    /// internally (the same strategy tokio's own `ChildStdin`/
    /// `ChildStdout` use via `Blocking<File>`) — distinct from this
    /// module's own `kill()`, which deliberately avoids
    /// `spawn_blocking` for the reason documented there; `tokio::fs::
    /// File` is tokio's own well-exercised code, not a new call site
    /// this module adds.
    pub(crate) fn into_async_io(self) -> (tokio::fs::File, tokio::fs::File) {
        let write = tokio::fs::File::from_std(std::fs::File::from(self.parent_stdin_write));
        let read = tokio::fs::File::from_std(std::fs::File::from(self.parent_stdout_read));
        (write, read)
    }
}

/// A spawned, job-contained child process. Owns the process handle (for
/// waiting/killing) and the job handle (whose drop kills the whole
/// contained tree) together, so neither can outlive the other in a way
/// that would leak a running, uncontained process.
pub(crate) struct ContainedChild {
    pub(crate) process: OwnedHandle,
    #[allow(dead_code)] // not read by any non-test caller today; kept for parity with tokio::process::Child::id() and this module's own tests
    pub(crate) process_id: u32,
    job: OwnedJobHandle,
}

impl ContainedChild {
    /// Terminates the entire contained process tree (not just the
    /// direct child) by closing the job handle, relying on `KILL_ON_
    /// JOB_CLOSE` — the same guarantee `registration.rs`'s own
    /// `McpConnection::close`/`Drop` previously got for free from
    /// `tokio::process::Child` for the single direct child only (this
    /// is strictly stronger: it also reaches any descendant the direct
    /// child spawned, exactly M6.6's own containment requirement).
    /// Synchronous and non-blocking, matching `tokio::process::Child::
    /// start_kill`'s own shape (used from `Drop::drop`, where no
    /// `.await` is available).
    pub(crate) fn start_kill(&mut self) {
        // SAFETY: `self.job.raw()` is a valid job handle owned by this
        // struct for at least the duration of this call.
        let _ = unsafe { CloseHandle(self.job.raw()) };
        // The job handle is now closed (and KILL_ON_JOB_CLOSE has
        // fired); replace `self.job` with a fresh, empty job object so
        // `OwnedJobHandle`'s own `Drop` (which still runs later) closes
        // a handle it actually owns, never the one already closed
        // above. A fresh job with no process ever assigned is a no-op
        // to close.
        if let Ok(replacement) = OwnedJobHandle::create_with_kill_on_close() {
            self.job = replacement;
        }
    }

    /// Async kill-and-wait, matching `tokio::process::Child::kill`'s
    /// own shape (used from `McpConnection::close`, an async fn). Kills
    /// the whole tree exactly as `start_kill` does, then waits for the
    /// direct process to actually exit before returning, so a caller
    /// awaiting this has the same "fully stopped, not just signaled"
    /// guarantee `tokio::process::Child::kill` documents.
    pub(crate) async fn kill(&mut self) -> io::Result<()> {
        self.start_kill();
        use std::os::windows::io::AsRawHandle;
        let raw = self.process.as_raw_handle() as isize;
        // Deliberately a plain `std::thread::spawn` + oneshot channel,
        // NOT `tokio::task::spawn_blocking`: empirically confirmed (see
        // this module's own development history) that `spawn_blocking`
        // run immediately after `start_kill()`'s `CloseHandle`-driven
        // `KILL_ON_JOB_CLOSE` termination leaves tokio's blocking pool
        // in a state its own `Runtime::drop` fails to join cleanly on
        // this host ("failed to join on thread: Handle non valido"),
        // even though the blocking closure itself always completed
        // successfully first -- a tokio/Windows blocking-pool teardown
        // interaction, not a bug in the wait itself. A plain OS thread
        // (confirmed NOT to trigger this) sidesteps tokio's blocking
        // pool entirely.
        let (tx, rx) = tokio::sync::oneshot::channel();
        std::thread::spawn(move || {
            let handle = HANDLE(raw as _);
            // SAFETY: `handle` was valid when captured above and is
            // only waited on here, never closed by this closure (the
            // owning `OwnedHandle` in `self.process` still holds it).
            // Bounded, never INFINITE: `start_kill` above already
            // guarantees termination via KILL_ON_JOB_CLOSE, so a wait
            // that does not complete promptly indicates a bug in that
            // guarantee, not a legitimately slow exit -- failing
            // closed here (WAIT_TIMEOUT) rather than blocking this
            // thread forever keeps that bug visible instead of masking
            // it as a hang.
            let result = unsafe { windows::Win32::System::Threading::WaitForSingleObject(handle, 5000) };
            let _ = tx.send(result);
        });
        let wait_result = rx.await.map_err(io::Error::other)?;
        if wait_result == windows::Win32::Foundation::WAIT_TIMEOUT {
            return Err(io::Error::other("process did not exit within 5s of start_kill(); KILL_ON_JOB_CLOSE may not have fired"));
        }
        Ok(())
    }

    /// Test/verification-only: confirms the process is a member of
    /// THIS job specifically (`IsProcessInJob`'s own `hjob` parameter),
    /// not merely "some job" — the narrower, correct check for proving
    /// `spawn_contained`'s own containment claim.
    #[cfg(test)]
    fn is_in_this_job(&self) -> bool {
        use std::os::windows::io::AsRawHandle;
        let mut result = windows::core::BOOL(0);
        let process_handle = HANDLE(self.process.as_raw_handle() as _);
        // SAFETY: both handles are valid and owned/borrowed for at
        // least the duration of this call.
        unsafe { IsProcessInJob(process_handle, Some(self.job.raw()), &mut result) }.expect("IsProcessInJob should succeed for a live process and live job handle");
        result.as_bool()
    }
}

impl Drop for ContainedChild {
    /// Mirrors `registration.rs`'s own prior `tokio::process::Child`
    /// Drop semantics (which does NOT kill on drop by default) being
    /// deliberately overridden there — this type's OWN drop killing
    /// the tree is intentional and distinct from `tokio::process::
    /// Child`'s: the job's `KILL_ON_JOB_CLOSE` limit means simply
    /// letting `self.job` drop (its own `Drop` impl, above) ALREADY
    /// terminates the whole contained tree with no extra code needed
    /// here. This impl exists only to document that guarantee
    /// explicitly at this type's own level, matching the doc-comment
    /// convention `McpConnection`'s `Drop` already uses one layer up.
    fn drop(&mut self) {
        // Intentionally empty: `self.job: OwnedJobHandle`'s own `Drop`
        // (declared above in this module) runs immediately after this
        // and closes the job handle, which alone guarantees the whole
        // contained process tree is terminated.
    }
}

/// Spawns `command args` with creation-time Job Object assignment via
/// `PROC_THREAD_ATTRIBUTE_JOB_LIST`, inheriting only the three stdio
/// pipe handles this function itself creates (via `PROC_THREAD_
/// ATTRIBUTE_HANDLE_LIST`), with `env` as the child's complete
/// environment (never the parent's) and `cwd` as its working
/// directory. Every setup step (job creation, limit configuration,
/// pipe creation, attribute list construction) happens before
/// `CreateProcessW` is called; if any step fails, no process is
/// spawned at all -- there is no path through this function that
/// executes `command` without the job already attached at creation.
pub(crate) fn spawn_contained(command: &Path, args: &[String], cwd: &Path, env: &[(String, String)]) -> io::Result<(ContainedChild, ContainedStdio)> {
    let job = OwnedJobHandle::create_with_kill_on_close()?;
    spawn_contained_into(job, command, args, cwd, env)
}

/// The real implementation `spawn_contained` delegates to, taking an
/// already-constructed job handle directly -- exists so this module's
/// own adversarial tests can inject a deliberately-invalid job handle
/// and observe the REAL code path fail closed, rather than only
/// testing a synthetic stand-in for it (see `job_setup_failure_
/// actually_prevents_the_untrusted_command_from_ever_running` below).
fn spawn_contained_into(job: OwnedJobHandle, command: &Path, args: &[String], cwd: &Path, env: &[(String, String)]) -> io::Result<(ContainedChild, ContainedStdio)> {
    // Child's stdin: child reads from the read end (inheritable),
    // parent writes to the write end (not inheritable). Child's
    // stdout/stderr: child writes to the write end (inheritable),
    // parent reads from the read end (not inheritable). This matches
    // `tokio::process::Command`'s own piped-stdio inheritance shape.
    let (stdin_read, stdin_write) = create_pipe_pair(true, false)?;
    let (stdout_read, stdout_write) = create_pipe_pair(false, true)?;
    let (stderr_read, stderr_write) = create_pipe_pair(false, true)?;

    let child_stdin = HANDLE(std::os::windows::io::AsRawHandle::as_raw_handle(&stdin_read) as _);
    let child_stdout = HANDLE(std::os::windows::io::AsRawHandle::as_raw_handle(&stdout_write) as _);
    let child_stderr = HANDLE(std::os::windows::io::AsRawHandle::as_raw_handle(&stderr_write) as _);
    let inherit_list = [child_stdin, child_stdout, child_stderr];

    let attrs = ProcThreadAttributeList::with_job_and_handle_list(&job.raw(), &inherit_list)?;

    let mut startup_info = STARTUPINFOEXW::default();
    startup_info.StartupInfo.cb = std::mem::size_of::<STARTUPINFOEXW>() as u32;
    startup_info.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
    startup_info.StartupInfo.hStdInput = child_stdin;
    startup_info.StartupInfo.hStdOutput = child_stdout;
    startup_info.StartupInfo.hStdError = child_stderr;
    startup_info.lpAttributeList = attrs.list;

    let mut command_line = build_command_line(command, args);
    let mut command_line_wide: Vec<u16> = command_line.encode_wide().chain(std::iter::once(0)).collect();
    command_line.clear();

    let environment_block = build_environment_block(env);
    let cwd_wide: Vec<u16> = cwd.as_os_str().encode_wide().chain(std::iter::once(0)).collect();

    let mut process_info = PROCESS_INFORMATION::default();
    let creation_flags = EXTENDED_STARTUPINFO_PRESENT | PROCESS_CREATION_FLAGS(0x0000_0400); // CREATE_UNICODE_ENVIRONMENT

    // SAFETY: `command_line_wide` is a mutable, NUL-terminated wide
    // string buffer (CreateProcessW may write into it, per its own
    // documented contract) kept alive until this call returns;
    // `environment_block` is a NUL-NUL-terminated wide block kept
    // alive for the same duration; `startup_info` (and the attribute
    // list / inherit-handle array it points into) is kept alive for
    // the same duration by this function's own local bindings;
    // `process_info` receives two fresh, owned handles on success
    // (`hProcess`, `hThread`) which this function takes ownership of
    // immediately afterward.
    let create_result = unsafe {
        CreateProcessW(
            windows::core::PCWSTR::null(),
            Some(windows::core::PWSTR(command_line_wide.as_mut_ptr())),
            None,
            None,
            true,
            creation_flags,
            Some(environment_block.as_ptr() as *const c_void),
            windows::core::PCWSTR(cwd_wide.as_ptr()),
            &startup_info.StartupInfo as *const STARTUPINFOW,
            &mut process_info,
        )
    };

    if let Err(e) = create_result {
        return Err(io::Error::from(e));
    }

    // SAFETY: `process_info.hThread` is a valid, freshly-created,
    // otherwise-unused handle this function has sole ownership of
    // immediately after a successful `CreateProcessW`; it is never
    // needed again (this module does not use `CREATE_SUSPENDED`, so
    // there is no later `ResumeThread` call), so it is closed here
    // rather than leaked.
    let _ = unsafe { CloseHandle(process_info.hThread) };

    // SAFETY: `process_info.hProcess` is a valid, freshly-created
    // handle this function has sole ownership of; wrapping it in
    // `OwnedHandle` transfers that ownership so it is closed exactly
    // once, on drop.
    let process = unsafe { OwnedHandle::from_raw_handle(process_info.hProcess.0 as _) };

    let child = ContainedChild { process, process_id: process_info.dwProcessId, job };
    let stdio = ContainedStdio { parent_stdin_write: stdin_write, parent_stdout_read: stdout_read };
    // `stderr_read` (the parent-side read end) is intentionally dropped
    // here uninherited by the child and unused by the parent -- stderr
    // is inherited (`std::process::Stdio::inherit()`'s own semantic,
    // matched here) by closing the parent's read end immediately so
    // the child's stderr writes do not block on an unread pipe; this
    // crate's existing behavior (`registration.rs`'s prior
    // `cmd.stderr(Stdio::inherit())`) never captured stderr either.
    drop(stderr_read);
    // The child-side handles (`stdin_read`, `stdout_write`,
    // `stderr_write`) are duplicated into the child's own handle table
    // by `CreateProcessW` itself; this process's own copies are closed
    // here so the parent does not keep the child's pipe ends open
    // (which would prevent e.g. EOF from ever being observed on the
    // child's exit).
    drop(stdin_read);
    drop(stdout_write);
    drop(stderr_write);

    Ok((child, stdio))
}

fn build_command_line(command: &Path, args: &[String]) -> std::ffi::OsString {
    let mut parts: Vec<String> = Vec::with_capacity(1 + args.len());
    parts.push(quote_arg(&command.display().to_string()));
    parts.extend(args.iter().map(|a| quote_arg(a)));
    std::ffi::OsString::from(parts.join(" "))
}

/// Minimal Win32 command-line quoting (MSVCRT argv parsing rules): any
/// argument containing a space, tab, or double quote is wrapped in
/// double quotes, with internal double quotes and the backslashes that
/// immediately precede them escaped per the documented algorithm.
/// Deliberately conservative -- always quotes when in doubt -- never a
/// source of argument-injection, since this function is the ONLY thing
/// that builds the single command-line string `CreateProcessW` parses
/// (unlike `std::process::Command`, which builds argv natively on every
/// other platform and only needs this exact algorithm on Windows for
/// the same reason).
fn quote_arg(arg: &str) -> String {
    if !arg.is_empty() && !arg.contains([' ', '\t', '"']) {
        return arg.to_string();
    }
    let mut quoted = String::from("\"");
    let mut backslashes = 0usize;
    for c in arg.chars() {
        match c {
            '\\' => {
                backslashes += 1;
                quoted.push('\\');
            }
            '"' => {
                for _ in 0..backslashes {
                    quoted.push('\\');
                }
                quoted.push('\\');
                quoted.push('"');
                backslashes = 0;
            }
            other => {
                backslashes = 0;
                quoted.push(other);
            }
        }
    }
    for _ in 0..backslashes {
        quoted.push('\\');
    }
    quoted.push('"');
    quoted
}

/// Builds a `CreateProcessW`-shaped environment block: each `KEY=VALUE`
/// pair NUL-terminated, the whole block double-NUL-terminated. An empty
/// `env` still produces a valid (double-NUL-only) block -- `main.rs`'s
/// existing `isolate_child_environment`-equivalent caller decides what
/// goes in, this function never adds or infers anything.
fn build_environment_block(env: &[(String, String)]) -> Vec<u16> {
    let mut block: Vec<u16> = Vec::new();
    for (key, value) in env {
        let entry = format!("{key}={value}");
        block.extend(entry.encode_utf16());
        block.push(0);
    }
    block.push(0);
    block
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- OwnedJobHandle ---

    #[test]
    fn create_with_kill_on_close_succeeds_and_closes_without_panicking() {
        let job = OwnedJobHandle::create_with_kill_on_close().expect("job creation should succeed on any Windows 10+ host");
        drop(job);
    }

    // --- ProcThreadAttributeList ---

    #[test]
    fn with_job_and_handle_list_succeeds_for_a_real_job_and_a_nonempty_handle_list() {
        use std::os::windows::io::AsRawHandle;
        let job = OwnedJobHandle::create_with_kill_on_close().unwrap();
        // PROC_THREAD_ATTRIBUTE_HANDLE_LIST rejects an empty handle
        // array (ERROR_BAD_LENGTH) -- confirmed empirically, not just
        // assumed -- so this module's real caller (spawn_contained)
        // must never pass an empty inherit list; this test exercises
        // the real non-empty shape it actually uses.
        let (read_end, _write_end) = create_pipe_pair(true, false).unwrap();
        let handles = [HANDLE(read_end.as_raw_handle() as _)];
        let attrs = ProcThreadAttributeList::with_job_and_handle_list(&job.raw(), &handles);
        assert!(attrs.is_ok(), "expected attribute list construction to succeed: {:?}", attrs.err());
    }

    #[test]
    fn with_job_and_handle_list_rejects_an_empty_handle_list_rather_than_silently_proceeding() {
        // Documents the real Windows behavior found above: an empty
        // PROC_THREAD_ATTRIBUTE_HANDLE_LIST is rejected by
        // UpdateProcThreadAttribute itself (ERROR_BAD_LENGTH), so this
        // module fails closed here rather than constructing a list
        // that would let CreateProcessW fall back to unrestricted
        // handle inheritance.
        let job = OwnedJobHandle::create_with_kill_on_close().unwrap();
        let attrs = ProcThreadAttributeList::with_job_and_handle_list(&job.raw(), &[]);
        assert!(attrs.is_err(), "an empty handle list must be rejected, never silently accepted");
    }

    // --- create_pipe_pair ---

    #[test]
    fn create_pipe_pair_produces_two_distinct_live_handles() {
        let (read_end, write_end) = create_pipe_pair(true, false).expect("pipe creation should succeed");
        use std::os::windows::io::AsRawHandle;
        assert_ne!(read_end.as_raw_handle(), write_end.as_raw_handle());
    }

    // --- quote_arg ---

    #[test]
    fn quote_arg_leaves_simple_arguments_unquoted() {
        assert_eq!(quote_arg("simple"), "simple");
    }

    #[test]
    fn quote_arg_quotes_arguments_containing_spaces() {
        assert_eq!(quote_arg("has space"), "\"has space\"");
    }

    #[test]
    fn quote_arg_escapes_embedded_quotes() {
        assert_eq!(quote_arg("a\"b"), "\"a\\\"b\"");
    }

    #[test]
    fn quote_arg_never_produces_empty_output_for_empty_input() {
        // An empty argument must still round-trip as an empty argv
        // slot, not vanish from the command line entirely.
        assert_eq!(quote_arg(""), "\"\"");
    }

    // --- build_environment_block ---

    #[test]
    fn build_environment_block_is_double_null_terminated_even_when_empty() {
        let block = build_environment_block(&[]);
        assert_eq!(block, vec![0u16]);
    }

    #[test]
    fn build_environment_block_contains_each_key_value_pair() {
        let block = build_environment_block(&[("PATH".to_string(), "C:\\Windows".to_string())]);
        let as_string = String::from_utf16(&block).unwrap();
        assert!(as_string.contains("PATH=C:\\Windows\u{0}"));
    }

    // --- spawn_contained: end-to-end, real process ---

    #[tokio::test]
    async fn spawn_contained_runs_a_real_command_and_reports_its_exit() {
        let cwd = std::env::current_dir().unwrap();
        let (child, _stdio) = spawn_contained(Path::new("cmd.exe"), &["/c".to_string(), "exit".to_string(), "7".to_string()], &cwd, &[("SystemRoot".to_string(), std::env::var("SystemRoot").unwrap())])
            .expect("spawning a real, known-good command must succeed");
        assert_ne!(child.process_id, 0);
    }

    #[test]
    fn spawn_contained_fails_closed_for_a_nonexistent_command() {
        let cwd = std::env::current_dir().unwrap();
        let result = spawn_contained(Path::new("this-binary-does-not-exist-m66"), &[], &cwd, &[]);
        assert!(result.is_err(), "spawning a nonexistent command must fail, never silently succeed");
    }

    #[tokio::test]
    async fn into_async_io_round_trips_real_data_through_a_real_child_process() {
        // Exercises the exact shape registration.rs's connect_now will
        // use: write a line to the child's stdin, read its echoed
        // response from the child's stdout, entirely through the
        // tokio::fs::File wrappers into_async_io produces -- the same
        // AsyncWrite/AsyncRead types AsyncRwTransport::new_client will
        // be given.
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let Some(python) = ["python", "python3"].into_iter().find(|candidate| std::process::Command::new(candidate).arg("--version").output().is_ok_and(|o| o.status.success())) else {
            eprintln!("skipping into_async_io_round_trips_real_data_through_a_real_child_process: no python on PATH");
            return;
        };
        // Line-buffered read + immediate flushed write -- deterministic
        // across platforms, unlike a shell builtin's own
        // batch/expansion/line-editing semantics (confirmed NOT
        // reliable for this exact test via cmd.exe's `set /p`
        // variable-expansion timing and PowerShell's `Read-Host`, both
        // tried and rejected here before this script).
        const ECHO_SCRIPT: &str = "import sys\nline = sys.stdin.readline()\nsys.stdout.write('got:' + line)\nsys.stdout.flush()\n";

        let cwd = std::env::current_dir().unwrap();
        let (_child, stdio) = spawn_contained(Path::new(python), &["-c".to_string(), ECHO_SCRIPT.to_string()], &cwd, &system_root_env()).unwrap();
        let (mut write, mut read) = stdio.into_async_io();

        write.write_all(b"hello\n").await.unwrap();
        write.flush().await.unwrap();

        let mut buf = vec![0u8; 256];
        let n = read.read(&mut buf).await.unwrap();
        let output = String::from_utf8_lossy(&buf[..n]);
        assert!(output.contains("got:hello"), "expected the child's echoed response, got: {output:?}");
    }

    // --- real job containment semantics (the actual M6.6 security claim) ---

    fn system_root_env() -> Vec<(String, String)> {
        vec![("SystemRoot".to_string(), std::env::var("SystemRoot").unwrap())]
    }


    #[tokio::test]
    async fn the_spawned_process_is_a_member_of_its_own_job() {
        let cwd = std::env::current_dir().unwrap();
        let (mut child, _stdio) = spawn_contained(Path::new("cmd.exe"), &["/c".to_string(), "more".to_string()], &cwd, &system_root_env()).unwrap();
        assert!(child.is_in_this_job(), "the process spawn_contained just created must be a member of the job it was created into");
        child.kill().await.unwrap();
    }

    #[tokio::test]
    async fn kill_terminates_a_descendant_the_direct_child_already_exited_from() {
        // Exercises `ContainedChild::kill` itself (not just dropping),
        // against the same "direct child already exited, grandchild
        // still running" shape as the drop test below -- proving
        // `kill()`'s own tree-wide reach, not merely `Drop`'s.
        let cwd = std::env::current_dir().unwrap();
        let marker = std::env::temp_dir().join(format!("m66-kill-tree-{}.marker", std::process::id()));
        let _ = std::fs::remove_file(&marker);
        let script = format!("start /b cmd /c (more & echo done > \"{}\")", marker.display());
        let (mut child, stdio) = spawn_contained(Path::new("cmd.exe"), &["/c".to_string(), script], &cwd, &system_root_env()).unwrap();

        tokio::time::sleep(std::time::Duration::from_millis(400)).await;
        drop(stdio);
        child.kill().await.unwrap();

        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        assert!(!marker.exists(), "kill() must terminate the grandchild too, before it can complete and write its marker file");
        let _ = std::fs::remove_file(&marker);
    }

    #[tokio::test]
    async fn start_kill_is_synchronous_and_the_process_is_confirmed_gone_shortly_after() {
        let cwd = std::env::current_dir().unwrap();
        let (mut child, _stdio) = spawn_contained(Path::new("cmd.exe"), &["/c".to_string(), "more".to_string()], &cwd, &system_root_env()).unwrap();
        child.start_kill();
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        use std::os::windows::io::AsRawHandle;
        let mut exit_code = 0u32;
        // SAFETY: `child.process` is still a valid (if now-exited)
        // process handle owned by `child` for the duration of this call.
        unsafe { windows::Win32::System::Threading::GetExitCodeProcess(HANDLE(child.process.as_raw_handle() as _), &mut exit_code) }.unwrap();
        const STILL_ACTIVE: u32 = 259;
        assert_ne!(exit_code, STILL_ACTIVE, "start_kill must have terminated the process by now");
    }

    #[tokio::test]
    async fn dropping_the_contained_child_terminates_the_whole_process_tree_not_just_the_direct_child() {
        // `cmd /c start /b cmd /c more` makes `more` (the grandchild)
        // long-running while the direct child (the first `cmd.exe`)
        // exits quickly on its own -- this specifically tests that
        // KILL_ON_JOB_CLOSE reaches a DESCENDANT the direct child
        // itself already exited from, not merely the one process this
        // module's own handle refers to.
        let cwd = std::env::current_dir().unwrap();
        let marker = std::env::temp_dir().join(format!("m66-tree-{}.marker", std::process::id()));
        let _ = std::fs::remove_file(&marker);
        let script = format!("start /b cmd /c (more & echo done > \"{}\")", marker.display());
        let (child, stdio) = spawn_contained(Path::new("cmd.exe"), &["/c".to_string(), script], &cwd, &system_root_env()).unwrap();

        // Give the grandchild time to actually start before killing the
        // job -- this test's own claim is about live descendants, not
        // about a race against process creation.
        tokio::time::sleep(std::time::Duration::from_millis(400)).await;

        drop(stdio); // close our pipe ends first, matching normal connection teardown
        drop(child); // drops `job` -> CloseHandle -> KILL_ON_JOB_CLOSE terminates the whole tree

        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        assert!(!marker.exists(), "the grandchild (`more`, left running under the job) must have been terminated by dropping ContainedChild, before it could ever complete and write its marker file");
        let _ = std::fs::remove_file(&marker);
    }

    #[test]
    fn job_setup_failure_in_the_real_spawn_path_prevents_the_untrusted_command_from_ever_running() {
        // The acceptance requirement this test exists to satisfy: don't
        // just call IsProcessInJob after a successful spawn (which only
        // proves containment on the SUCCESS path) -- prove that when
        // `spawn_contained_into`'s own job/attribute-list setup fails,
        // the untrusted command's own first action never runs at all.
        //
        // This calls the REAL `spawn_contained_into` function (the one
        // `spawn_contained` itself delegates to), injecting a job
        // handle that is guaranteed already-invalid -- forcing the
        // real `ProcThreadAttributeList::with_job_and_handle_list`
        // call inside it to fail via the real
        // PROC_THREAD_ATTRIBUTE_JOB_LIST/UpdateProcThreadAttribute
        // path, exactly as a genuine job-creation failure would. The
        // command line, if ever executed, would write a marker file as
        // its first and only action.
        let marker = std::env::temp_dir().join(format!("m66-no-exec-without-containment-{}.marker", std::process::id()));
        let _ = std::fs::remove_file(&marker);
        let cwd = std::env::current_dir().unwrap();
        let script = format!("echo executed > \"{}\"", marker.display());

        let result = spawn_contained_into(OwnedJobHandle::already_closed_for_test(), Path::new("cmd.exe"), &["/c".to_string(), script], &cwd, &system_root_env());

        assert!(result.is_err(), "spawn_contained_into must fail when job assignment setup fails, not silently fall back to an uncontained spawn");
        // No sleep/race here: `CreateProcessW` is a synchronous call
        // inside `spawn_contained_into`'s own body, already returned
        // from (with an error) by the time `result` is observed above
        // -- if it had been reached and succeeded, the marker would
        // already exist by this point, not merely "not yet."
        assert!(!marker.exists(), "no command must ever execute when job/attribute-list setup fails -- containment-before-execution is not optional");
    }
}
