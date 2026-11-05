//! B2 Task 0/1 scaffold: proves `codegraph-index` can depend on and drive
//! `codegraph-domain::RepositorySource` — now using Task 1's validated
//! `RepositoryId`/`CommitHash`/`FileIdentity` types exclusively, with no
//! raw-`&str` path left over from Task 0's initial scaffold — without
//! depending on tree-sitter, git2, `storage` or `api`. No parsing,
//! extraction, graph or persistence logic exists yet — those are Tasks 3
//! onward.

#[cfg(test)]
mod seam_reachability {
    use codegraph_domain::{
        CommitHash, FileIdentity, RepositoryId, RepositoryRegistry, RepositoryRoot, RepositorySource, SourceError,
    };

    /// A second, independent test double (distinct from
    /// `codegraph_domain`'s own `FakeRepositorySource`) implemented here,
    /// in `codegraph-index`, to prove the seam is reachable and
    /// implementable from a consumer crate — not just from inside
    /// `codegraph-domain`'s own test module. Built entirely from
    /// `codegraph_domain`'s validated types: there is no raw-string
    /// constructor this crate could use instead even if it wanted to.
    struct SingleFileSource {
        repo: RepositoryId,
        commit: CommitHash,
        file: FileIdentity,
        bytes: &'static [u8],
    }

    impl RepositorySource for SingleFileSource {
        fn list_files_at(&self, repository: RepositoryId, commit: &CommitHash) -> Result<Vec<String>, SourceError> {
            if repository == self.repo && commit == &self.commit {
                Ok(vec![self.file.relative_path().to_string()])
            } else {
                Err(SourceError::UnknownCommit {
                    repo: repository.as_uuid(),
                    commit: commit.as_str().to_string(),
                })
            }
        }

        fn read_file(&self, file: &FileIdentity) -> Result<Vec<u8>, SourceError> {
            if file == &self.file {
                Ok(self.bytes.to_vec())
            } else {
                Err(SourceError::UnknownFile {
                    repo: file.repository().as_uuid(),
                    commit: file.commit().as_str().to_string(),
                    file: file.relative_path().to_string(),
                })
            }
        }
    }

    fn test_repo_and_commit() -> (RepositoryId, CommitHash) {
        let mut registry = RepositoryRegistry::new();
        let repo = registry.register(RepositoryRoot::parse(&std::env::current_dir().unwrap()).unwrap());
        let commit = CommitHash::parse(&"a".repeat(40)).unwrap();
        (repo, commit)
    }

    #[test]
    fn codegraph_index_can_implement_and_drive_repository_source() {
        let (repo, commit) = test_repo_and_commit();
        let file = FileIdentity::parse(repo, commit.clone(), "src/main.rs").unwrap();
        let source = SingleFileSource {
            repo,
            commit: commit.clone(),
            file: file.clone(),
            bytes: b"fn main() {}",
        };

        let files = source.list_files_at(repo, &commit).unwrap();
        assert_eq!(files, vec!["src/main.rs".to_string()]);

        let bytes = source.read_file(&file).unwrap();
        assert_eq!(bytes, b"fn main() {}".to_vec());
    }

    #[test]
    fn codegraph_index_receives_the_same_typed_errors_codegraph_domain_defines() {
        let (repo, commit) = test_repo_and_commit();
        let file = FileIdentity::parse(repo, commit.clone(), "src/main.rs").unwrap();
        let source = SingleFileSource {
            repo,
            commit: commit.clone(),
            file: file.clone(),
            bytes: b"fn main() {}",
        };

        let missing = FileIdentity::parse(repo, commit.clone(), "src/missing.rs").unwrap();
        let result = source.read_file(&missing);

        assert_eq!(
            result,
            Err(SourceError::UnknownFile {
                repo: repo.as_uuid(),
                commit: commit.as_str().to_string(),
                file: "src/missing.rs".to_string(),
            })
        );
    }

    // `codegraph-index` cannot bypass validation: there is no overload of
    // `RepositorySource::list_files_at`/`read_file` taking a raw `&str`
    // for `repository`/`commit`/`file` to call instead of the typed one
    // above. The only way to obtain a `RepositoryId` is through
    // `RepositoryRegistry`, the only way to obtain a `CommitHash` is
    // through `CommitHash::parse` (which rejects malformed hashes), and
    // the only way to obtain a `FileIdentity` is through
    // `FileIdentity::parse` (which rejects traversal/absolute paths) bound
    // to an already-validated `RepositoryId`/`CommitHash`. This is a
    // structural property the type signatures already enforce at compile
    // time — demonstrated here by the fact that every call site above
    // must go through those constructors to produce a value with the
    // right type at all.
}
