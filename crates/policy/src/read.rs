//! Local read policy supplied by the trusted single-user host, not a network
//! authentication mechanism or an authorization inferred from retrieved text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadOperation {
    Search,
    Retrieve,
    /// B5.1 Task 16. Gates `brain_api::get_code_context`.
    GetCodeContext,
    /// B5.1 Task 17. Gates `brain_api::graph_query`.
    GraphQuery,
    /// B5.1 Task 17. Gates `brain_api::get_entity`.
    GetEntity,
    /// B5.1 Task 18. Gates `brain_api::get_previous_solution`.
    GetPreviousSolution,
}

/// `get_code_context_allowed`/`graph_query_allowed`/`get_entity_allowed`/
/// `get_previous_solution_allowed` are separate, additive fields — NOT
/// additional parameters on [`ReadPolicy::new`] — specifically so `new`'s
/// existing 2-argument call sites (16 of them across `crates/api`'s own
/// tests and `b3_*` example binaries, none of which these capabilities are
/// relevant to) keep compiling unchanged. `crates/api` stays exactly what
/// the B5.0 contract says it is ("B1 callable core... nothing here
/// modifies it"); changing `new`'s arity would have forced touching every
/// one of those call sites for capabilities `api` itself has no concept
/// of. [`ReadPolicy::with_get_code_context`]/[`ReadPolicy::with_graph_query`]/
/// [`ReadPolicy::with_get_entity`]/[`ReadPolicy::with_get_previous_solution`]
/// are the builders that set them; every policy constructed via `new()`
/// alone defaults all four to `false` (fail-closed, same default
/// `Default`/`new` have always given every other field).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ReadPolicy {
    search_allowed: bool,
    retrieve_allowed: bool,
    get_code_context_allowed: bool,
    graph_query_allowed: bool,
    get_entity_allowed: bool,
    get_previous_solution_allowed: bool,
}

impl ReadPolicy {
    pub fn new(search_allowed: bool, retrieve_allowed: bool) -> Self {
        Self {
            search_allowed,
            retrieve_allowed,
            get_code_context_allowed: false,
            graph_query_allowed: false,
            get_entity_allowed: false,
            get_previous_solution_allowed: false,
        }
    }

    /// Returns `self` with `get_code_context_allowed` set to `allowed`,
    /// leaving every other field exactly as `new` set them. Additive
    /// builder, not a `new`-arity change — see the struct's own doc
    /// comment for why.
    pub fn with_get_code_context(mut self, allowed: bool) -> Self {
        self.get_code_context_allowed = allowed;
        self
    }

    /// Returns `self` with `graph_query_allowed` set to `allowed`, leaving
    /// every other field exactly as `new` set them. Additive builder, same
    /// reasoning as [`ReadPolicy::with_get_code_context`].
    pub fn with_graph_query(mut self, allowed: bool) -> Self {
        self.graph_query_allowed = allowed;
        self
    }

    /// Returns `self` with `get_entity_allowed` set to `allowed`, leaving
    /// every other field exactly as `new` set them. Additive builder, same
    /// reasoning as [`ReadPolicy::with_get_code_context`].
    pub fn with_get_entity(mut self, allowed: bool) -> Self {
        self.get_entity_allowed = allowed;
        self
    }

    /// Returns `self` with `get_previous_solution_allowed` set to
    /// `allowed`, leaving every other field exactly as `new` set them.
    /// Additive builder, same reasoning as
    /// [`ReadPolicy::with_get_code_context`].
    pub fn with_get_previous_solution(mut self, allowed: bool) -> Self {
        self.get_previous_solution_allowed = allowed;
        self
    }

    pub fn allows(&self, operation: ReadOperation) -> bool {
        match operation {
            ReadOperation::Search => self.search_allowed,
            ReadOperation::Retrieve => self.retrieve_allowed,
            ReadOperation::GetCodeContext => self.get_code_context_allowed,
            ReadOperation::GraphQuery => self.graph_query_allowed,
            ReadOperation::GetEntity => self.get_entity_allowed,
            ReadOperation::GetPreviousSolution => self.get_previous_solution_allowed,
        }
    }
}
