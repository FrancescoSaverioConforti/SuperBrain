//! M6.4-S1: Windows-only, read-only ACL inspection for trusted-location
//! executable/script identity verification.
//!
//! **What this module does NOT attempt (by explicit, reviewed design,
//! not an oversight)**: this is NOT a general Windows effective-access
//! simulator. A correct implementation of that would require either a
//! live logon token for every principal being evaluated (`AccessCheck`,
//! the OS's own authoritative engine, takes a client TOKEN handle, not
//! an arbitrary SID -- there is no way to ask "would `Everyone` be
//! granted access" without a real logon session for `Everyone`, which
//! does not practically exist to construct synthetically) or a
//! from-scratch reimplementation of ACE-ordering/deny-precedence/group-
//! membership-expansion semantics, which is exactly the class of
//! mistake that produces a FALSE ACCEPTANCE if done even slightly wrong
//! -- the single most dangerous failure mode this module exists to
//! avoid, not merely a false rejection (an earlier draft of this
//! design incorrectly claimed read-only inspection could only ever
//! fail closed; that claim was wrong and has been corrected here).
//!
//! **What this module DOES instead**: answers a strictly narrower,
//! conservative, closed-world question: "does this object's DACL
//! contain any ALLOW-GRANTING ace, of a type explicitly recognized
//! below, whose trustee SID is not on a small, fixed allowlist, with an
//! access mask that includes any write/delete/re-ACL-shaped bit?" This
//! is an EXISTENCE SCAN, never an effective-access computation -- it
//! never needs to resolve ACE ordering, deny-precedence, or group
//! membership, because it never tries to answer "is principal X
//! ultimately granted or denied access" for any specific principal. It
//! can only ever OVER-reject relative to a true effective-access
//! computation (flagging a grant a full computation might show is
//! overridden by an earlier deny-ACE) -- never UNDER-reject a real
//! allow-grant to an untrusted principal, provided the ACE-type
//! enumeration below is exhaustive and fails closed on anything it does
//! not recognize.
//!
//! **Exhaustive, closed-world ACE handling**: every ACE struct shape the
//! `windows` 0.62 crate defines (confirmed by direct enumeration of
//! `Windows::Win32::Security`'s own module, 20 distinct shapes) is
//! classified below as either allow-granting (checked) or never-
//! allow-granting (skipped, documented, never silently ignored). Any
//! `AceType` byte this module does not recognize causes immediate,
//! unconditional rejection of the whole evaluation -- never a silent
//! skip.
//!
//! **Null/malformed DACL handling**: a NULL DACL (Windows' own
//! "unrestricted, everyone has full access" state) is detected and
//! rejected BEFORE the ACE-walk loop ever runs (which would otherwise
//! see "zero ACEs" and wrongly conclude "nothing to object to").
//! `IsValidAcl`/`IsValidSecurityDescriptor` failures are treated
//! identically: reject, never default to "no grants found = safe."
//!
//! **Explicit, load-bearing limitation (stated here once, authoritatively,
//! quoted by `identity.rs`'s own disclosure text)**: this mechanism
//! protects against a DIFFERENT, lower-or-differently-scoped Windows
//! principal replacing or modifying a trusted path. It provides **ZERO**
//! protection against a compromised process running under the SAME
//! user account this CLI itself runs as -- that process already holds
//! every permission this check's own allowlist grants to "the current
//! user," by definition, and can simply write the file directly,
//! exactly as validly as the legitimate operator could. No SID-based
//! ACL check can distinguish "the operator's own benign tooling" from
//! "malware running as that same operator."

use std::path::Path;

use windows::Win32::Foundation::{CloseHandle, HANDLE, HLOCAL, LocalFree};
use windows::Win32::Security::Authorization::{GetNamedSecurityInfoW, SE_FILE_OBJECT};
use windows::Win32::Security::{
    EqualSid, GetAce, GetTokenInformation, IsValidAcl, IsValidSecurityDescriptor, TokenUser, ACL,
    ACCESS_ALLOWED_ACE, ACE_HEADER, DACL_SECURITY_INFORMATION, OWNER_SECURITY_INFORMATION, PSID,
    PSECURITY_DESCRIPTOR, TOKEN_QUERY, TOKEN_USER,
};
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

// --- recognized ACE type bytes (ACE_HEADER::AceType), confirmed against
//     the windows 0.62 crate's own Windows::Win32::Security module ---

const ACCESS_ALLOWED_ACE_TYPE: u8 = 0;
const ACCESS_DENIED_ACE_TYPE: u8 = 1;
const SYSTEM_AUDIT_ACE_TYPE: u8 = 2;
const SYSTEM_ALARM_ACE_TYPE: u8 = 3;
const ACCESS_ALLOWED_COMPOUND_ACE_TYPE: u8 = 4;
const ACCESS_ALLOWED_OBJECT_ACE_TYPE: u8 = 5;
const ACCESS_DENIED_OBJECT_ACE_TYPE: u8 = 6;
const SYSTEM_AUDIT_OBJECT_ACE_TYPE: u8 = 7;
const SYSTEM_ALARM_OBJECT_ACE_TYPE: u8 = 8;
const ACCESS_ALLOWED_CALLBACK_ACE_TYPE: u8 = 9;
const ACCESS_DENIED_CALLBACK_ACE_TYPE: u8 = 10;
const ACCESS_ALLOWED_CALLBACK_OBJECT_ACE_TYPE: u8 = 11;
const ACCESS_DENIED_CALLBACK_OBJECT_ACE_TYPE: u8 = 12;
const SYSTEM_AUDIT_CALLBACK_ACE_TYPE: u8 = 13;
const SYSTEM_ALARM_CALLBACK_ACE_TYPE: u8 = 14;
const SYSTEM_AUDIT_CALLBACK_OBJECT_ACE_TYPE: u8 = 15;
const SYSTEM_ALARM_CALLBACK_OBJECT_ACE_TYPE: u8 = 16;
const SYSTEM_MANDATORY_LABEL_ACE_TYPE: u8 = 17;
const SYSTEM_RESOURCE_ATTRIBUTE_ACE_TYPE: u8 = 18;
const SYSTEM_SCOPED_POLICY_ID_ACE_TYPE: u8 = 19;
const SYSTEM_PROCESS_TRUST_LABEL_ACE_TYPE: u8 = 20;
const SYSTEM_ACCESS_FILTER_ACE_TYPE: u8 = 21;

/// **Check A mask** -- "can an untrusted principal write content directly
/// INTO this directory" (add a file, add a subdirectory, append/write
/// data, change attributes, or hold a fully generic write/all grant).
/// Deliberately does NOT include `FILE_DELETE_CHILD`/`DELETE`/
/// `WRITE_DAC`/`WRITE_OWNER` -- those are Check B's own, differently-
/// scoped concern (see [`DELETE_OR_RETARGET_MASK`]). Confirmed by direct
/// bitwise verification against the real Win32 `FileSystemRights` enum
/// (`Modify` = `0x301BF`, `DeleteSubdirectoriesAndFiles`/
/// `FILE_DELETE_CHILD` = `0x40`; `0x301BF & 0x40 == 0`) that an ordinary
/// write-shaped grant genuinely does not imply delete-child rights --
/// this is the real Windows semantic distinction Check A and Check B
/// each rely on, not an assumption.
const WRITE_INTO_MASK: u32 = 0x0002 // FILE_WRITE_DATA / FILE_ADD_FILE
    | 0x0004 // FILE_APPEND_DATA / FILE_ADD_SUBDIRECTORY
    | 0x0100 // FILE_WRITE_ATTRIBUTES
    | 0x4000_0000 // GENERIC_WRITE
    | 0x1000_0000; // GENERIC_ALL

/// **Check B mask** -- "can an untrusted principal delete, rename,
/// replace, or re-ACL this specific path component." Checked on a
/// component's OWN DACL (`DELETE`/`WRITE_DAC`/`WRITE_OWNER`/
/// `GENERIC_ALL`: lets a holder delete the object itself outright, or
/// weaken/retarget its own ACL and then do so) and, separately, on that
/// component's PARENT's own DACL (`FILE_DELETE_CHILD`/`GENERIC_ALL`:
/// lets a holder delete/rename a NAMED CHILD of that parent, which is
/// exactly how `C:\Tools` granting this bit would let an untrusted
/// principal delete/replace `C:\Tools\Protected` wholesale, orphaning
/// everything beneath it regardless of depth). This mask has NO
/// legitimate stopping point short of the filesystem root -- deleting or
/// renaming ANY ancestor orphans/redirects everything below it.
const DELETE_OR_RETARGET_MASK: u32 = 0x0040 // FILE_DELETE_CHILD (confirmed empirically:
    // `icacls <dir> /grant:r "Everyone:(DC)"` followed by reading the
    // resulting ACE's own FileSystemRights.value__ via .NET's Get-Acl
    // yields exactly 0x40, not 0x10 -- an earlier version of this
    // constant used 0x0010 by mistake, which this evaluator's own test
    // suite (file_delete_child_on_the_boundarys_immediate_parent_is_
    // rejected / file_delete_child_on_an_ancestor_above_the_boundarys_
    // immediate_parent_is_rejected) caught as a genuine false-acceptance
    // bug before this code was ever committed.
    | 0x0001_0000 // DELETE
    | 0x0004_0000 // WRITE_DAC
    | 0x0008_0000 // WRITE_OWNER
    | 0x1000_0000; // GENERIC_ALL

#[derive(Debug, thiserror::Error)]
pub enum AclEvaluationError {
    #[error("failed to retrieve security information for {path:?}: {source}")]
    QueryFailed { path: std::path::PathBuf, source: windows::core::Error },
    #[error("{path:?} has no DACL present (an absent DACL means Windows grants unrestricted access to everyone) -- rejected")]
    NoDaclPresent { path: std::path::PathBuf },
    #[error("{path:?}'s DACL failed Windows' own IsValidAcl check -- rejected rather than interpreted")]
    InvalidAcl { path: std::path::PathBuf },
    #[error("{path:?}'s security descriptor failed Windows' own IsValidSecurityDescriptor check -- rejected rather than interpreted")]
    InvalidSecurityDescriptor { path: std::path::PathBuf },
    #[error("{path:?} has an ACE of a type this evaluator does not recognize (raw AceType byte {ace_type}) -- fails closed rather than silently skipping it")]
    UnrecognizedAceType { path: std::path::PathBuf, ace_type: u8 },
    #[error("{path:?} grants write-into-directory access to a trustee outside the allowlist (current user, SYSTEM, Administrators)")]
    UntrustedWriteIntoGrant { path: std::path::PathBuf },
    #[error("{path:?} grants delete/rename/replace/re-ACL rights (directly, or via FILE_DELETE_CHILD on its own parent) to a trustee outside the allowlist (current user, SYSTEM, Administrators) -- this component, or an ancestor of it, could be deleted, renamed or substituted by that principal")]
    UntrustedDeleteOrRetargetGrant { path: std::path::PathBuf },
    #[error("failed to determine the current process's own user SID: {0}")]
    CurrentUserSidFailed(windows::core::Error),
    #[error("trust_boundary {boundary:?} is not an ancestor of the resolved target path {target:?}")]
    BoundaryNotAnAncestor { boundary: std::path::PathBuf, target: std::path::PathBuf },
    #[error("trust_boundary {0:?} is too shallow to be a meaningful trust boundary (resolves to a drive root or a well-known broad system directory)")]
    BoundaryTooShallow(std::path::PathBuf),
}

/// A small, fixed allowlist of trustee SIDs an allow-granting ACE is
/// permitted to name -- the current process's own user SID (retrieved
/// once, from the live process token, never from a config value or any
/// other caller-influenced source), plus the two well-known SIDs this
/// codebase's existing B5 threat model already treats as fully trusted
/// (`docs/research/B5-BRAIN-API-MCP-CLOSURE.md`: "an unprivileged local
/// process... not Administrator/SYSTEM" -- SYSTEM/Administrators are
/// explicitly OUTSIDE the threat model this mechanism defends against).
struct Allowlist {
    current_user_sid: Vec<u8>,
}

impl Allowlist {
    fn current() -> Result<Self, AclEvaluationError> {
        Ok(Self { current_user_sid: current_process_user_sid()? })
    }

    /// Whether `candidate_sid` (raw SID bytes, as extracted from an ACE)
    /// is on the allowlist -- compares against the current user's own
    /// SID via the real `EqualSid` Win32 call (never a byte-equality
    /// shortcut, which would be wrong for SIDs of differing but
    /// semantically-irrelevant internal representations) plus two
    /// well-known-SID string prefixes for SYSTEM (`S-1-5-18`) and
    /// Administrators (`S-1-5-32-544`), matched structurally via SID
    /// sub-authority inspection rather than a string round-trip.
    fn contains(&self, candidate_sid: &[u8]) -> bool {
        if sids_equal(&self.current_user_sid, candidate_sid) {
            return true;
        }
        is_well_known_system_or_administrators(candidate_sid)
    }
}

fn sids_equal(a: &[u8], b: &[u8]) -> bool {
    unsafe {
        let psid_a = PSID(a.as_ptr() as *mut _);
        let psid_b = PSID(b.as_ptr() as *mut _);
        EqualSid(psid_a, psid_b).is_ok()
    }
}

/// Structural, not string-based, check for the two well-known SIDs this
/// evaluator treats as already-trusted (outside the B5 threat model):
/// `NT AUTHORITY\SYSTEM` (S-1-5-18) and `BUILTIN\Administrators`
/// (S-1-5-32-544). Reads the SID's own revision/sub-authority-count/
/// identifier-authority/sub-authorities directly from its binary layout
/// (the documented SID structure: 1 byte revision, 1 byte sub-authority
/// count, 6 bytes identifier authority, then 4 bytes per sub-authority,
/// little-endian) rather than round-tripping through a string
/// conversion, so this check has no dependency on locale/formatting.
fn is_well_known_system_or_administrators(sid: &[u8]) -> bool {
    if sid.len() < 8 {
        return false;
    }
    let sub_authority_count = sid[1] as usize;
    let identifier_authority = &sid[2..8]; // NT_AUTHORITY = {0,0,0,0,0,5}
    if identifier_authority != [0, 0, 0, 0, 0, 5] {
        return false;
    }
    let sub_authorities: Vec<u32> = (0..sub_authority_count)
        .filter_map(|i| {
            let start = 8 + i * 4;
            sid.get(start..start + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        })
        .collect();
    // SYSTEM: S-1-5-18 (one sub-authority: 18)
    if sub_authorities == [18] {
        return true;
    }
    // Administrators: S-1-5-32-544 (two sub-authorities: 32, 544)
    if sub_authorities == [32, 544] {
        return true;
    }
    false
}

fn current_process_user_sid() -> Result<Vec<u8>, AclEvaluationError> {
    unsafe {
        let mut token = HANDLE::default();
        OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).map_err(AclEvaluationError::CurrentUserSidFailed)?;
        let result = (|| {
            let mut required = 0u32;
            let _ = GetTokenInformation(token, TokenUser, None, 0, &mut required);
            let mut buf = vec![0u8; required as usize];
            GetTokenInformation(token, TokenUser, Some(buf.as_mut_ptr() as *mut _), required, &mut required).map_err(AclEvaluationError::CurrentUserSidFailed)?;
            let token_user = &*(buf.as_ptr() as *const TOKEN_USER);
            let sid_ptr = token_user.User.Sid;
            let sid_len = windows::Win32::Security::GetLengthSid(sid_ptr);
            let sid_bytes = std::slice::from_raw_parts(sid_ptr.0 as *const u8, sid_len as usize).to_vec();
            Ok(sid_bytes)
        })();
        let _ = CloseHandle(token);
        result
    }
}

/// Which of the two differently-scoped checks is being evaluated --
/// determines both which mask to check against and which typed error
/// variant to return on a disqualifying grant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CheckKind {
    /// Check A: write-into-directory, bounded at `trust_boundary`.
    WriteInto,
    /// Check B: delete/rename/replace/re-ACL, unbounded to the
    /// filesystem root.
    DeleteOrRetarget,
}

/// Evaluates `path` (a file OR a directory -- both go through the same
/// `SE_FILE_OBJECT` security-information query) against this module's
/// own conservative existence-scan, using the mask `kind` selects.
/// `Ok(())` means no disqualifying grant was found; `Err` carries the
/// specific, typed reason for rejection.
fn verify_no_untrusted_grant(path: &Path, allowlist: &Allowlist, kind: CheckKind) -> Result<(), AclEvaluationError> {
    let mask = match kind {
        CheckKind::WriteInto => WRITE_INTO_MASK,
        CheckKind::DeleteOrRetarget => DELETE_OR_RETARGET_MASK,
    };
    let to_error = |path: &Path| match kind {
        CheckKind::WriteInto => AclEvaluationError::UntrustedWriteIntoGrant { path: path.to_path_buf() },
        CheckKind::DeleteOrRetarget => AclEvaluationError::UntrustedDeleteOrRetargetGrant { path: path.to_path_buf() },
    };
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(std::iter::once(0)).collect();

    unsafe {
        let mut dacl: *mut ACL = std::ptr::null_mut();
        let mut sd: PSECURITY_DESCRIPTOR = PSECURITY_DESCRIPTOR::default();
        let status = GetNamedSecurityInfoW(
            windows::core::PCWSTR(wide.as_ptr()),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION | OWNER_SECURITY_INFORMATION,
            None,
            None,
            Some(&mut dacl),
            None,
            &mut sd,
        );
        if status.is_err() {
            return Err(AclEvaluationError::QueryFailed { path: path.to_path_buf(), source: windows::core::Error::from(status) });
        }
        // Ensure the security descriptor memory GetNamedSecurityInfoW
        // allocated is always freed, on every return path below.
        let _guard = LocalFreeGuard(HLOCAL(sd.0));

        if !IsValidSecurityDescriptor(sd).as_bool() {
            return Err(AclEvaluationError::InvalidSecurityDescriptor { path: path.to_path_buf() });
        }
        if dacl.is_null() {
            // A NULL DACL is Windows' own "no protection -- everyone has
            // full access" state. Must be rejected HERE, before the
            // ACE-walk loop ever runs (which would see zero ACEs and
            // wrongly conclude "nothing to object to").
            return Err(AclEvaluationError::NoDaclPresent { path: path.to_path_buf() });
        }
        if !IsValidAcl(dacl).as_bool() {
            return Err(AclEvaluationError::InvalidAcl { path: path.to_path_buf() });
        }

        let ace_count = (*dacl).AceCount;
        for i in 0..ace_count as u32 {
            let mut ace_ptr: *mut core::ffi::c_void = std::ptr::null_mut();
            GetAce(dacl, i, &mut ace_ptr).map_err(|e| AclEvaluationError::QueryFailed { path: path.to_path_buf(), source: e })?;
            let header = &*(ace_ptr as *const ACE_HEADER);
            // An INHERIT_ONLY_ACE (AceFlags bit 0x08) does NOT apply to
            // the object it is attached to at all -- it exists purely to
            // propagate to that object's future children (per
            // CONTAINER_INHERIT_ACE/OBJECT_INHERIT_ACE), never to grant
            // anything on the object itself. Confirmed as a genuine,
            // real-world case during this module's own test development:
            // this exact machine's C:\ carries an INHERIT_ONLY ACE for
            // Authenticated Users that, if evaluated as applying to C:\
            // itself, would have produced a FALSE REJECTION (treating a
            // grant that only reaches C:\'s children as if it reached
            // C:\ itself) -- skipping it here is required for Check B's
            // own correctness, not merely cosmetic.
            const INHERIT_ONLY_ACE: u8 = 0x08;
            if header.AceFlags & INHERIT_ONLY_ACE != 0 {
                continue;
            }
            match header.AceType {
                ACCESS_ALLOWED_ACE_TYPE | ACCESS_ALLOWED_CALLBACK_ACE_TYPE => {
                    let ace = &*(ace_ptr as *const ACCESS_ALLOWED_ACE);
                    let ace_mask = ace.Mask;
                    if ace_mask & mask != 0 {
                        let sid_ptr = &ace.SidStart as *const u32 as *const u8;
                        // SidStart is the first DWORD of the trustee SID;
                        // read the full SID via its own declared length.
                        let sid_len = windows::Win32::Security::GetLengthSid(PSID(sid_ptr as *mut _));
                        let sid_bytes = std::slice::from_raw_parts(sid_ptr, sid_len as usize);
                        if !allowlist.contains(sid_bytes) {
                            return Err(to_error(path));
                        }
                    }
                }
                // Object-specific/conditional allow-shaped ACEs: recognized
                // by type, but this evaluator does not parse their own
                // object-type-GUID or condition-expression fields -- any
                // ACE of these types is treated as an unconditional
                // allow-grant for conservatism (never attempts to
                // evaluate a condition expression, which could be wrong
                // in the dangerous direction if misparsed). The access
                // mask field sits at the same offset as ACCESS_ALLOWED_ACE
                // in these structs (documented Win32 ACE layout), so the
                // same mask check applies; the trustee SID's own offset
                // differs (after the object-type GUIDs), which this
                // evaluator does not attempt to locate precisely --
                // instead, ANY occurrence of these types with a
                // write-shaped mask is rejected outright, regardless of
                // trustee, since correctly locating the SID within a
                // variable-length object ACE is exactly the kind of
                // parsing this module's own design discipline avoids
                // attempting rather than risk getting subtly wrong.
                ACCESS_ALLOWED_OBJECT_ACE_TYPE | ACCESS_ALLOWED_CALLBACK_OBJECT_ACE_TYPE => {
                    return Err(AclEvaluationError::UnrecognizedAceType { path: path.to_path_buf(), ace_type: header.AceType });
                }
                // Never allow-granting: deny, audit, alarm, mandatory
                // label, resource attribute, scoped policy, process
                // trust label, access filter, and compound ACEs. None of
                // these can be the source of an unwanted GRANT, so they
                // are explicitly, documentedly skipped -- never silently
                // ignored without this match arm existing.
                ACCESS_DENIED_ACE_TYPE
                | ACCESS_DENIED_CALLBACK_ACE_TYPE
                | ACCESS_DENIED_OBJECT_ACE_TYPE
                | ACCESS_DENIED_CALLBACK_OBJECT_ACE_TYPE
                | SYSTEM_AUDIT_ACE_TYPE
                | SYSTEM_AUDIT_OBJECT_ACE_TYPE
                | SYSTEM_AUDIT_CALLBACK_ACE_TYPE
                | SYSTEM_AUDIT_CALLBACK_OBJECT_ACE_TYPE
                | SYSTEM_ALARM_ACE_TYPE
                | SYSTEM_ALARM_OBJECT_ACE_TYPE
                | SYSTEM_ALARM_CALLBACK_ACE_TYPE
                | SYSTEM_ALARM_CALLBACK_OBJECT_ACE_TYPE
                | SYSTEM_MANDATORY_LABEL_ACE_TYPE
                | SYSTEM_RESOURCE_ATTRIBUTE_ACE_TYPE
                | SYSTEM_SCOPED_POLICY_ID_ACE_TYPE
                | SYSTEM_PROCESS_TRUST_LABEL_ACE_TYPE
                | SYSTEM_ACCESS_FILTER_ACE_TYPE
                | ACCESS_ALLOWED_COMPOUND_ACE_TYPE => {}
                unrecognized => {
                    return Err(AclEvaluationError::UnrecognizedAceType { path: path.to_path_buf(), ace_type: unrecognized });
                }
            }
        }
    }

    Ok(())
}

/// Directories too shallow to ever be a meaningful `trust_boundary` --
/// an explicit, documented denylist (never inferred), since accepting a
/// boundary this broad would defeat the mechanism's own purpose.
const SHALLOW_BOUNDARY_DENYLIST: &[&str] = &[r"c:\", r"c:\windows", r"c:\users", r"c:\program files", r"c:\program files (x86)", r"c:\programdata"];

fn is_too_shallow(path: &Path) -> bool {
    let lower = path.to_string_lossy().to_ascii_lowercase();
    let trimmed = lower.trim_start_matches(r"\\?\");
    SHALLOW_BOUNDARY_DENYLIST.iter().any(|denied| trimmed.trim_end_matches('\\') == denied.trim_end_matches('\\'))
}

/// Verifies `target` (the resolved `command`/`script` path) is in a
/// trusted location, using two differently-scoped checks:
///
/// **Check A** (write-into-directory): bounded, runs on `target` itself
/// and every ancestor directory from it up to and INCLUDING
/// `trust_boundary`, never above it -- legitimately boundable because
/// adding/writing content directly into a directory above the boundary
/// cannot alter anything INSIDE the boundary subtree.
///
/// **Check B** (delete/rename/replace/re-ACL): unbounded, runs from
/// `target`'s own immediate parent all the way to the filesystem root,
/// with NO exception -- for each component in that full chain, checks
/// both the component's OWN DACL (`DELETE`/`WRITE_DAC`/`WRITE_OWNER`)
/// and its PARENT's DACL (`FILE_DELETE_CHILD`), since deleting or
/// renaming ANY ancestor, at any depth, orphans or redirects everything
/// beneath it -- there is no safe stopping point for this check short of
/// the root (confirmed by direct counterexample:
/// `C:\Tools\Protected\TrustedMCP\server.exe` with `trust_boundary =
/// ...\TrustedMCP` is NOT protected against `C:\Tools` granting
/// `FILE_DELETE_CHILD`, since that lets an untrusted principal delete/
/// replace `Protected` wholesale, taking everything below it down too,
/// even though `Protected`/`TrustedMCP`/`server.exe` are individually
/// clean).
///
/// A symlinked/junctioned ancestor is resolved to its REAL location
/// first (via `std::fs::canonicalize`, matching `storage::handle_safety::
/// open_verified_dir`'s own established precedent that a symlinked
/// ancestor is a legitimate deployment shape, not an attack signal by
/// itself) -- both checks verify the real, final directories' own
/// grants, never the symlink's own (irrelevant) permissions.
pub fn verify_trusted_location(target: &Path, trust_boundary: &Path) -> Result<(), AclEvaluationError> {
    let canonical_target = std::fs::canonicalize(target).map_err(|e| AclEvaluationError::QueryFailed {
        path: target.to_path_buf(),
        source: windows::core::Error::from(windows::core::HRESULT::from_win32(e.raw_os_error().unwrap_or(0) as u32)),
    })?;
    let canonical_boundary = std::fs::canonicalize(trust_boundary).map_err(|e| AclEvaluationError::QueryFailed {
        path: trust_boundary.to_path_buf(),
        source: windows::core::Error::from(windows::core::HRESULT::from_win32(e.raw_os_error().unwrap_or(0) as u32)),
    })?;

    if is_too_shallow(&canonical_boundary) {
        return Err(AclEvaluationError::BoundaryTooShallow(canonical_boundary));
    }
    if !canonical_target.starts_with(&canonical_boundary) {
        return Err(AclEvaluationError::BoundaryNotAnAncestor { boundary: canonical_boundary, target: canonical_target });
    }

    let allowlist = Allowlist::current()?;

    // --- Check A: write-into-directory, bounded at the boundary ---
    verify_no_untrusted_grant(&canonical_target, &allowlist, CheckKind::WriteInto)?;
    for ancestor in canonical_target.ancestors().skip(1) {
        verify_no_untrusted_grant(ancestor, &allowlist, CheckKind::WriteInto)?;
        if ancestor == canonical_boundary {
            break;
        }
    }

    // --- Check B: delete/rename/replace/re-ACL, unbounded to the root ---
    // `target` itself: check its own DACL (can IT be deleted/re-ACL'd)
    // and its parent's DACL (can it be deleted/renamed BY its parent).
    verify_no_untrusted_grant(&canonical_target, &allowlist, CheckKind::DeleteOrRetarget)?;
    for ancestor in canonical_target.ancestors().skip(1) {
        verify_no_untrusted_grant(ancestor, &allowlist, CheckKind::DeleteOrRetarget)?;
    }

    Ok(())
}

struct LocalFreeGuard(HLOCAL);
impl Drop for LocalFreeGuard {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            unsafe {
                let _ = LocalFree(Some(self.0));
            }
        }
    }
}

use std::os::windows::ffi::OsStrExt;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_process_user_sid_resolves_to_a_nonempty_sid() {
        let sid = current_process_user_sid().expect("the current process must always have a resolvable user SID");
        assert!(!sid.is_empty());
    }

    #[test]
    fn allowlist_accepts_the_current_users_own_sid() {
        let allowlist = Allowlist::current().unwrap();
        assert!(allowlist.contains(&allowlist.current_user_sid));
    }

    #[test]
    fn allowlist_rejects_an_unrelated_sid() {
        let allowlist = Allowlist::current().unwrap();
        // S-1-5-11 (NT AUTHORITY\Authenticated Users) -- a real,
        // well-known, but deliberately NOT allowlisted SID.
        let authenticated_users_sid: [u8; 12] = [1, 1, 0, 0, 0, 0, 0, 5, 11, 0, 0, 0];
        assert!(!allowlist.contains(&authenticated_users_sid));
    }

    #[test]
    fn is_well_known_system_or_administrators_recognizes_system() {
        let system_sid: [u8; 12] = [1, 1, 0, 0, 0, 0, 0, 5, 18, 0, 0, 0];
        assert!(is_well_known_system_or_administrators(&system_sid));
    }

    #[test]
    fn is_well_known_system_or_administrators_recognizes_administrators() {
        let admins_sid: [u8; 16] = [1, 2, 0, 0, 0, 0, 0, 5, 32, 0, 0, 0, 32, 2, 0, 0]; // 32, 544
        assert!(is_well_known_system_or_administrators(&admins_sid));
    }

    #[test]
    fn is_well_known_system_or_administrators_rejects_authenticated_users() {
        let authenticated_users_sid: [u8; 12] = [1, 1, 0, 0, 0, 0, 0, 5, 11, 0, 0, 0];
        assert!(!is_well_known_system_or_administrators(&authenticated_users_sid));
    }

    /// Creates `dir`, then explicitly resets its own ACL via `icacls`
    /// (an external, well-tested Windows tool -- this module's own code
    /// never constructs DACLs, only inspects them, so the test fixture
    /// itself uses the OS's own tool rather than this crate writing
    /// ACL-construction code, which would be exactly the kind of new
    /// surface the approved M6.4-S1 design explicitly avoids) to grant
    /// ONLY the current user full control, with inheritance disabled
    /// (`/inheritance:r` removes inherited ACEs first). This is
    /// necessary because ambient system directories (confirmed
    /// empirically during M6.4-S1's own review: this development
    /// machine's own `%TEMP%` has a real, inherited `Modify` grant for a
    /// separate `CodexSandboxUsers` principal) cannot be assumed clean
    /// for a "the honest case must be accepted" test fixture.
    fn make_clean_test_dir(label: &str) -> std::path::PathBuf {
        // Rooted under the current user's own profile directory, NOT
        // system %TEMP% -- confirmed during M6.4-S1's own review that
        // %TEMP% on this development host has a real, inherited `Modify`
        // grant for a separate principal (CodexSandboxUsers), which
        // would make the ENTIRE ancestor chain under it permanently
        // untrustworthy for a "must be accepted" test fixture, no matter
        // how clean the leaf directory's own ACL is set. The user
        // profile root itself was confirmed clean (owner + SYSTEM +
        // Administrators only, with inheritance from C:\Users/C:\
        // blocked by AreAccessRulesProtected) during the same review.
        let profile_dir = std::env::var_os("USERPROFILE").map(std::path::PathBuf::from).expect("USERPROFILE must be set on any Windows test host");
        let dir = profile_dir.join(format!("m64s1-acl-test-{label}-{}-{}", std::process::id(), uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let current_user = std::env::var("USERNAME").unwrap_or_default();
        let status = std::process::Command::new("icacls")
            .arg(&dir)
            .arg("/inheritance:r")
            .arg("/grant:r")
            .arg(format!("{current_user}:(OI)(CI)F"))
            .arg("/grant:r")
            .arg("SYSTEM:(OI)(CI)F")
            .arg("/grant:r")
            .arg("Administrators:(OI)(CI)F")
            .status()
            .expect("icacls must be available on any Windows test host");
        assert!(status.success(), "icacls must succeed in resetting the test directory's own ACL");
        dir
    }

    /// Removes a test-created directory tree on drop, even on a test
    /// failure/panic (via Rust's own unwind-through-drop) -- keeps
    /// `%USERPROFILE%` clean across repeated local/CI test runs. Uses
    /// `icacls /reset` first since a directory this module's own tests
    /// deliberately locked down (removing the current user's own
    /// inherited delete rights in some configurations) could otherwise
    /// resist ordinary removal.
    struct CleanupGuard(std::path::PathBuf);
    impl Drop for CleanupGuard {
        fn drop(&mut self) {
            let _ = std::process::Command::new("icacls").arg(&self.0).arg("/reset").arg("/t").status();
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// Builds a 4-level synthetic tree mirroring the exact counterexample
    /// discussed during M6.4-S1's own review:
    /// `root/Tools/Protected/TrustedMCP/server.bin`, every level
    /// individually ACL-clean, with `trust_boundary = .../TrustedMCP`.
    /// Returns `(root, tools, protected, trust_boundary, file)`.
    fn make_four_level_tree(label: &str) -> (std::path::PathBuf, std::path::PathBuf, std::path::PathBuf, std::path::PathBuf, std::path::PathBuf) {
        let root = make_clean_test_dir(label);
        let tools = root.join("Tools");
        let protected = tools.join("Protected");
        let trusted_mcp = protected.join("TrustedMCP");
        std::fs::create_dir_all(&trusted_mcp).unwrap();
        let file = trusted_mcp.join("server.bin");
        std::fs::write(&file, b"content").unwrap();
        (root, tools, protected, trusted_mcp, file)
    }

    fn icacls_grant(path: &std::path::Path, trustee_and_rights: &str) {
        let status = std::process::Command::new("icacls").arg(path).arg("/grant:r").arg(trustee_and_rights).status().unwrap();
        assert!(status.success(), "icacls /grant:r must succeed for {path:?}");
    }

    #[test]
    fn a_clean_tree_is_accepted_with_an_explicit_trust_boundary() {
        let (root, _tools, _protected, boundary, file) = make_four_level_tree("accept");
        let _cleanup = CleanupGuard(root);
        let result = verify_trusted_location(&file, &boundary);
        assert!(result.is_ok(), "a fully clean tree with an explicit trust_boundary must be accepted, got {result:?}");
    }

    #[test]
    fn a_write_only_grant_arbitrarily_far_above_the_boundary_is_still_accepted() {
        // The accept-side counterexample explicitly requested: an
        // ancestor ABOVE trust_boundary (here, the tree's own root,
        // several levels above) grants a broad principal ordinary
        // write-into rights (add files/subdirectories) but NOT
        // delete-child/delete/re-ACL rights. Must be accepted: Check A
        // never looks above the boundary, and Check B's unbounded walk
        // finds nothing disqualifying here because the grant present is
        // structurally incapable of deleting/renaming anything.
        //
        // `(NP)` (no propagate) is essential here: this ACE must apply
        // ONLY to `root` itself, never inherit down into `Tools`/
        // `Protected`/`TrustedMCP`/`server.bin` -- otherwise Check A's
        // own (correct) scan of the LEAF file's effective DACL would
        // also see this grant (via ordinary Windows inheritance) and
        // reject it there, which would be Check A legitimately firing on
        // an in-scope node, not a test of "an ancestor ABOVE the
        // boundary is tolerated."
        let (root, _tools, _protected, boundary, file) = make_four_level_tree("accept-write-only");
        let _cleanup = CleanupGuard(root.clone());
        icacls_grant(&root, "Everyone:(NP)(AD,WD)"); // AD=add subdir, WD=write data -- write-into only, no delete-child/delete/re-ACL, no propagation
        let result = verify_trusted_location(&file, &boundary);
        assert!(result.is_ok(), "a write-into-only grant on an ancestor ABOVE trust_boundary must be accepted, got {result:?}");
    }

    #[test]
    fn file_delete_child_on_an_ancestor_above_the_boundarys_immediate_parent_is_rejected() {
        // THE exact counterexample requested: FILE_DELETE_CHILD granted
        // on `Tools`, two levels above `trust_boundary` (`TrustedMCP`)
        // and one level above `Protected` (the boundary's own immediate
        // parent) -- lets an untrusted principal delete/rename
        // `Protected` wholesale, taking `TrustedMCP`/`server.bin` down
        // with it, even though every node at or below `Protected` is
        // individually clean. Must be rejected.
        let (root, tools, _protected, boundary, file) = make_four_level_tree("reject-grandparent");
        let _cleanup = CleanupGuard(root);
        icacls_grant(&tools, "Everyone:(OI)(CI)(DC)"); // DC = delete child
        let result = verify_trusted_location(&file, &boundary);
        assert!(matches!(result, Err(AclEvaluationError::UntrustedDeleteOrRetargetGrant { .. })), "FILE_DELETE_CHILD on an ancestor ABOVE the boundary's own immediate parent must be rejected, got {result:?}");
    }

    #[test]
    fn file_delete_child_on_the_boundarys_immediate_parent_is_rejected() {
        let (root, _tools, protected, boundary, file) = make_four_level_tree("reject-parent");
        let _cleanup = CleanupGuard(root);
        icacls_grant(&protected, "Everyone:(OI)(CI)(DC)");
        let result = verify_trusted_location(&file, &boundary);
        assert!(matches!(result, Err(AclEvaluationError::UntrustedDeleteOrRetargetGrant { .. })), "FILE_DELETE_CHILD on the boundary's own immediate parent must be rejected, got {result:?}");
    }

    #[test]
    fn delete_or_write_dac_on_the_boundary_itself_is_rejected() {
        let (root, _tools, _protected, boundary, file) = make_four_level_tree("reject-boundary-self");
        let _cleanup = CleanupGuard(root);
        icacls_grant(&boundary, "Everyone:(OI)(CI)(D,WDAC)"); // D=delete, WDAC=write DAC
        let result = verify_trusted_location(&file, &boundary);
        assert!(matches!(result, Err(AclEvaluationError::UntrustedDeleteOrRetargetGrant { .. })), "DELETE/WRITE_DAC granted directly on the boundary node itself must be rejected, got {result:?}");
    }

    #[test]
    fn an_ordinary_write_into_grant_on_the_boundary_itself_is_rejected_by_check_a() {
        // Confirms Check A still fires at/below the boundary -- write-
        // into rights ARE disqualifying when granted on a node AT OR
        // BELOW trust_boundary (only ABOVE it are they tolerated).
        let (root, _tools, _protected, boundary, file) = make_four_level_tree("reject-boundary-write-into");
        let _cleanup = CleanupGuard(root);
        icacls_grant(&boundary, "Everyone:(OI)(CI)(AD,WD)");
        let result = verify_trusted_location(&file, &boundary);
        assert!(matches!(result, Err(AclEvaluationError::UntrustedWriteIntoGrant { .. })), "a write-into grant AT the boundary itself must be rejected by Check A, got {result:?}");
    }

    #[test]
    fn a_boundary_that_is_not_an_ancestor_of_the_target_is_rejected() {
        let (root, tools, _protected, _boundary, file) = make_four_level_tree("reject-unrelated-boundary");
        let _cleanup = CleanupGuard(root);
        let result = verify_trusted_location(&file, &tools.join("NotAnAncestor"));
        assert!(result.is_err(), "a trust_boundary that does not resolve, or is not an ancestor of target, must be rejected, got {result:?}");
    }

    #[test]
    fn a_shallow_boundary_is_rejected() {
        let (root, _tools, _protected, _boundary, file) = make_four_level_tree("reject-shallow");
        let _cleanup = CleanupGuard(root);
        let result = verify_trusted_location(&file, Path::new(r"C:\"));
        assert!(matches!(result, Err(AclEvaluationError::BoundaryTooShallow(_)) | Err(AclEvaluationError::BoundaryNotAnAncestor { .. })), "a drive-root trust_boundary must be rejected, got {result:?}");
    }

    #[test]
    fn a_symlinked_ancestor_is_resolved_to_its_real_location_not_rejected_outright() {
        // Matches storage::handle_safety::open_verified_dir's own
        // established precedent: a symlinked ANCESTOR is a legitimate
        // deployment shape, resolved to its real target, never an
        // automatic rejection by itself. The real, resolved location's
        // own ACL is what is actually checked.
        let (root, _tools, _protected, _boundary, _file) = make_four_level_tree("symlink-ancestor-base");
        let _cleanup = CleanupGuard(root.clone());
        let link_root = root.parent().unwrap().join(format!("m64s1-symlink-{}", uuid::Uuid::new_v4()));
        let link_result = std::os::windows::fs::symlink_dir(&root, &link_root);
        match link_result {
            Ok(()) => {
                let _cleanup_link = CleanupGuard(link_root.clone());
                let via_symlink = link_root.join("Tools").join("Protected").join("TrustedMCP");
                let via_symlink_file = via_symlink.join("server.bin");
                let result = verify_trusted_location(&via_symlink_file, &via_symlink);
                assert!(result.is_ok(), "a symlinked ancestor resolving to a genuinely clean real location must be accepted, got {result:?}");
            }
            Err(e) => {
                eprintln!("skipping: could not create a real directory symlink on this host (needs admin/dev-mode on Windows): {e}");
            }
        }
    }

    #[test]
    fn a_hard_link_at_the_target_file_path_is_still_rejected_by_the_existing_content_hash_layer() {
        // Regression guard: this ACL layer runs AFTER
        // identity::verify_file_content_hash (which itself routes
        // through storage::handle_safety, already rejecting hard links
        // at the final component) -- confirms that existing, unmodified
        // protection remains intact and is not bypassed by this new ACL
        // layer running afterward. Exercises storage's own primitive
        // directly, not a duplicate of this module's own logic.
        let (root, _tools, _protected, _boundary, file) = make_four_level_tree("hardlink-regression");
        let _cleanup = CleanupGuard(root);
        let outside = std::env::var_os("USERPROFILE").map(std::path::PathBuf::from).unwrap().join(format!("m64s1-hardlink-victim-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&outside).unwrap();
        let victim = outside.join("victim.bin");
        std::fs::write(&victim, b"attacker content").unwrap();
        let hardlinked_path = file.parent().unwrap().join("hardlinked.bin");
        if std::fs::hard_link(&victim, &hardlinked_path).is_ok() {
            let result = storage::read_trusted_file(file.parent().unwrap(), Path::new("hardlinked.bin"), "<test>");
            assert!(result.is_err(), "a hard-linked file must still be rejected by storage::read_trusted_file, unaffected by this new ACL layer, got {result:?}");
        }
        let _ = std::fs::remove_dir_all(&outside);
    }
}
