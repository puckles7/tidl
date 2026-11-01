//! ETag generation and validation for optimistic concurrency control

use sha2::{Digest, Sha256};

/// Generate an ETag from content
pub fn generate_etag(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    let hash = hasher.finalize();
    format!("\"{}\"", hex::encode(hash))
}

/// Generate a weak ETag
pub fn generate_weak_etag(content: &str) -> String {
    format!("W/{}", generate_etag(content))
}

/// Validate if an ETag matches
pub fn validate_etag(content: &str, etag: &str) -> bool {
    let generated = generate_etag(content);
    generated == etag || generated == etag.trim_matches('"')
}

/// Check if ETag indicates a match
pub fn check_if_match(content: &str, if_match: Option<&str>) -> bool {
    match if_match {
        None => true,      // No condition, always match
        Some("*") => true, // Wildcard match
        Some(etag) => validate_etag(content, etag),
    }
}

/// Check if ETag indicates no match
pub fn check_if_none_match(content: &str, if_none_match: Option<&str>) -> bool {
    match if_none_match {
        None => true,       // No condition, always proceed
        Some("*") => false, // Wildcard, never matches
        Some(etag) => !validate_etag(content, etag),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_etag() {
        let content = "test content";
        let etag = generate_etag(content);
        assert!(etag.starts_with('"'));
        assert!(etag.ends_with('"'));
    }

    #[test]
    fn test_validate_etag() {
        let content = "test content";
        let etag = generate_etag(content);
        assert!(validate_etag(content, &etag));
        assert!(!validate_etag("different content", &etag));
    }

    #[test]
    fn test_check_if_match() {
        let content = "test";
        let etag = generate_etag(content);

        assert!(check_if_match(content, None));
        assert!(check_if_match(content, Some("*")));
        assert!(check_if_match(content, Some(&etag)));
        assert!(!check_if_match(content, Some("\"wrong\"")));
    }
}
