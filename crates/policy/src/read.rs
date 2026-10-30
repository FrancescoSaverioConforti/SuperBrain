//! Local read policy supplied by the trusted single-user host, not a network
//! authentication mechanism or an authorization inferred from retrieved text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadOperation {
    Search,
    Retrieve,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ReadPolicy {
    search_allowed: bool,
    retrieve_allowed: bool,
}

impl ReadPolicy {
    pub fn new(search_allowed: bool, retrieve_allowed: bool) -> Self {
        Self {
            search_allowed,
            retrieve_allowed,
        }
    }
    pub fn allows(&self, operation: ReadOperation) -> bool {
        match operation {
            ReadOperation::Search => self.search_allowed,
            ReadOperation::Retrieve => self.retrieve_allowed,
        }
    }
}
