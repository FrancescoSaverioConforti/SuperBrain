use regex::Regex;
use sha2::{Digest, Sha256};

/// A single registered exception for one exact secret-pattern occurrence.
///
/// Scoped to (`pattern_id`, `note_id`, `matched_text_hash`) *together* —
/// never to `pattern_id` + `note_id` alone. This stops one approved false
/// positive from silently whitelisting every future occurrence of the same
/// pattern in the same note: a real leaked key added later would still be
/// caught even though an earlier doc-placeholder was excepted, because its
/// matched text hashes to a different value.
#[derive(Debug, Clone)]
pub struct SecretException {
    pub pattern_id: String,
    pub matched_text_hash: String,
    pub reason: String,
    pub approved_by: String,
    pub note_id: String,
}

/// One secret-scan hit: which pattern matched, a hash of the exact matched
/// substring (never the matched text itself, to avoid echoing secrets back
/// out in findings), and whether a registered exception covers this exact
/// occurrence.
#[derive(Debug, Clone)]
pub struct SecretFinding {
    pub pattern_id: String,
    pub matched_text_hash: String,
    pub excepted: bool,
}

/// Minimal, versioned set of high-confidence secret-shape patterns. This is
/// intentionally not a comprehensive secret scanner — that is a separate
/// repository-level CI concern (Task 1's workflow). This is a small,
/// deliberately narrow in-process check.
struct SecretPattern {
    id: &'static str,
    regex: &'static str,
}

const SECRET_PATTERNS: &[SecretPattern] = &[
    SecretPattern {
        id: "aws-access-key",
        regex: "AKIA[0-9A-Z]{16}",
    },
    SecretPattern {
        id: "pem-private-key",
        regex: "-----BEGIN PRIVATE KEY-----",
    },
];

pub fn sha256_hex(text: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(text.as_bytes());
    let digest = hasher.finalize();
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

#[derive(Debug, Clone)]
pub struct SecretPolicy {
    #[allow(dead_code)]
    version: u32,
    exceptions: Vec<SecretException>,
}

impl SecretPolicy {
    pub fn new(version: u32) -> Self {
        Self {
            version,
            exceptions: Vec::new(),
        }
    }

    pub fn with_exception(mut self, exc: SecretException) -> Self {
        self.exceptions.push(exc);
        self
    }

    /// Runs each versioned regex pattern against `content`. For each match,
    /// computes a SHA-256 hex hash of the matched substring and checks
    /// whether any registered exception matches on `pattern_id`, `note_id`,
    /// AND `matched_text_hash` all at once — only then is the finding marked
    /// `excepted: true`.
    pub fn scan(&self, content: &str, note_id: &str) -> Vec<SecretFinding> {
        let mut findings = Vec::new();

        for pattern in SECRET_PATTERNS {
            let re = Regex::new(pattern.regex).expect("secret pattern regex is valid");
            for m in re.find_iter(content) {
                let matched_text_hash = sha256_hex(m.as_str());
                let excepted = self.exceptions.iter().any(|exc| {
                    exc.pattern_id == pattern.id
                        && exc.note_id == note_id
                        && exc.matched_text_hash == matched_text_hash
                });
                findings.push(SecretFinding {
                    pattern_id: pattern.id.to_string(),
                    matched_text_hash,
                    excepted,
                });
            }
        }

        findings
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scan_finds_aws_access_key_pattern() {
        let policy = SecretPolicy::new(1);
        let findings = policy.scan("aws_key = AKIAABCDEFGHIJKLMNOP", "note-1");
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern_id, "aws-access-key");
        assert!(!findings[0].excepted);
    }

    #[test]
    fn scan_finds_pem_private_key_marker() {
        let policy = SecretPolicy::new(1);
        let findings = policy.scan("-----BEGIN PRIVATE KEY-----\nMII...", "note-1");
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].pattern_id, "pem-private-key");
    }

    #[test]
    fn scan_does_not_flag_fictitious_example_that_fails_pattern_shape() {
        let policy = SecretPolicy::new(1);
        // 19 chars after AKIA-shape prefix instead of exactly 16 -> no match.
        let findings = policy.scan("AKIAEXAMPLE1234567", "note-1");
        assert!(findings.is_empty());
    }

    #[test]
    fn exception_matches_only_exact_pattern_note_and_hash_triple() {
        let hash = sha256_hex("AKIAABCDEFGHIJKLMNOP");
        let exception = SecretException {
            pattern_id: "aws-access-key".to_string(),
            matched_text_hash: hash,
            reason: "documentation placeholder, verified non-functional".to_string(),
            approved_by: "francesco".to_string(),
            note_id: "note-1".to_string(),
        };
        let policy = SecretPolicy::new(1).with_exception(exception);

        let findings = policy.scan("aws_key = AKIAABCDEFGHIJKLMNOP", "note-1");
        assert!(findings[0].excepted);

        // Different matched value, same pattern_id and note_id: must NOT be excepted.
        let findings = policy.scan("aws_key = AKIAZZZZZZZZZZZZZZZZ", "note-1");
        assert!(!findings[0].excepted);

        // Same matched value, different note_id: must NOT be excepted.
        let findings = policy.scan("aws_key = AKIAABCDEFGHIJKLMNOP", "note-2");
        assert!(!findings[0].excepted);
    }
}
