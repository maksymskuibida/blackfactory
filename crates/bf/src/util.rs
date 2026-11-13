//! Small helpers: time, hashing, atomic writes, glob matching.

use sha2::{Digest, Sha256};
use std::fs;
use std::io::{self, Read, Write};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

/// Current UTC time as RFC 3339 with second precision.
pub fn now() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    rfc3339(secs)
}

pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub fn rfc3339(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    // Civil-from-days (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        y,
        m,
        d,
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

/// Parse the output of `rfc3339` back to seconds. Returns 0 when malformed.
pub fn parse_rfc3339(s: &str) -> u64 {
    let b = s.as_bytes();
    if b.len() < 19 {
        return 0;
    }
    let num = |a: usize, z: usize| s.get(a..z).and_then(|x| x.parse::<i64>().ok()).unwrap_or(0);
    let (y, m, d) = (num(0, 4), num(5, 7), num(8, 10));
    let (hh, mm, ss) = (num(11, 13), num(14, 16), num(17, 19));
    let y2 = if m <= 2 { y - 1 } else { y };
    let era = y2.div_euclid(400);
    let yoe = y2.rem_euclid(400);
    let mp = if m > 2 { m - 3 } else { m + 9 };
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    (days * 86_400 + hh * 3600 + mm * 60 + ss).max(0) as u64
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

/// HMAC-SHA256, as hex.
pub fn hmac_sha256_hex(key: &[u8], msg: &[u8]) -> String {
    const B: usize = 64;
    let mut k = if key.len() > B {
        Sha256::digest(key).to_vec()
    } else {
        key.to_vec()
    };
    k.resize(B, 0);
    let mut inner = Sha256::new();
    inner.update(k.iter().map(|b| b ^ 0x36).collect::<Vec<u8>>());
    inner.update(msg);
    let mut outer = Sha256::new();
    outer.update(k.iter().map(|b| b ^ 0x5c).collect::<Vec<u8>>());
    outer.update(inner.finalize());
    hex(&outer.finalize())
}

pub fn hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

pub fn sha256_file(path: &Path) -> io::Result<String> {
    let mut f = fs::File::open(path)?;
    let mut h = Sha256::new();
    let mut buf = [0u8; 65_536];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(hex(&h.finalize()))
}

/// Write a file atomically: write a sibling temp file, then rename over the target.
pub fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension(format!("tmp{}", std::process::id()));
    {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.flush()?;
    }
    fs::rename(&tmp, path)
}

/// `n` random bytes as hex, from the operating system.
pub fn random_hex(n: usize) -> io::Result<String> {
    let mut f = fs::File::open("/dev/urandom")?;
    let mut buf = vec![0u8; n];
    f.read_exact(&mut buf)?;
    Ok(hex(&buf))
}

/// Match a path against a simple pattern. `*` matches within a segment,
/// `**` matches across segments, a trailing `/` means "everything below",
/// and a plain path matches itself and anything below it.
pub fn path_matches(pattern: &str, path: &str) -> bool {
    let pattern = pattern.trim_start_matches("./");
    let path = path.trim_start_matches("./");
    if pattern.is_empty() || pattern == "." {
        return true;
    }
    if !pattern.contains('*') {
        let p = pattern.trim_end_matches('/');
        return path == p || path.starts_with(&format!("{p}/"));
    }
    glob(pattern.as_bytes(), path.as_bytes())
}

fn glob(p: &[u8], s: &[u8]) -> bool {
    if p.is_empty() {
        return s.is_empty() || p.is_empty() && s.first() == Some(&b'/');
    }
    if p.starts_with(b"**") {
        let rest = p[2..].strip_prefix(b"/").unwrap_or(&p[2..]);
        for i in 0..=s.len() {
            if glob(rest, &s[i..]) {
                return true;
            }
        }
        return false;
    }
    if p[0] == b'*' {
        let mut i = 0;
        loop {
            if glob(&p[1..], &s[i..]) {
                return true;
            }
            if i >= s.len() || s[i] == b'/' {
                return false;
            }
            i += 1;
        }
    }
    if !s.is_empty() && p[0] == s[0] {
        return glob(&p[1..], &s[1..]);
    }
    false
}

/// Reduce a file scope to its literal folder prefix, for overlap tests.
pub fn norm_scope(x: &str) -> String {
    let x = x.trim_start_matches("./");
    let cut = x.find('*').map(|i| &x[..i]).unwrap_or(x);
    cut.trim_end_matches('/').to_string()
}

/// Do two normalised scopes overlap (one is the same as, or inside, the other)?
pub fn norm_overlap(a: &str, b: &str) -> bool {
    if a.is_empty() || b.is_empty() {
        return true;
    }
    a == b
        || (a.len() > b.len() && a.starts_with(b) && a.as_bytes()[b.len()] == b'/')
        || (b.len() > a.len() && b.starts_with(a) && b.as_bytes()[a.len()] == b'/')
}

/// Do two file scopes overlap (one is the same as, or inside, the other)?
pub fn scopes_overlap(a: &str, b: &str) -> bool {
    norm_overlap(&norm_scope(a), &norm_scope(b))
}

pub fn median(v: &mut [f64]) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[v.len() / 2]
}

pub fn percentile(v: &mut [f64], p: f64) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let i = ((v.len() as f64 - 1.0) * p).round() as usize;
    v[i]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn time_roundtrip() {
        for s in [0u64, 86_399, 1_700_000_000, 1_790_000_123] {
            assert_eq!(parse_rfc3339(&rfc3339(s)), s);
        }
        assert_eq!(rfc3339(0), "1970-01-01T00:00:00Z");
    }

    #[test]
    fn globs() {
        assert!(path_matches("src/", "src/a/b.rs"));
        assert!(path_matches("src", "src/a.rs"));
        assert!(!path_matches("src", "srcs/a.rs"));
        assert!(path_matches("src/*.rs", "src/a.rs"));
        assert!(!path_matches("src/*.rs", "src/x/a.rs"));
        assert!(path_matches("src/**/*.rs", "src/x/y/a.rs"));
        assert!(path_matches("**", "anything/at/all"));
    }

    #[test]
    fn overlap() {
        assert!(scopes_overlap("src/", "src/a.rs"));
        assert!(scopes_overlap("src/a.rs", "src/a.rs"));
        assert!(!scopes_overlap("src/a.rs", "src/b.rs"));
        assert!(!scopes_overlap("docs/", "src/"));
        assert!(scopes_overlap("src/**", "src/x/y.rs"));
    }

    #[test]
    fn hmac_matches_the_rfc_4231_vector() {
        // Test case 2 of RFC 4231.
        assert_eq!(
            hmac_sha256_hex(b"Jefe", b"what do ya want for nothing?"),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
    }

    #[test]
    fn stats() {
        let mut v = vec![3.0, 1.0, 2.0];
        assert_eq!(median(&mut v), 2.0);
        assert_eq!(percentile(&mut v, 1.0), 3.0);
    }
}
