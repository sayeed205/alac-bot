//! MD5 request signature generation for Qobuz API endpoints.

use md5::{Digest, Md5};

pub fn generate_request_signature(
    method: &str,
    params: &[(&str, &str)],
    request_ts: u64,
    app_secret: &str,
) -> String {
    let method_clean = method.replace('/', "");
    let mut sorted_params = params.to_vec();
    sorted_params.sort_by(|a, b| a.0.cmp(b.0));

    let mut sig_base = method_clean;
    for (key, val) in sorted_params {
        sig_base.push_str(key);
        sig_base.push_str(val);
    }
    sig_base.push_str(&request_ts.to_string());
    sig_base.push_str(app_secret);

    let mut hasher = Md5::new();
    hasher.update(sig_base.as_bytes());
    let result = hasher.finalize();
    result.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_params() -> [(&'static str, &'static str); 3] {
        [
            ("track_id", "46487920"),
            ("format_id", "27"),
            ("intent", "stream"),
        ]
    }

    #[test]
    fn test_signature_format() {
        let sig = generate_request_signature(
            "track/getFileUrl",
            &sample_params(),
            1600000000,
            "f69a7734686cb9427629378a4b7ac381",
        );
        assert_eq!(sig.len(), 32);
        assert!(sig.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_signature_is_deterministic() {
        let a = generate_request_signature(
            "track/getFileUrl",
            &sample_params(),
            1600000000,
            "f69a7734686cb9427629378a4b7ac381",
        );
        let b = generate_request_signature(
            "track/getFileUrl",
            &sample_params(),
            1600000000,
            "f69a7734686cb9427629378a4b7ac381",
        );
        assert_eq!(a, b);
    }

    #[test]
    fn test_signature_sorts_params_before_hashing() {
        // Callers may pass params in any order; the wire signature must not
        // depend on it, or native lookups fail intermittently.
        let ordered = generate_request_signature(
            "track/getFileUrl",
            &sample_params(),
            1600000000,
            "f69a7734686cb9427629378a4b7ac381",
        );
        let shuffled = generate_request_signature(
            "track/getFileUrl",
            &[
                ("intent", "stream"),
                ("track_id", "46487920"),
                ("format_id", "27"),
            ],
            1600000000,
            "f69a7734686cb9427629378a4b7ac381",
        );
        assert_eq!(ordered, shuffled);
    }

    #[test]
    fn test_signature_is_sensitive_to_inputs() {
        let base = generate_request_signature(
            "track/getFileUrl",
            &sample_params(),
            1600000000,
            "f69a7734686cb9427629378a4b7ac381",
        );
        // Different timestamp, track, or secret must all change the digest;
        // otherwise every request would share one signature.
        let other_ts = generate_request_signature(
            "track/getFileUrl",
            &sample_params(),
            1600000001,
            "f69a7734686cb9427629378a4b7ac381",
        );
        let other_track = generate_request_signature(
            "track/getFileUrl",
            &[
                ("track_id", "46487921"),
                ("format_id", "27"),
                ("intent", "stream"),
            ],
            1600000000,
            "f69a7734686cb9427629378a4b7ac381",
        );
        let other_secret = generate_request_signature(
            "track/getFileUrl",
            &sample_params(),
            1600000000,
            "806331c3b0b641da923b890aed01d04a",
        );
        assert_ne!(base, other_ts);
        assert_ne!(base, other_track);
        assert_ne!(base, other_secret);
    }
}
