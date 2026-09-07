use regex::bytes::Regex;
use std::sync::OnceLock;

fn secret_regexes() -> &'static [Regex] {
    static RE: OnceLock<Vec<Regex>> = OnceLock::new();
    RE.get_or_init(|| {
        [
            r"-----BEGIN [A-Z ]*PRIVATE KEY-----",
            r"AKIA[0-9A-Z]{16}",
            r"ghp_[A-Za-z0-9]{20,}",
            r"sk-[A-Za-z0-9]{20,}",
        ]
        .into_iter()
        .map(|p| Regex::new(p).expect("secret regex"))
        .collect()
    })
}

/// High-confidence secret content check. Uses `is_match` only — never stores captures.
pub fn is_secret_content(bytes: &[u8]) -> bool {
    secret_regexes().iter().any(|re| re.is_match(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_pem_header() {
        let pem = b"-----BEGIN RSA PRIVATE KEY-----\nMIIEowIBAAKCAQEA\n";
        assert!(is_secret_content(pem));
        assert!(!is_secret_content(b"hello world"));
    }
}
