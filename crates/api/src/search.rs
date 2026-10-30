use crate::{check, response, ApiError, ApiResponse, RequestContext};
use index::{
    search::{SearchFilters, SearchResponse},
    Index,
};

#[derive(Debug, Clone)]
pub struct SearchRequest {
    pub text: String,
    pub filters: SearchFilters,
    pub k: usize,
}

pub fn search(
    index: &Index<'_>,
    policy: &policy::ReadPolicy,
    context: RequestContext,
    request: SearchRequest,
) -> ApiResponse<SearchResponse> {
    let result = check(context, policy, policy::ReadOperation::Search).and_then(|()| {
        index
            .search(&request.text, request.filters, request.k)
            .map_err(ApiError::Search)
    });
    response(context, result)
}
