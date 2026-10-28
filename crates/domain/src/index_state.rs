/// Reported state of whether a real index reflects a note's current content.
///
/// This is a claim about the *index*, not about the write pipeline. It is
/// deliberately distinct from `WriteState::IndexCurrent`, which only means the
/// write pipeline's index step completed. In A2 there is no real indexer, so
/// `Stale` is the only honestly producible value; `Current` exists so that a
/// future real indexer (phase B1) can report it once it actually exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndexState {
    Current,
    Stale,
}

#[cfg(test)]
mod tests {
    use super::IndexState;

    #[test]
    fn index_state_has_current_and_stale_variants() {
        let a = IndexState::Current;
        let b = IndexState::Stale;
        assert_ne!(a, b);
    }
}
