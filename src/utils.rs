pub fn slugify(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .filter_map(|c| match c {
            ' ' | '_' => Some('-'),
            c if c.is_ascii_alphanumeric() => Some(c),
            _ => None,
        })
        .collect()
}

/// Hash the given `bytes`.
pub fn hash(bytes: &[u8]) -> String {
    format!("{:016x}", seahash::hash(bytes))
}

pub fn divide_round_up(dividend: u8, divisor: u8) -> u8 {
    if divisor == 0 {
        panic!("Division by zero");
    }
    (dividend + divisor - 1) / divisor
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_slugify() {
        assert_eq!(slugify("Hello, World!"), "hello-world");
    }

    #[test]
    fn test_divide_round_up() {
        assert_eq!(divide_round_up(5, 2), 3);
        assert_eq!(divide_round_up(4, 2), 2);
    }
}
