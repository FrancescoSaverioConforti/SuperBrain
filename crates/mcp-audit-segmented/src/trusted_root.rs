//! M7.7.4: a trusted segmentation root — the one source of truth for
//! WHERE the retention ledger and pin store live, derived
//! deterministically, never read from the manifest (the frozen M7.7
//! correction).
//!
//! **Why this exists**: `segment-manifest.json` is an accelerator/
//! projection, never an authority (the standing rule throughout
//! `audit-segmentation`). A tampered manifest that names a different,
//! internally-valid retention ledger or pin store must never be
//! silently accepted as "the real one" just because it IS internally
//! valid. [`TrustedSegmentationRoot`] is the independent anchor every
//! locator is checked against — for the pin store, its own locator is
//! NEVER even read from the manifest at all; for the retention ledger
//! (whose locator the manifest DOES carry, by `audit-segmentation`'s
//! own, already-committed design), this module's own derived value is
//! what every comparison is made against, never the manifest's raw
//! field trusted on its own.

#[derive(Debug, Clone)]
pub struct TrustedSegmentationRoot(std::path::PathBuf);

impl TrustedSegmentationRoot {
    /// `root` is supplied explicitly by the caller that owns/controls
    /// this segmentation state directory — the same "locator is
    /// explicit, never discovered" discipline `audit-segmentation`'s
    /// own `crate::adoption` already established for the legacy
    /// segment's own locator.
    pub fn new(root: std::path::PathBuf) -> Self {
        Self(root)
    }

    pub fn expected_retention_ledger_locator(&self) -> std::path::PathBuf {
        self.0.join("retention-ledger")
    }

    pub fn expected_pin_store_locator(&self) -> std::path::PathBuf {
        self.0.join("pin-store")
    }

    pub fn manifest_path(&self) -> std::path::PathBuf {
        self.0.join("segment-manifest.json")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expected_locators_are_deterministic_siblings_of_the_root() {
        let root = TrustedSegmentationRoot::new(std::path::PathBuf::from("/some/trusted/root"));
        assert_eq!(root.expected_retention_ledger_locator(), std::path::PathBuf::from("/some/trusted/root/retention-ledger"));
        assert_eq!(root.expected_pin_store_locator(), std::path::PathBuf::from("/some/trusted/root/pin-store"));
        assert_eq!(root.manifest_path(), std::path::PathBuf::from("/some/trusted/root/segment-manifest.json"));
    }
}
