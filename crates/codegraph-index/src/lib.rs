//! B2 Task 0 scaffold: proves `codegraph-index` can depend on and drive
//! `codegraph-domain::RepositorySource` without depending on tree-sitter,
//! git2, `storage` or `api`. No parsing, extraction, graph or persistence
//! logic exists yet — those are Tasks 3 onward.

#[cfg(test)]
mod seam_reachability {
    use codegraph_domain::{RepositorySource, SourceError};

    /// A second, independent test double (distinct from
    /// `codegraph_domain`'s own `FakeRepositorySource`) implemented here,
    /// in `codegraph-index`, to prove the seam is reachable and
    /// implementable from a consumer crate — not just from inside
    /// `codegraph-domain`'s own test module.
    struct SingleFileSource {
        repo: &'static str,
        commit: &'static str,
        file: &'static str,
        bytes: &'static [u8],
    }

    impl RepositorySource for SingleFileSource {
        fn list_files_at(&self, repo: &str, commit: &str) -> Result<Vec<String>, SourceError> {
            if repo == self.repo && commit == self.commit {
                Ok(vec![self.file.to_string()])
            } else {
                Err(SourceError::UnknownCommit {
                    repo: repo.to_string(),
                    commit: commit.to_string(),
                })
            }
        }

        fn read_file(&self, repo: &str, commit: &str, file: &str) -> Result<Vec<u8>, SourceError> {
            if repo == self.repo && commit == self.commit && file == self.file {
                Ok(self.bytes.to_vec())
            } else {
                Err(SourceError::UnknownFile {
                    repo: repo.to_string(),
                    commit: commit.to_string(),
                    file: file.to_string(),
                })
            }
        }
    }

    #[test]
    fn codegraph_index_can_implement_and_drive_repository_source() {
        let source = SingleFileSource {
            repo: "superbrain",
            commit: "cafefeed",
            file: "src/main.rs",
            bytes: b"fn main() {}",
        };

        let files = source.list_files_at("superbrain", "cafefeed").unwrap();
        assert_eq!(files, vec!["src/main.rs".to_string()]);

        let bytes = source.read_file("superbrain", "cafefeed", "src/main.rs").unwrap();
        assert_eq!(bytes, b"fn main() {}".to_vec());
    }

    #[test]
    fn codegraph_index_receives_the_same_typed_errors_codegraph_domain_defines() {
        let source = SingleFileSource {
            repo: "superbrain",
            commit: "cafefeed",
            file: "src/main.rs",
            bytes: b"fn main() {}",
        };

        let result = source.read_file("superbrain", "cafefeed", "src/missing.rs");

        assert_eq!(
            result,
            Err(SourceError::UnknownFile {
                repo: "superbrain".to_string(),
                commit: "cafefeed".to_string(),
                file: "src/missing.rs".to_string(),
            })
        );
    }
}
