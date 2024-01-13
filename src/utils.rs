pub fn slugify(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .filter_map(|c| match c {
            ' ' => Some('-'),
            c if c.is_ascii_alphanumeric() => Some(c),
            _ => None,
        })
        .collect()
}

/// Hash the given `bytes`.
pub fn hash(bytes: &[u8]) -> String {
    format!("{:016x}", seahash::hash(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_slugify() {
        assert_eq!(slugify("Hello, World!"), "hello-world");
    }
}
