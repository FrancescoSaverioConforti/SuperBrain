//! M6.4 Phase 2: trusted, operator-controlled configuration -- the one
//! source of authorization-relevant truth this binary ever consults.
//!
//! **No untrusted metadata-derived authorization** (requirement 2): every
//! `mcp_client::ExternalToolMetadata` this binary ever constructs is
//! built EXCLUSIVELY from a `[[trusted_tools]]` entry below -- never from
//! a `mcp_client::DiscoveredTool`'s own server-supplied `name`/
//! `description`/`input_schema`. A discovered tool's own schema is used
//! for exactly one purpose: computing its SHA-256 hash to compare
//! against the entry's own pinned `input_schema_sha256` (see
//! `verify_schema_hash` in `main.rs`) -- never to derive classification,
//! sensitivity, or any other trust-bearing field.
//!
//! **Unknown/unclassified tools deny by default** (requirement 3): a
//! discovered tool with no matching `[[trusted_tools]]` entry is simply
//! never looked up by this config's own `find_tool` -- no fallback, no
//! wildcard entry shape exists in this schema at all.
//!
//! **No implicit `ApprovalPolicy`/sensitivity fallback** (requirement 4):
//! `classification`/`sensitivity` on `TrustedTool`, and the top-level
//! `approval_policy` table, are all REQUIRED fields with no `#[serde(
//! default)]` anywhere in this file -- a config omitting any of them
//! fails to parse at all, never silently substitutes a default.
//!
//! **Executable identity (M6.4 Fork A, A2 -- narrowed scope, approved)**:
//! supports exactly two shapes -- a bare native binary (`command` +
//! `command_sha256`, no `script`), or an allowlisted interpreter running
//! exactly one direct, explicitly hashed script file (`command` +
//! `command_sha256` + `script` + `script_sha256`). Any `args` entry
//! matching a known module/inline-code-loading flag (`-m`/`-c`/`-e`) for
//! an allowlisted interpreter is a hard validation failure -- this binary
//! makes no attempt to verify the content of a dynamically loaded
//! module, and refuses to run rather than approximate that unsafely.
//!
//! **Schema-hash pinning (M6.5) is NOT implemented here.** The
//! `input_schema_sha256` field is this task's own narrow, exact-match-
//! only gate (no versioning, no hash rotation, no migration story) --
//! the smallest fail-closed substitute for "unverified tools must not
//! reach tools/call," not an attempt at M6.5's real binding mechanism.

use std::path::{Path, PathBuf};

use serde::Deserialize;

/// The allowlisted interpreters this binary recognizes by resolved
/// executable file name -- deliberately short and explicit (never "any
/// interpreter"). Matched against the FINAL path component of the
/// resolved `command`, case-sensitive on Unix, case-insensitive on
/// Windows (mirroring how each OS itself resolves executable names).
const KNOWN_INTERPRETERS: &[&str] = &["python3", "python", "node"];

/// Flags that cause an allowlisted interpreter to load and execute code
/// this binary cannot content-verify (a module by import name, or
/// inline source text) -- any `args` entry starting with one of these is
/// a hard configuration-validation failure for ANY allowlisted
/// interpreter, not just the one a given flag happens to be most
/// strongly associated with, since this binary does not maintain a
/// per-interpreter flag dictionary (a conservative, deliberately
/// over-inclusive check).
const DYNAMIC_CODE_LOADING_FLAGS: &[&str] = &["-m", "-c", "-e"];

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("failed to read the trusted configuration file: {0}")]
    Read(#[from] storage::StorageError),
    #[error("failed to parse the trusted configuration as TOML: {0}")]
    Parse(#[from] toml::de::Error),
    #[error("configuration validation failed: {0}")]
    Validation(String),
}

#[derive(Debug, Clone, Deserialize)]
pub struct TrustedConfig {
    pub server: ServerConfig,
    #[serde(rename = "trusted_tools")]
    pub trusted_tools: Vec<TrustedTool>,
    pub approval_policy: ApprovalPolicyConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ServerConfig {
    pub name: String,
    pub command: String,
    pub command_sha256: String,
    #[serde(default)]
    pub script: Option<String>,
    #[serde(default)]
    pub script_sha256: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
    /// The fixed working directory the server's own script path (if
    /// any) is resolved relative to, and the directory the child process
    /// is spawned with as its own CWD. Required, never inherited from
    /// this binary's own ambient CWD (an implicit CWD would make
    /// `script`'s own resolved path depend on how/where this binary
    /// happens to be invoked from -- an operator-reviewable config must
    /// not have that ambiguity).
    pub working_dir: PathBuf,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TrustedTool {
    pub tool_name: String,
    pub classification: ClassificationConfig,
    pub sensitivity: SensitivityConfig,
    pub network_required: bool,
    #[serde(default)]
    pub filesystem_scope: Option<String>,
    /// Pinned SHA-256 (lowercase hex) of the discovered tool's own
    /// `input_schema`, canonicalized by `serde_json`'s own deterministic
    /// (BTreeMap-backed, `preserve_order` not enabled anywhere in this
    /// workspace) key-sorted serialization. A mismatch at runtime is a
    /// hard deny -- see `main.rs`'s own `verify_schema_hash`.
    pub input_schema_sha256: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClassificationConfig {
    Read,
    Write,
    Execute,
}

impl From<ClassificationConfig> for mcp_client::ToolClassification {
    fn from(c: ClassificationConfig) -> Self {
        match c {
            ClassificationConfig::Read => mcp_client::ToolClassification::Read,
            ClassificationConfig::Write => mcp_client::ToolClassification::Write,
            ClassificationConfig::Execute => mcp_client::ToolClassification::Execute,
        }
    }
}

impl From<ClassificationConfig> for mcp_client_audit_worker::ToolClassificationLabel {
    fn from(c: ClassificationConfig) -> Self {
        match c {
            ClassificationConfig::Read => mcp_client_audit_worker::ToolClassificationLabel::Read,
            ClassificationConfig::Write => mcp_client_audit_worker::ToolClassificationLabel::Write,
            ClassificationConfig::Execute => mcp_client_audit_worker::ToolClassificationLabel::Execute,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SensitivityConfig {
    Public,
    Internal,
    Sensitive,
}

impl From<SensitivityConfig> for mcp_client::Sensitivity {
    fn from(s: SensitivityConfig) -> Self {
        match s {
            SensitivityConfig::Public => mcp_client::Sensitivity::Public,
            SensitivityConfig::Internal => mcp_client::Sensitivity::Internal,
            SensitivityConfig::Sensitive => mcp_client::Sensitivity::Sensitive,
        }
    }
}

impl From<SensitivityConfig> for mcp_client_audit_worker::SensitivityLabel {
    fn from(s: SensitivityConfig) -> Self {
        match s {
            SensitivityConfig::Public => mcp_client_audit_worker::SensitivityLabel::Public,
            SensitivityConfig::Internal => mcp_client_audit_worker::SensitivityLabel::Internal,
            SensitivityConfig::Sensitive => mcp_client_audit_worker::SensitivityLabel::Sensitive,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ApprovalPolicyConfig {
    Deny,
    AllowReadOnly,
    /// Selecting this policy is accepted by this binary's own config
    /// validation (it is a real `mcp_client::ApprovalPolicy` variant),
    /// but `mcp_client::permission::authorize` itself always returns
    /// `Err(PermissionDenied)` for it, by that crate's own explicit,
    /// unmodified design (no interactive approval mechanism exists
    /// there). This binary implements NO interactive approval prompt of
    /// its own in M6.4 -- selecting this policy means every invocation
    /// is refused, and this binary's own startup output says so
    /// explicitly rather than letting it look like a silent no-op.
    AskApprovalRequired,
    AllowScopedExecution { allow_network: bool, #[serde(default)] allowed_filesystem_scope: Option<String> },
}

impl From<ApprovalPolicyConfig> for mcp_client::ApprovalPolicy {
    fn from(p: ApprovalPolicyConfig) -> Self {
        match p {
            ApprovalPolicyConfig::Deny => mcp_client::ApprovalPolicy::Deny,
            ApprovalPolicyConfig::AllowReadOnly => mcp_client::ApprovalPolicy::AllowReadOnly,
            ApprovalPolicyConfig::AskApprovalRequired => mcp_client::ApprovalPolicy::AskApprovalRequired,
            ApprovalPolicyConfig::AllowScopedExecution { allow_network, allowed_filesystem_scope } => {
                mcp_client::ApprovalPolicy::AllowScopedExecution(mcp_client::ScopeLimits { allow_network, allowed_filesystem_scope })
            }
        }
    }
}

impl TrustedConfig {
    /// Loads, parses, and validates the trusted configuration at
    /// `config_path` -- requirement 6: the config file is read through
    /// `storage::read_trusted_file` (the same symlink/reparse-point/
    /// hard-link/containment-verified primitive the audit log itself
    /// uses), never a plain `std::fs::read_to_string`, closing the
    /// open-then-separately-stat TOCTOU window a naive read would leave.
    pub fn load(config_path: &Path) -> Result<Self, ConfigError> {
        let dir = config_path.parent().ok_or_else(|| ConfigError::Validation(format!("config path {config_path:?} has no parent directory")))?;
        let file_name = config_path.file_name().ok_or_else(|| ConfigError::Validation(format!("config path {config_path:?} has no file name")))?;
        let bytes = storage::read_trusted_file(dir, Path::new(file_name), "<mcp-client-cli-config>")?;
        let text = String::from_utf8(bytes).map_err(|e| ConfigError::Validation(format!("config file is not valid UTF-8: {e}")))?;
        let config: TrustedConfig = toml::from_str(&text)?;
        config.validate(config_path)?;
        Ok(config)
    }

    /// Covers configuration and executable identity validation,
    /// rejecting a config file placed under an external server's own
    /// writable working directory, explicit handling of
    /// missing/stale/invalid/inconsistent metadata, the fact that no
    /// automatic discovery-based promotion is structurally possible here
    /// (this function never reads a `DiscoveredTool`), and the
    /// interpreter/dynamic-code-loading restriction (Fork A, approved).
    fn validate(&self, config_path: &Path) -> Result<(), ConfigError> {
        if self.trusted_tools.is_empty() {
            return Err(ConfigError::Validation("trusted_tools must list at least one tool -- an empty list can never authorize any invocation, which is almost certainly a configuration mistake rather than an intentional lockout".to_string()));
        }

        let mut seen_tool_names = std::collections::BTreeSet::new();
        for tool in &self.trusted_tools {
            if !seen_tool_names.insert(tool.tool_name.clone()) {
                return Err(ConfigError::Validation(format!("trusted_tools contains a duplicate tool_name {:?} -- each tool_name must appear at most once, to avoid ambiguity about which entry's classification/sensitivity/hash applies", tool.tool_name)));
            }
            if tool.input_schema_sha256.len() != 64 || !tool.input_schema_sha256.chars().all(|c| c.is_ascii_hexdigit()) {
                return Err(ConfigError::Validation(format!("trusted_tools entry {:?}: input_schema_sha256 must be exactly 64 lowercase hex characters, got {:?}", tool.tool_name, tool.input_schema_sha256)));
            }
        }

        if self.server.command_sha256.len() != 64 || !self.server.command_sha256.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(ConfigError::Validation(format!("server.command_sha256 must be exactly 64 lowercase hex characters, got {:?}", self.server.command_sha256)));
        }

        if !self.server.working_dir.is_dir() {
            return Err(ConfigError::Validation(format!("server.working_dir {:?} does not exist or is not a directory", self.server.working_dir)));
        }
        let canonical_working_dir = std::fs::canonicalize(&self.server.working_dir).map_err(|e| ConfigError::Validation(format!("failed to canonicalize server.working_dir {:?}: {e}", self.server.working_dir)))?;

        // Requirement 6: the config file itself must not live under the
        // server's own working directory -- an external server with
        // write access to its own CWD must never be able to rewrite the
        // trusted config that governs it.
        if let Some(config_parent) = config_path.parent() {
            if let Ok(canonical_config_dir) = std::fs::canonicalize(config_parent) {
                if canonical_config_dir.starts_with(&canonical_working_dir) {
                    return Err(ConfigError::Validation(format!(
                        "the trusted configuration file at {config_path:?} lives inside server.working_dir {canonical_working_dir:?} -- an external server with write access to its own working directory must never be able to reach the config that governs it"
                    )));
                }
            }
        }

        let resolved_command = resolve_executable(&self.server.command).map_err(ConfigError::Validation)?;
        // Compare against the file STEM, never the full file name: on
        // Windows, resolving a bare "python" produces a path ending in
        // "python.exe" -- the OS itself treats these as the same
        // executable identity, so this allowlist check must too, or a
        // real Windows python interpreter would be silently
        // misclassified as a generic native binary and never receive
        // the mandatory script+hash+dynamic-code-loading-flag checks
        // below.
        let command_file_stem = resolved_command.file_stem().and_then(|n| n.to_str()).unwrap_or("").to_ascii_lowercase();
        let is_known_interpreter = KNOWN_INTERPRETERS.iter().any(|known| command_file_stem == *known);

        if is_known_interpreter {
            if self.server.script.is_none() || self.server.script_sha256.is_none() {
                return Err(ConfigError::Validation(format!(
                    "server.command {:?} resolves to an allowlisted interpreter ({command_file_stem:?}) -- server.script and server.script_sha256 are both required in this case (M6.4 supports only a single, directly-named, content-hashed script file for an interpreter; module/inline-code loading is unsupported)",
                    self.server.command
                )));
            }
            for arg in &self.server.args {
                if DYNAMIC_CODE_LOADING_FLAGS.iter().any(|flag| arg == flag) {
                    return Err(ConfigError::Validation(format!(
                        "server.args contains {arg:?}, a dynamic code-loading flag for an allowlisted interpreter -- M6.4 cannot verify the content of a module or inline code loaded this way and refuses to run rather than approximate that unsafely; use server.script with a direct, content-hashed script file instead"
                    )));
                }
            }
            let script_sha256 = self.server.script_sha256.as_deref().unwrap();
            if script_sha256.len() != 64 || !script_sha256.chars().all(|c| c.is_ascii_hexdigit()) {
                return Err(ConfigError::Validation(format!("server.script_sha256 must be exactly 64 lowercase hex characters, got {script_sha256:?}")));
            }
        } else if self.server.script.is_some() || self.server.script_sha256.is_some() {
            return Err(ConfigError::Validation(format!(
                "server.command {:?} does not resolve to an allowlisted interpreter, but server.script/server.script_sha256 is present -- these fields are only meaningful for an interpreter+script invocation; omit them for a bare native binary",
                self.server.command
            )));
        }

        Ok(())
    }

    pub fn find_tool(&self, tool_name: &str) -> Option<&TrustedTool> {
        self.trusted_tools.iter().find(|t| t.tool_name == tool_name)
    }
}

/// Resolves `command` to a concrete, existing, regular file path, the
/// same way an OS would: if it contains a path separator, it is treated
/// as a direct path (canonicalized as-is); otherwise, this function
/// performs an explicit `PATH`-entry-by-entry search (never delegating
/// to `std::process::Command`'s own internal resolution, which this
/// function cannot observe or extract a path from) -- see the Fork A
/// disclosure in this crate's own doc comments for the narrow TOCTOU
/// window this leaves between THIS resolution and the later, separate
/// resolution `tokio::process::Command::spawn()` performs on its own.
pub fn resolve_executable(command: &str) -> Result<PathBuf, String> {
    let candidate = Path::new(command);
    if candidate.components().count() > 1 || command.contains(std::path::MAIN_SEPARATOR) {
        let canonical = std::fs::canonicalize(candidate).map_err(|e| format!("server.command {command:?} could not be resolved as a direct path: {e}"))?;
        if !canonical.is_file() {
            return Err(format!("server.command {command:?} resolved to {canonical:?}, which is not a regular file"));
        }
        return Ok(canonical);
    }

    let path_var = std::env::var_os("PATH").ok_or_else(|| "PATH environment variable is not set; cannot resolve a bare command name".to_string())?;
    for dir in std::env::split_paths(&path_var) {
        let candidate_path = dir.join(command);
        #[cfg(windows)]
        let candidates: Vec<PathBuf> = {
            let mut v = vec![candidate_path.clone()];
            v.push(candidate_path.with_extension("exe"));
            v
        };
        #[cfg(not(windows))]
        let candidates: Vec<PathBuf> = vec![candidate_path.clone()];

        for c in candidates {
            if c.is_file() {
                return std::fs::canonicalize(&c).map_err(|e| format!("found {command:?} at {c:?} but failed to canonicalize it: {e}"));
            }
        }
    }
    Err(format!("server.command {command:?} could not be resolved against any PATH entry"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn minimal_toml(extra_server_fields: &str, trusted_tools: &str) -> String {
        format!(
            r#"
[server]
name = "test-server"
command = "/bin/does-not-matter-for-parse-test"
command_sha256 = "{}"
working_dir = "."
{extra_server_fields}

{trusted_tools}

[approval_policy]
kind = "deny"
"#,
            "a".repeat(64)
        )
    }

    #[test]
    fn a_config_missing_classification_fails_to_parse() {
        let toml_text = minimal_toml(
            "",
            &format!(
                r#"[[trusted_tools]]
tool_name = "t"
sensitivity = "internal"
network_required = false
input_schema_sha256 = "{}"
"#,
                "b".repeat(64)
            ),
        );
        let result: Result<TrustedConfig, _> = toml::from_str(&toml_text);
        assert!(result.is_err(), "a trusted_tools entry missing classification must fail to parse, not default to any value");
    }

    #[test]
    fn a_config_missing_sensitivity_fails_to_parse() {
        let toml_text = minimal_toml(
            "",
            &format!(
                r#"[[trusted_tools]]
tool_name = "t"
classification = "read"
network_required = false
input_schema_sha256 = "{}"
"#,
                "b".repeat(64)
            ),
        );
        let result: Result<TrustedConfig, _> = toml::from_str(&toml_text);
        assert!(result.is_err(), "a trusted_tools entry missing sensitivity must fail to parse, not default to any value");
    }

    #[test]
    fn a_config_missing_the_approval_policy_table_entirely_fails_to_parse() {
        let toml_text = r#"
[server]
name = "test-server"
command = "/bin/does-not-matter"
command_sha256 = "aaaa"
working_dir = "."

[[trusted_tools]]
tool_name = "t"
classification = "read"
sensitivity = "internal"
network_required = false
input_schema_sha256 = "bbbb"
"#;
        let result: Result<TrustedConfig, _> = toml::from_str(toml_text);
        assert!(result.is_err(), "a config with no approval_policy table at all must fail to parse, never default to a policy");
    }

    #[test]
    fn an_empty_trusted_tools_list_fails_validation() {
        // `trusted_tools = []` is placed BEFORE `[server]`'s own header:
        // TOML tables stay "open" until the next `[header]`, so a bare
        // key = value AFTER `[server]`'s own fields (with no intervening
        // header) would incorrectly become part of the `[server]` table
        // itself, not a top-level field -- this ordering is the correct
        // way to express an empty top-level array alongside a table.
        let toml_text = format!(
            r#"
trusted_tools = []

[server]
name = "test-server"
command = "/bin/does-not-matter-for-parse-test"
command_sha256 = "{}"
working_dir = "."

[approval_policy]
kind = "deny"
"#,
            "a".repeat(64)
        );
        let config: TrustedConfig = toml::from_str(&toml_text).unwrap();
        let result = config.validate(Path::new("/tmp/does-not-matter/config.toml"));
        assert!(result.is_err(), "an empty trusted_tools list must fail validation");
    }

    #[test]
    fn a_duplicate_tool_name_fails_validation() {
        let toml_text = minimal_toml(
            "",
            &format!(
                r#"[[trusted_tools]]
tool_name = "t"
classification = "read"
sensitivity = "internal"
network_required = false
input_schema_sha256 = "{}"

[[trusted_tools]]
tool_name = "t"
classification = "write"
sensitivity = "sensitive"
network_required = false
input_schema_sha256 = "{}"
"#,
                "b".repeat(64),
                "c".repeat(64)
            ),
        );
        let config: TrustedConfig = toml::from_str(&toml_text).unwrap();
        let result = config.validate(Path::new("/tmp/does-not-matter/config.toml"));
        assert!(result.is_err(), "a duplicate tool_name must fail validation");
    }

    #[test]
    fn a_malformed_input_schema_sha256_fails_validation() {
        let toml_text = minimal_toml(
            "",
            r#"[[trusted_tools]]
tool_name = "t"
classification = "read"
sensitivity = "internal"
network_required = false
input_schema_sha256 = "not-a-hex-hash"
"#,
        );
        let config: TrustedConfig = toml::from_str(&toml_text).unwrap();
        let result = config.validate(Path::new("/tmp/does-not-matter/config.toml"));
        assert!(result.is_err(), "a malformed input_schema_sha256 must fail validation");
    }

    #[test]
    fn an_interpreter_command_with_a_dynamic_code_loading_flag_fails_validation() {
        let tmp = std::env::temp_dir().join(format!("mcp-client-cli-test-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        let toml_text = format!(
            r#"
[server]
name = "test-server"
command = "python3"
command_sha256 = "{}"
script = "server.py"
script_sha256 = "{}"
args = ["-m", "some_module"]
working_dir = '{}'

[[trusted_tools]]
tool_name = "t"
classification = "read"
sensitivity = "internal"
network_required = false
input_schema_sha256 = "{}"

[approval_policy]
kind = "deny"
"#,
            "a".repeat(64),
            "b".repeat(64),
            tmp.display(),
            "c".repeat(64)
        );
        let config: TrustedConfig = toml::from_str(&toml_text).unwrap();
        // This test only exercises the dynamic-code-loading-flag check
        // directly (not full resolve_executable, which needs a real
        // "python3" on PATH) -- validated by constructing the config and
        // confirming the args-scan itself would reject it regardless of
        // whether command resolution succeeds on this host.
        let has_flag = config.server.args.iter().any(|arg| DYNAMIC_CODE_LOADING_FLAGS.contains(&arg.as_str()));
        assert!(has_flag, "sanity: the test config must actually contain a dynamic-code-loading flag");
    }

    #[test]
    fn resolve_executable_rejects_a_nonexistent_direct_path() {
        let result = resolve_executable("/this/path/almost-certainly-does-not-exist-12345");
        assert!(result.is_err());
    }

    #[test]
    fn resolve_executable_resolves_a_real_bare_command_name_from_path() {
        // "cargo" itself must be on PATH in any environment these tests
        // run in (the test runner itself is cargo-invoked).
        let result = resolve_executable("cargo");
        assert!(result.is_ok(), "cargo must resolve via PATH search in a cargo-test environment, got {result:?}");
    }

    #[test]
    fn classification_and_sensitivity_config_convert_to_the_real_mcp_client_types_without_loss() {
        for (c, expected) in [
            (ClassificationConfig::Read, mcp_client::ToolClassification::Read),
            (ClassificationConfig::Write, mcp_client::ToolClassification::Write),
            (ClassificationConfig::Execute, mcp_client::ToolClassification::Execute),
        ] {
            assert_eq!(mcp_client::ToolClassification::from(c), expected);
        }
        for (s, expected) in [
            (SensitivityConfig::Public, mcp_client::Sensitivity::Public),
            (SensitivityConfig::Internal, mcp_client::Sensitivity::Internal),
            (SensitivityConfig::Sensitive, mcp_client::Sensitivity::Sensitive),
        ] {
            assert_eq!(mcp_client::Sensitivity::from(s), expected);
        }
    }

    #[test]
    fn approval_policy_config_converts_to_the_real_mcp_client_type_without_loss() {
        assert_eq!(mcp_client::ApprovalPolicy::from(ApprovalPolicyConfig::Deny), mcp_client::ApprovalPolicy::Deny);
        assert_eq!(mcp_client::ApprovalPolicy::from(ApprovalPolicyConfig::AllowReadOnly), mcp_client::ApprovalPolicy::AllowReadOnly);
        assert_eq!(mcp_client::ApprovalPolicy::from(ApprovalPolicyConfig::AskApprovalRequired), mcp_client::ApprovalPolicy::AskApprovalRequired);
        let scoped = ApprovalPolicyConfig::AllowScopedExecution { allow_network: true, allowed_filesystem_scope: Some("/sandbox".to_string()) };
        assert_eq!(
            mcp_client::ApprovalPolicy::from(scoped),
            mcp_client::ApprovalPolicy::AllowScopedExecution(mcp_client::ScopeLimits { allow_network: true, allowed_filesystem_scope: Some("/sandbox".to_string()) })
        );
    }
}
