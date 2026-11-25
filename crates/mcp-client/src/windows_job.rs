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
//! or an attribute list), so this module also creates the stdin/stdout
//! pipes itself (`CreatePipe`; stderr is an inherited duplicate of this
//! process's own real stderr handle instead of a pipe — see
//! `inheritable_stderr_duplicate`'s own doc comment, an M6.7 fix) and
//! restricts which handles the child inherits via the complementary
//! `PROC_THREAD_ATTRIBUTE_HANDLE_LIST` attribute — the child inherits
//! exactly its own stdin/stdout/stderr handles, never any other handle
//! open in this process, regardless of how many other inheritable
//! handles this process may hold.
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

use windows::Win32::Foundation::{CloseHandle, DUPLICATE_SAME_ACCESS, DuplicateHandle, HANDLE};
use windows::Win32::Security::SECURITY_ATTRIBUTES;
use windows::Win32::System::Console::{GetStdHandle, STD_ERROR_HANDLE};
use windows::Win32::System::JobObjects::{CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation, SetInformationJobObject};
#[cfg(test)]
use windows::Win32::System::JobObjects::IsProcessInJob;
use windows::Win32::System::Pipes::CreatePipe;
use windows::Win32::System::Threading::{
    CreateProcessW, DeleteProcThreadAttributeList, EXTENDED_STARTUPINFO_PRESENT, GetCurrentProcess, InitializeProcThreadAttributeList, LPPROC_THREAD_ATTRIBUTE_LIST,
    PROC_THREAD_ATTRIBUTE_HANDLE_LIST, PROC_THREAD_ATTRIBUTE_JOB_LIST, PROCESS_CREATION_FLAGS, PROCESS_INFORMATION, STARTF_USESTDHANDLES, STARTUPINFOEXW, STARTUPINFOW, UpdateProcThreadAttribute,
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

    /// A NULL `HANDLE` — never created by `CreateJobObjectW`, so its
    /// own `Drop`'s `CloseHandle` call on it is a documented, safe
    /// no-op (`CloseHandle(NULL)` fails closed with `ERROR_INVALID_
    /// HANDLE`, never affecting any real object), unlike closing a
    /// handle VALUE that used to be real and may since have been
    /// reused. Exists only so `start_kill` has a value to leave in
    /// place of a real job handle it has already closed itself,
    /// without ever constructing a second `OwnedJobHandle` that owns
    /// (and would re-close) that same real handle.
    fn placeholder() -> Self {
        Self(HANDLE::default())
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

/// M6.7 fix: returns an inheritable DUPLICATE of this process's own
/// real stderr handle (`GetStdHandle(STD_ERROR_HANDLE)`), matching
/// `std::process::Stdio::inherit()`'s actual semantic -- the child
/// writes directly to wherever THIS process's own stderr goes
/// (terminal, redirected file, etc.), never to a throwaway pipe with
/// no reader (the M6.7 finding this replaces: closing a pipe's parent-
/// side read end is NOT equivalent to inherit() and causes the
/// child's own stderr writes to fail/crash once its pipe buffer
/// fills). A duplicate, not the original, is marked inheritable and
/// handed to the child: `GetStdHandle`'s returned handle is NOT
/// guaranteed inheritable by itself, and mutating this process's own
/// real stderr handle's inheritability in place (rather than
/// duplicating) would be a wider, unwanted side effect reaching past
/// this one child's own setup.
fn inheritable_stderr_duplicate() -> io::Result<OwnedHandle> {
    // SAFETY: `GetStdHandle` returns a handle this process does not
    // own in the usual sense (closing it would be a process-wide
    // mistake) -- never passed to `OwnedHandle`/`CloseHandle` directly
    // here; only used as `DuplicateHandle`'s source.
    let real_stderr = unsafe { GetStdHandle(STD_ERROR_HANDLE) }.map_err(io::Error::from)?;
    let mut duplicate = HANDLE::default();
    // SAFETY: duplicates `real_stderr` within the current process
    // (`GetCurrentProcess()` for both source and target process
    // handles is the documented pattern for an intra-process
    // duplicate); `DUPLICATE_SAME_ACCESS` preserves the original's
    // access rights; `bInheritHandle: true` is the one property this
    // duplicate needs that the original is not guaranteed to have.
    // The resulting `duplicate` is a freshly owned handle this
    // function hands to its caller via `OwnedHandle`.
    unsafe { DuplicateHandle(GetCurrentProcess(), real_stderr, GetCurrentProcess(), &mut duplicate, 0, true, DUPLICATE_SAME_ACCESS) }.map_err(io::Error::from)?;
    // SAFETY: `duplicate` was just created above and is not yet owned
    // by anything else.
    unsafe { Ok(OwnedHandle::from_raw_handle(duplicate.0 as _)) }
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
        // M6.7 finding (fixed here): a plain `self.job = replacement`
        // assignment would DROP the old `self.job` value first (Rust's
        // own assignment semantics), running `OwnedJobHandle::drop` --
        // which calls `CloseHandle` AGAIN on the handle this function
        // already closed two lines below, a genuine double-close.
        // `CloseHandle` on an already-closed handle value is usually
        // harmless on Windows, but not always: if the handle VALUE has
        // already been reused by an unrelated `Create*`-style call in
        // the meantime (handle-value reuse is real and documented
        // Windows behavior), the second `CloseHandle` would close that
        // UNRELATED object instead -- a narrow but genuine
        // use-after-close / wrong-object-closed bug. `std::mem::
        // replace` swaps the fields without ever constructing then
        // dropping an intermediate "old self.job" value in the normal
        // way; `old_job` is explicitly forgotten below instead of
        // dropped, since its handle was already closed by this
        // function itself, deliberately, one line above.
        let old_job = std::mem::replace(&mut self.job, OwnedJobHandle::placeholder());
        // SAFETY: `old_job.raw()` is the valid job handle this
        // `ContainedChild` owned until the line above; closed here,
        // exactly once, as the actual kill action.
        let _ = unsafe { CloseHandle(old_job.raw()) };
        std::mem::forget(old_job); // already closed above; must not be closed again by its own Drop
        if let Ok(replacement) = OwnedJobHandle::create_with_kill_on_close() {
            self.job = replacement;
        }
        // If `replacement` creation failed, `self.job` is left as the
        // placeholder (an invalid handle, never created, so closing it
        // on eventual Drop is a safe no-op) rather than retrying
        // `CloseHandle` on the already-closed real handle again.
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
    // stdout: child writes to the write end (inheritable), parent
    // reads from the read end (not inheritable). This matches
    // `tokio::process::Command`'s own piped-stdio inheritance shape.
    //
    // Child's stderr: NOT a pipe at all -- an inheritable duplicate of
    // THIS process's own real stderr handle (M6.7 fix; see
    // `inheritable_stderr_duplicate`'s own doc comment for the bug
    // this replaces), matching `Stdio::inherit()`'s actual semantic.
    let (stdin_read, stdin_write) = create_pipe_pair(true, false)?;
    let (stdout_read, stdout_write) = create_pipe_pair(false, true)?;
    let stderr_inherited = inheritable_stderr_duplicate()?;

    let child_stdin = HANDLE(std::os::windows::io::AsRawHandle::as_raw_handle(&stdin_read) as _);
    let child_stdout = HANDLE(std::os::windows::io::AsRawHandle::as_raw_handle(&stdout_write) as _);
    let child_stderr = HANDLE(std::os::windows::io::AsRawHandle::as_raw_handle(&stderr_inherited) as _);
    let inherit_list = [child_stdin, child_stdout, child_stderr];

    // M6.7 finding (Critical, fixed here): `UpdateProcThreadAttribute`'s
    // own documentation (confirmed directly against Microsoft Learn)
    // states `lpValue` "must persist until the attribute list is
    // destroyed using DeleteProcThreadAttributeList" -- i.e. the
    // attribute list stores the POINTER itself, not a copy of the
    // value, and the OS reads through it again at `CreateProcessW`
    // time (and potentially until `DeleteProcThreadAttributeList`).
    // Passing `&job.raw()` directly (as this line previously did) took
    // the address of a TEMPORARY `HANDLE` that is already gone by the
    // time `CreateProcessW` runs -- a genuine use-after-free, not
    // merely defensive-style over-caution. `job_handle_value` is a
    // named local that lives for the rest of this function's scope,
    // past `CreateProcessW` and past `attrs`'s own `Drop`
    // (`DeleteProcThreadAttributeList`), closing that window.
    let job_handle_value = job.raw();
    let attrs = ProcThreadAttributeList::with_job_and_handle_list(&job_handle_value, &inherit_list)?;

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
    // The child-side handles (`stdin_read`, `stdout_write`, and the
    // stderr duplicate) are themselves duplicated again into the
    // child's own handle table by `CreateProcessW` (standard Windows
    // handle-inheritance semantics: the child gets its own copy, not
    // this process's copy); this process's own copies are closed here
    // so the parent does not keep the child's pipe ends open (which
    // would prevent e.g. EOF from ever being observed on the child's
    // stdin/stdout exit), and so the stderr duplicate this function
    // created does not leak.
    drop(stdin_read);
    drop(stdout_write);
    drop(stderr_inherited);

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
        let job_handle_value = job.raw();
        let attrs = ProcThreadAttributeList::with_job_and_handle_list(&job_handle_value, &handles);
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
        let job_handle_value = job.raw();
        let attrs = ProcThreadAttributeList::with_job_and_handle_list(&job_handle_value, &[]);
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

    // --- M6.7: real argv round-trip through a real spawned child ---
    // (quote_arg's own unit tests above check the ESCAPING algorithm in
    // isolation; these additionally confirm the escaped command line
    // ACTUALLY decodes back to the original argv inside a real Python
    // child process's own sys.argv -- M6.7's explicit requirement, since
    // a unit test of the encoder alone cannot catch a mismatch between
    // this module's own encoding convention and Python's/MSVCRT's real
    // decoding rules.)

    #[tokio::test]
    async fn real_argv_round_trip_through_a_spawned_child_for_adversarial_argument_shapes() {
        let Some(python) = ["python", "python3"].into_iter().find(|candidate| std::process::Command::new(candidate).arg("--version").output().is_ok_and(|o| o.status.success())) else {
            eprintln!("skipping real_argv_round_trip_through_a_spawned_child_for_adversarial_argument_shapes: no python on PATH");
            return;
        };
        const PRINT_ARGV_AS_JSON: &str = "import sys, json\nsys.stdout.write(json.dumps(sys.argv[1:]))\nsys.stdout.flush()\n";

        let cases: Vec<Vec<String>> = vec![
            vec!["simple".to_string()],
            vec!["has space".to_string()],
            vec!["trailing\\".to_string()],
            vec!["trailing\\\\".to_string()],
            vec!["a\"b".to_string()],
            vec!["\\\"escaped-quote-after-backslash".to_string()],
            vec!["".to_string()],
            vec!["unicode-héllo-日本語-🎉".to_string()],
            vec!["multiple".to_string(), "separate".to_string(), "args".to_string()],
            vec!["-x".to_string(), "--flag=value".to_string(), "/looks/like/an/option".to_string()],
            vec!["new\nline".to_string()],
            vec!["tab\ttab".to_string()],
            vec!["a b\\c\"d\\\\e".to_string()], // combined: space, backslash, quote, trailing-before-quote backslashes
        ];

        for case in cases {
            let cwd = std::env::current_dir().unwrap();
            let mut args = vec!["-c".to_string(), PRINT_ARGV_AS_JSON.to_string()];
            args.extend(case.iter().cloned());
            let (_child, stdio) = spawn_contained(Path::new(python), &args, &cwd, &system_root_env()).unwrap();
            let (_write, mut read) = stdio.into_async_io();

            use tokio::io::AsyncReadExt;
            let mut buf = Vec::new();
            let mut chunk = [0u8; 4096];
            loop {
                let n = read.read(&mut chunk).await.unwrap();
                if n == 0 {
                    break;
                }
                buf.extend_from_slice(&chunk[..n]);
            }
            let output = String::from_utf8_lossy(&buf);
            let decoded: Vec<String> = serde_json::from_str(&output).unwrap_or_else(|e| panic!("child's argv output was not valid JSON for case {case:?}: {e}, raw output: {output:?}"));
            assert_eq!(decoded, case, "argv did not round-trip correctly for case {case:?} -- got {decoded:?}");
        }
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

    // --- M6.7: environment isolation adversarial tests ---
    // (`build_environment_block`'s only real caller today,
    // `registration.rs`'s `child_environment_pairs`, reads values only
    // from this process's own trusted environment via a fixed
    // allowlist -- these cases are not reachable from an untrusted MCP
    // server today. Tested anyway per M6.7's explicit instruction, and
    // because a future caller could pass less-trusted values without
    // this function's own contract changing to say so.)

    #[test]
    fn build_environment_block_preserves_an_empty_value() {
        let block = build_environment_block(&[("EMPTY_VAR".to_string(), String::new())]);
        let as_string = String::from_utf16(&block).unwrap();
        assert!(as_string.contains("EMPTY_VAR=\u{0}"), "an empty value must still produce a KEY= entry, not be silently dropped: {as_string:?}");
    }

    #[test]
    fn build_environment_block_preserves_non_ascii_unicode_values() {
        let block = build_environment_block(&[("UNICODE_VAR".to_string(), "héllo wörld 日本語 🎉".to_string())]);
        let as_string = String::from_utf16(&block).unwrap();
        assert!(as_string.contains("UNICODE_VAR=héllo wörld 日本語 🎉\u{0}"), "non-ASCII values must survive the UTF-16 conversion unchanged: {as_string:?}");
    }

    #[test]
    fn build_environment_block_is_case_preserving_for_windows_style_variable_names() {
        // Windows env var names are conventionally case-insensitive
        // (e.g. `Path` vs `PATH` vs `path` are often treated as the
        // same variable by the OS/CRT), but this function itself must
        // not silently normalize case -- that decision belongs to its
        // caller (the allowlist), not to the block-building step.
        let block = build_environment_block(&[("Path".to_string(), "C:\\A".to_string()), ("PATH".to_string(), "C:\\B".to_string())]);
        let as_string = String::from_utf16(&block).unwrap();
        assert!(as_string.contains("Path=C:\\A\u{0}"), "mixed-case key must be preserved verbatim: {as_string:?}");
        assert!(as_string.contains("PATH=C:\\B\u{0}"), "both differently-cased entries must both be present (deduplication, if any, is the caller's job): {as_string:?}");
    }

    #[test]
    fn build_environment_block_handles_duplicate_keys_by_including_both_entries_verbatim() {
        // This function performs no deduplication itself -- if a
        // caller passes the same key twice, both entries land in the
        // block as-is (CreateProcessW's own documented behavior for a
        // malformed/duplicated environment block is implementation-
        // defined; this function's contract is "verbatim pass-through
        // of what the caller gives it," so this test pins THAT
        // contract rather than guessing at CreateProcessW's own
        // resolution order).
        let block = build_environment_block(&[("DUP".to_string(), "first".to_string()), ("DUP".to_string(), "second".to_string())]);
        let as_string = String::from_utf16(&block).unwrap();
        assert!(as_string.contains("DUP=first\u{0}"));
        assert!(as_string.contains("DUP=second\u{0}"));
    }

    #[test]
    fn build_environment_block_does_not_let_an_embedded_null_in_a_value_corrupt_the_block_structure() {
        // M6.7 adversarial case: a Rust `String` CAN contain an
        // embedded NUL byte (`\0` is valid UTF-8) even though this
        // function's only current real caller never produces one (its
        // values come from `std::env::var_os` on a fixed allowlist of
        // real OS environment variables, which cannot themselves
        // contain embedded NULs) -- tested anyway since this function
        // has no type-level guard against it. An embedded NUL in the
        // VALUE would prematurely terminate that one block entry one
        // NUL early, causing CreateProcessW's own environment-block
        // parser to interpret the remaining bytes of this entry as the
        // START of the NEXT entry -- shifting every subsequent
        // variable's effective name/value and potentially fabricating
        // an extra, attacker-shaped environment variable the caller
        // never intended to pass. Documents the CURRENT (unguarded)
        // behavior precisely so a future change either fixes this
        // deliberately or this test fails loudly rather than silently
        // drifting.
        let block = build_environment_block(&[("A".to_string(), "before\0after".to_string()), ("B".to_string(), "second".to_string())]);
        let as_string = String::from_utf16(&block).unwrap();
        // Split on NUL exactly as CreateProcessW's own parser would:
        // each NUL-separated piece is one "entry" (the final empty
        // piece is the double-NUL terminator).
        let entries: Vec<&str> = as_string.split('\u{0}').collect();
        // Current (unguarded) behavior produces FOUR pieces --
        // "A=before", "after", "B=second", "" -- not the clean
        // THREE-piece ("A=before\0after", "B=second", "") shape a
        // caller who wrote this value almost certainly intended. The
        // "after" fragment becomes its own malformed entry (no `=`,
        // so CreateProcessW's handling of it is implementation-
        // defined) instead of remaining part of A's value, and B's
        // real entry is pushed one position later than a correct
        // encoder would place it. This pins that the corruption is
        // real and structural, not merely "the raw bytes contain a
        // NUL somewhere."
        assert_eq!(entries, vec!["A=before", "after", "B=second", "", ""], "an embedded NUL in a value must not fracture the block into an extra malformed entry like this; got: {entries:?}");
    }

    #[test]
    fn build_environment_block_treats_an_embedded_equals_sign_in_a_value_as_part_of_the_value_not_a_second_key() {
        // A value containing `=` is syntactically fine for
        // CreateProcessW's own `KEY=VALUE` format (only the FIRST `=`
        // in an entry is the delimiter; Windows itself has env vars
        // like `=C:` carrying extra `=` characters) -- confirms this
        // function does not mis-split on an embedded `=` within the
        // value half.
        let block = build_environment_block(&[("CONNSTR".to_string(), "server=localhost;port=5432".to_string())]);
        let as_string = String::from_utf16(&block).unwrap();
        assert!(as_string.contains("CONNSTR=server=localhost;port=5432\u{0}"), "a value containing '=' must be preserved verbatim, not mis-split: {as_string:?}");
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

    // --- M6.7: handle inheritance adversarial test ---

    #[tokio::test]
    async fn an_artificially_inheritable_handle_open_in_the_parent_is_not_accessible_from_the_child() {
        // The actual PROC_THREAD_ATTRIBUTE_HANDLE_LIST claim this
        // module makes: the child inherits EXACTLY its own 3 stdio
        // handles, never any other inheritable handle this process
        // happens to hold open. Proves it adversarially: creates a
        // SEPARATE pipe pair with BOTH ends marked inheritable
        // (`create_pipe_pair(true, true)`-equivalent, simulating some
        // other inheritable handle this process has open for an
        // unrelated reason -- e.g. a leaked or forgotten inheritable
        // handle elsewhere in a real process), writes a secret into
        // it, then spawns a child via the REAL spawn_contained whose
        // script tries to read that exact handle VALUE (passed via an
        // allowlisted env var) using raw Win32 ReadFile through
        // ctypes. If the handle leaked through, the child reads the
        // secret; if PROC_THREAD_ATTRIBUTE_HANDLE_LIST is doing its
        // job, the child's own handle table has no entry at that
        // value and ReadFile fails.
        let Some(python) = ["python", "python3"].into_iter().find(|candidate| std::process::Command::new(candidate).arg("--version").output().is_ok_and(|o| o.status.success())) else {
            eprintln!("skipping an_artificially_inheritable_handle_open_in_the_parent_is_not_accessible_from_the_child: no python on PATH");
            return;
        };

        const SECRET: &str = "m67-handle-leak-check-secret";
        let (leak_read, leak_write) = create_pipe_pair(true, true).unwrap(); // BOTH ends inheritable -- the adversarial condition
        {
            use std::io::Write;
            let mut std_write = std::fs::File::from(leak_write);
            std_write.write_all(SECRET.as_bytes()).unwrap();
            std_write.flush().unwrap();
            // Keep std_write (and therefore the underlying handle)
            // open for the duration of spawn_contained below by NOT
            // dropping it here -- forget it deliberately so the
            // handle stays live exactly as a real "other inheritable
            // handle the process holds open" would, without this
            // test's own cleanup closing it before the child even
            // starts.
            std::mem::forget(std_write);
        }
        use std::os::windows::io::AsRawHandle;
        let leak_handle_value = leak_read.as_raw_handle() as isize;

        const PROBE_SCRIPT: &str = r#"
import ctypes, os, sys
handle = int(os.environ['M67_PROBE_HANDLE'])
buf = ctypes.create_string_buffer(256)
read = ctypes.c_ulong(0)
ok = ctypes.windll.kernel32.ReadFile(handle, buf, 256, ctypes.byref(read), None)
if ok:
    sys.stdout.write('LEAKED:' + buf.raw[:read.value].decode(errors='replace'))
else:
    sys.stdout.write('NOT_ACCESSIBLE:' + str(ctypes.windll.kernel32.GetLastError()))
sys.stdout.flush()
"#;
        let cwd = std::env::current_dir().unwrap();
        let mut env = system_root_env();
        env.push(("M67_PROBE_HANDLE".to_string(), leak_handle_value.to_string()));
        let (_child, stdio) = spawn_contained(Path::new(python), &["-c".to_string(), PROBE_SCRIPT.to_string()], &cwd, &env).unwrap();
        let (_write, mut read) = stdio.into_async_io();

        use tokio::io::AsyncReadExt;
        let mut buf = vec![0u8; 512];
        let n = read.read(&mut buf).await.unwrap();
        let output = String::from_utf8_lossy(&buf[..n]).into_owned();

        // Clean up the leaked-on-purpose read end now that the probe
        // has run.
        let _ = unsafe { CloseHandle(HANDLE(leak_handle_value as _)) };

        assert!(!output.contains(SECRET), "the child was able to read the parent's artificially-inheritable handle -- PROC_THREAD_ATTRIBUTE_HANDLE_LIST is not restricting inheritance as claimed: {output:?}");
        assert!(output.starts_with("NOT_ACCESSIBLE:"), "expected the probe to report the handle as not accessible, got: {output:?}");
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

    /// M6.7 finding: the earlier version of these two tree-containment
    /// tests used a `cmd /c start /b cmd /c (more & echo done > "...")`
    /// script. Directly confirmed (via a throwaway diagnostic test,
    /// using `tasklist` and checking the marker, both removed once the
    /// finding was established) that this exact script never actually
    /// launches `more.com` at all -- `cmd.exe`'s parser rejects the
    /// parenthesized-group-with-embedded-quotes shape ("La sintassi del
    /// nome del file... non è corretta" / invalid filename/path syntax,
    /// now visible on stderr only because M6.7 also fixed stderr
    /// inheritance -- it was previously silently swallowed by the
    /// now-removed throwaway-pipe bug, which is how this went
    /// undetected since M6.6). That means both tests were passing
    /// VACUOUSLY: "the grandchild's marker never appears" was trivially
    /// true because there never was a live grandchild to kill, not
    /// because kill actually reached one. Replaced with a Python-based
    /// script (`GRANDCHILD_SCRIPT`) that deterministically spawns a
    /// real grandchild and writes a START marker immediately so these
    /// tests can first assert the grandchild genuinely started before
    /// testing that killing the job terminates it.
    fn grandchild_script(start_marker: &std::path::Path, done_marker: &std::path::Path) -> String {
        format!(
            "import subprocess, sys, time\np = subprocess.Popen([sys.executable, '-c', 'import time; open(r{start:?}, \"w\").close(); time.sleep(30); open(r{done:?}, \"w\").close()'])\nprint(p.pid)\nsys.stdout.flush()\ntime.sleep(30)\n",
            start = start_marker.display().to_string(),
            done = done_marker.display().to_string(),
        )
    }

    #[tokio::test]
    async fn kill_terminates_a_descendant_the_direct_child_already_exited_from() {
        // Exercises `ContainedChild::kill` itself (not just dropping),
        // against a real grandchild process (see `grandchild_script`'s
        // own doc comment for the vacuous-test finding this replaces).
        let Some(python) = ["python", "python3"].into_iter().find(|candidate| std::process::Command::new(candidate).arg("--version").output().is_ok_and(|o| o.status.success())) else {
            eprintln!("skipping kill_terminates_a_descendant_the_direct_child_already_exited_from: no python on PATH");
            return;
        };
        let cwd = std::env::current_dir().unwrap();
        let pid = std::process::id();
        let start_marker = std::env::temp_dir().join(format!("m66-kill-tree-start-{pid}.marker"));
        let done_marker = std::env::temp_dir().join(format!("m66-kill-tree-done-{pid}.marker"));
        let _ = std::fs::remove_file(&start_marker);
        let _ = std::fs::remove_file(&done_marker);
        let script = grandchild_script(&start_marker, &done_marker);
        let (mut child, stdio) = spawn_contained(Path::new(python), &["-c".to_string(), script], &cwd, &system_root_env()).unwrap();

        // Confirm the grandchild genuinely started before testing
        // anything about killing it -- the exact gap the vacuous
        // version of this test had.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !start_marker.exists() && std::time::Instant::now() < deadline {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        assert!(start_marker.exists(), "the grandchild process never started -- this test cannot prove anything about killing it");

        drop(stdio);
        child.kill().await.unwrap();

        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        assert!(!done_marker.exists(), "kill() must terminate the grandchild before its own 30s sleep completes and it writes the done marker");
        let _ = std::fs::remove_file(&start_marker);
        let _ = std::fs::remove_file(&done_marker);
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
    async fn start_kill_called_twice_does_not_double_close_or_panic_m67_finding() {
        // M6.7 finding (fixed): the original `start_kill` did
        // `self.job = replacement` directly, which drops the OLD
        // `self.job` first (running its own `CloseHandle` again on a
        // handle this function had already closed one line above) --
        // a genuine double-close, narrowly exploitable as a
        // wrong-object-closed bug if the handle VALUE had been reused
        // by an unrelated Create*-style call in the meantime. Calling
        // start_kill() twice in a row exercises exactly this path a
        // second time; if the fix (std::mem::replace + explicit
        // mem::forget of the already-closed old job) were reverted,
        // this would be the double-close-on-an-arbitrary-handle-value
        // shape, not a hang or panic a test could otherwise catch --
        // so this test's main value is running real code through the
        // exact call sequence under a tool like a debug CRT handle
        // checker / Application Verifier, and documenting the
        // invariant for anyone reading or modifying this code later.
        let cwd = std::env::current_dir().unwrap();
        let (mut child, _stdio) = spawn_contained(Path::new("cmd.exe"), &["/c".to_string(), "more".to_string()], &cwd, &system_root_env()).unwrap();
        child.start_kill();
        child.start_kill(); // must not panic, double-close, or affect any unrelated handle
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        use std::os::windows::io::AsRawHandle;
        let mut exit_code = 0u32;
        unsafe { windows::Win32::System::Threading::GetExitCodeProcess(HANDLE(child.process.as_raw_handle() as _), &mut exit_code) }.unwrap();
        const STILL_ACTIVE: u32 = 259;
        assert_ne!(exit_code, STILL_ACTIVE, "the process must still be terminated after two start_kill() calls");
    }

    #[tokio::test]
    async fn dropping_the_contained_child_terminates_the_whole_process_tree_not_just_the_direct_child() {
        // See `grandchild_script`'s own doc comment for the vacuous-
        // test finding this replaces (the direct child here is the
        // Python interpreter itself, which stays alive for its own
        // 30s sleep -- this test's own claim is specifically about a
        // GRANDCHILD it spawned, reached only through job containment,
        // not through this module's own direct-child handle).
        let Some(python) = ["python", "python3"].into_iter().find(|candidate| std::process::Command::new(candidate).arg("--version").output().is_ok_and(|o| o.status.success())) else {
            eprintln!("skipping dropping_the_contained_child_terminates_the_whole_process_tree_not_just_the_direct_child: no python on PATH");
            return;
        };
        let cwd = std::env::current_dir().unwrap();
        let pid = std::process::id();
        let start_marker = std::env::temp_dir().join(format!("m66-drop-tree-start-{pid}.marker"));
        let done_marker = std::env::temp_dir().join(format!("m66-drop-tree-done-{pid}.marker"));
        let _ = std::fs::remove_file(&start_marker);
        let _ = std::fs::remove_file(&done_marker);
        let script = grandchild_script(&start_marker, &done_marker);
        let (child, stdio) = spawn_contained(Path::new(python), &["-c".to_string(), script], &cwd, &system_root_env()).unwrap();

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !start_marker.exists() && std::time::Instant::now() < deadline {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        assert!(start_marker.exists(), "the grandchild process never started -- this test cannot prove anything about dropping it");

        drop(stdio); // close our pipe ends first, matching normal connection teardown
        drop(child); // drops `job` -> CloseHandle -> KILL_ON_JOB_CLOSE terminates the whole tree

        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        assert!(!done_marker.exists(), "the grandchild (left running under the job) must have been terminated by dropping ContainedChild, before its own 30s sleep could complete and write the done marker");
        let _ = std::fs::remove_file(&start_marker);
        let _ = std::fs::remove_file(&done_marker);
    }

    #[tokio::test]
    async fn stderr_writes_do_not_silently_fail_or_hang_the_child_m67_finding() {
        // M6.7 finding: spawn_contained_into closes the parent-side read
        // end of the stderr pipe immediately (treating it as "like
        // Stdio::inherit()"), but that is NOT what Stdio::inherit()
        // does -- inherit() passes the PARENT's own real stderr handle
        // through, never a fresh pipe with no reader. Writing to a pipe
        // whose only reader has closed is a broken-pipe condition on
        // Windows. This test writes a large-ish amount to stderr (more
        // than one pipe buffer's worth, to force the OS to actually
        // notice there is no reader rather than buffering it away) and
        // asserts the child still exits cleanly with its intended exit
        // code -- if stderr writes block or fail the process, this
        // fails (reproducing the finding) until fixed.
        let cwd = std::env::current_dir().unwrap();
        let Some(python) = ["python", "python3"].into_iter().find(|candidate| std::process::Command::new(candidate).arg("--version").output().is_ok_and(|o| o.status.success())) else {
            eprintln!("skipping stderr_writes_do_not_silently_fail_or_hang_the_child_m67_finding: no python on PATH");
            return;
        };
        // Write ~128KB to stderr -- comfortably larger than the default
        // anonymous pipe buffer (commonly 4-64KB), so a blocked/broken
        // write would actually manifest rather than being masked by
        // buffering.
        const SCRIPT: &str = "import sys\nsys.stderr.write('e' * 131072)\nsys.stderr.flush()\nsys.exit(42)\n";
        let (child, _stdio) = spawn_contained(Path::new(python), &["-c".to_string(), SCRIPT.to_string()], &cwd, &system_root_env()).unwrap();

        use std::os::windows::io::AsRawHandle;
        let raw = child.process.as_raw_handle() as isize;
        let (tx, rx) = tokio::sync::oneshot::channel();
        std::thread::spawn(move || {
            let handle = HANDLE(raw as _);
            let wait = unsafe { windows::Win32::System::Threading::WaitForSingleObject(handle, 5000) };
            let mut code = 0u32;
            if wait != windows::Win32::Foundation::WAIT_TIMEOUT {
                let _ = unsafe { windows::Win32::System::Threading::GetExitCodeProcess(handle, &mut code) };
            }
            let _ = tx.send((wait, code));
        });
        let (wait_result, exit_code) = rx.await.unwrap();
        assert_ne!(wait_result, windows::Win32::Foundation::WAIT_TIMEOUT, "child hung writing >1 pipe buffer of stderr with no reader -- confirms the broken-pipe/blocking finding");
        assert_eq!(exit_code, 42, "child did not reach its intended exit(42); a stderr write error likely crashed it first, got exit code {exit_code}");
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
