use rand::{distr::Alphanumeric, Rng};

/// A random session token for LAN listening (Phase 1; replaced by pairing in Phase 2).
pub fn generate_token() -> String {
    rand::rng()
        .sample_iter(Alphanumeric)
        .take(20)
        .map(char::from)
        .collect()
}

/// Compare tokens without short-circuiting on the first differing byte.
pub fn tokens_match(expected: &str, given: &str) -> bool {
    let (a, b) = (expected.as_bytes(), given.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens() {
        let t = generate_token();
        assert_eq!(t.len(), 20);
        assert!(t.chars().all(|c| c.is_ascii_alphanumeric()));
        assert_ne!(t, generate_token());
        assert!(tokens_match(&t, &t.clone()));
        assert!(!tokens_match(&t, &t[..19]));
        assert!(!tokens_match("abc", "abd"));
    }
}
