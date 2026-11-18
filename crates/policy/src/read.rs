//! Local read policy supplied by the trusted single-user host, not a network
//! authentication mechanism or an authorization inferred from retrieved text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadOperation {
    Search,
    Retrieve,
    /// B5.1 Task 16. Gates `brain_api::get_code_context`.
    GetCodeContext,
}

/// `get_code_context_allowed` is a separate, additive field — NOT a third
/// parameter on [`ReadPolicy::new`] — specifically so `new`'s existing
/// 2-argument call sites (16 of them across `crates/api`'s own tests and
/// `b3_*` example binaries, none of which `get_code_context` is relevant
/// to) keep compiling unchanged. `crates/api` stays exactly what the B5.0
/// contract says it is ("B1 callable core... nothing here modifies it");
/// changing `new`'s arity would have forced touching every one of those
/// call sites for a capability `api` itself has no concept of.
/// [`ReadPolicy::with_get_code_context`] is the builder that sets it;
/// every policy constructed via `new()` alone defaults to `false`
/// (fail-closed, same default `Default`/`new` have always given every
/// other field).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ReadPolicy {
    search_allowed: bool,
    retrieve_allowed: bool,
    get_code_context_allowed: bool,
}

impl ReadPolicy {
    pub fn new(search_allowed: bool, retrieve_allowed: bool) -> Self {
        Self {
            search_allowed,
            retrieve_allowed,
            get_code_context_allowed: false,
        }
    }

    /// Returns `self` with `get_code_context_allowed` set to `allowed`,
    /// leaving `search_allowed`/`retrieve_allowed` exactly as `new` set
    /// them. Additive builder, not a `new`-arity change — see the struct's
    /// own doc comment for why.
    pub fn with_get_code_context(mut self, allowed: bool) -> Self {
        self.get_code_context_allowed = allowed;
        self
    }

    pub fn allows(&self, operation: ReadOperation) -> bool {
        match operation {
            ReadOperation::Search => self.search_allowed,
            ReadOperation::Retrieve => self.retrieve_allowed,
            ReadOperation::GetCodeContext => self.get_code_context_allowed,
        }
    }
}
