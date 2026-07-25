//! The Netscape cookie-jar format — the ONE owner of it in this codebase.
//!
//! # Why this file exists
//!
//! The highest-leverage thing an agent can do with a logged-in web flow is
//! split it: script the mechanical parts on `curl`, hand the session to a
//! surface for the one step that genuinely needs a browser, and hand it back.
//! That was proven both necessary and sufficient in the field — transplanting a
//! single `PHPSESSID` into a browser made rtionline render the applicant's name
//! and the fee — and the only thing missing was a way to move the jar.
//!
//! `curl -c`/`-b` speaks exactly one format, so this module speaks exactly that
//! format, and nothing else in the tree parses or writes it.
//!
//! # The format
//!
//! Seven TAB-separated fields per line:
//!
//! ```text
//! domain  include_subdomains  path  secure  expires  name  value
//! ```
//!
//! - `include_subdomains` is `TRUE`/`FALSE` and is DERIVED, not stored twice: a
//!   leading `.` on the domain is what "applies to subdomains" means, so it is
//!   written from the domain and consumed back into it. One rule, one field of
//!   truth.
//! - `expires` is a unix timestamp; `0` means a SESSION cookie.
//! - `#` starts a comment, with ONE exception: curl writes `#HttpOnly_<domain>`
//!   as the domain field of an http-only cookie. Dropping that prefix as a
//!   comment is the mistake every reimplementation makes, and it is exactly
//!   what a curl-written jar carries for a session id.

/// One cookie, in the terms the jar file speaks.
///
/// Deliberately NOT the engine's cookie type: the vendored engine layer must
/// not learn about jar files, and this module must not learn about libsoup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CookieSpec {
    pub name: String,
    pub value: String,
    /// A leading `.` means "and subdomains" — the same encoding the file uses.
    pub domain: String,
    pub path: String,
    /// `None` is a session cookie (`0` in the file).
    pub expires_unix: Option<i64>,
    pub secure: bool,
    pub http_only: bool,
}

const HTTP_ONLY_PREFIX: &str = "#HttpOnly_";

/// Parse a Netscape jar. Malformed LINES are skipped, not fatal — a jar written
/// by one tool and read by another routinely carries a header or a stray blank
/// line, and refusing the whole file over one of them would make the verb
/// useless. A line with the right shape but a bad number IS an error, because
/// silently treating a garbled expiry as a session cookie would change the
/// meaning of the transplant.
pub fn parse_netscape_jar(text: &str) -> Result<Vec<CookieSpec>, String> {
    let mut cookies = Vec::new();
    for (index, raw) in text.lines().enumerate() {
        let line = raw.trim_end_matches(['\r', '\n']);
        if line.trim().is_empty() {
            continue;
        }
        // The one comment that is not a comment.
        let (line, http_only) = match line.strip_prefix(HTTP_ONLY_PREFIX) {
            Some(rest) => (rest, true),
            None if line.trim_start().starts_with('#') => continue,
            None => (line, false),
        };
        let fields: Vec<&str> = line.split('\t').collect();
        if fields.len() < 7 {
            continue;
        }
        let expires: i64 = fields[4]
            .trim()
            .parse()
            .map_err(|_| format!("line {}: bad expires field {:?}", index + 1, fields[4]))?;
        let file_domain = fields[0].trim();
        let include_subdomains = fields[1].trim().eq_ignore_ascii_case("TRUE");
        // ONE rule: the leading dot IS "include subdomains". Whichever of the
        // two spellings the writer used, the parsed domain carries it.
        let domain = if include_subdomains && !file_domain.starts_with('.') {
            format!(".{file_domain}")
        } else {
            file_domain.to_string()
        };
        cookies.push(CookieSpec {
            name: fields[5].trim().to_string(),
            // The value is the REST of the line: a cookie value may legally
            // contain anything but a tab, and rejoining preserves it exactly.
            value: fields[6..].join("\t"),
            domain,
            path: fields[2].trim().to_string(),
            expires_unix: (expires != 0).then_some(expires),
            secure: fields[3].trim().eq_ignore_ascii_case("TRUE"),
            http_only,
        });
    }
    Ok(cookies)
}

/// Write a Netscape jar that `curl -b` accepts.
pub fn format_netscape_jar(cookies: &[CookieSpec]) -> String {
    let mut out = String::from("# Netscape HTTP Cookie File\n# Written by yggterm (web cookies --export)\n\n");
    for cookie in cookies {
        let include_subdomains = cookie.domain.starts_with('.');
        if cookie.http_only {
            out.push_str(HTTP_ONLY_PREFIX);
        }
        out.push_str(&format!(
            "{domain}\t{subdomains}\t{path}\t{secure}\t{expires}\t{name}\t{value}\n",
            domain = cookie.domain,
            subdomains = if include_subdomains { "TRUE" } else { "FALSE" },
            path = if cookie.path.is_empty() {
                "/"
            } else {
                &cookie.path
            },
            secure = if cookie.secure { "TRUE" } else { "FALSE" },
            expires = cookie.expires_unix.unwrap_or(0),
            name = cookie.name,
            value = cookie.value,
        ));
    }
    out
}

/// Write a jar file OWNER-ONLY (`0600`).
///
/// The bytes are live session credentials — the whole point of the verb is that
/// a transplanted `PHPSESSID` logs you in — so a jar written at the ambient
/// umask (typically `0644`) is a world-readable credential sitting in whatever
/// directory the agent was pointed at. The jar format's owner owns this too:
/// nowhere else may write one.
///
/// The mode is set on the OPEN and again after the write, because `mode()`
/// applies only when the file is created — exporting over a jar that already
/// exists at `0644` must tighten it, not inherit it.
pub fn write_jar_file(path: &std::path::Path, text: &str) -> std::io::Result<()> {
    use std::io::Write as _;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    file.write_all(text.as_bytes())?;
    file.flush()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A real curl-written jar, including the two things a reimplementation
    /// gets wrong: the `#HttpOnly_` domain prefix, and `expires 0` meaning a
    /// SESSION cookie rather than "expired in 1970".
    ///
    /// This is also the exact shape of the transplant that was proven
    /// sufficient in the field: one PHPSESSID moved from curl into a browser.
    const CURL_JAR: &str = "\
# Netscape HTTP Cookie File
# https://curl.se/docs/http-cookies.html
# This file was generated by libcurl! Edit at your own risk.

#HttpOnly_rtionline.gov.in\tFALSE\t/\tTRUE\t0\tPHPSESSID\tq7v2n8m4k1
.gov.in\tTRUE\t/\tFALSE\t1789000000\tlang\ten-IN
";

    #[test]
    fn a_curl_written_jar_round_trips_including_the_http_only_prefix() {
        let parsed = parse_netscape_jar(CURL_JAR).expect("a curl jar must parse");
        assert_eq!(parsed.len(), 2, "both cookies, and no comment lines");

        let session = &parsed[0];
        assert_eq!(session.name, "PHPSESSID");
        assert_eq!(session.value, "q7v2n8m4k1");
        assert_eq!(session.domain, "rtionline.gov.in");
        assert!(
            session.http_only,
            "#HttpOnly_ is a DOMAIN PREFIX, not a comment — dropping it is the \
             bug that loses exactly the session cookie this verb exists to move"
        );
        assert!(session.secure);
        assert_eq!(
            session.expires_unix, None,
            "expires 0 is a SESSION cookie, not an expiry in 1970"
        );

        let lang = &parsed[1];
        assert_eq!(lang.domain, ".gov.in", "include_subdomains IS the leading dot");
        assert_eq!(lang.expires_unix, Some(1_789_000_000));
        assert!(!lang.secure);
        assert!(!lang.http_only);

        // Round-trip: written and re-read, every field survives.
        let written = format_netscape_jar(&parsed);
        assert!(written.contains("#HttpOnly_rtionline.gov.in\t"));
        // `.gov.in` keeps its dot AND gets include_subdomains TRUE: the two
        // spellings of the same fact stay consistent.
        assert!(written.contains(".gov.in\tTRUE\t"));
        assert_eq!(parse_netscape_jar(&written).unwrap(), parsed);
    }

    #[test]
    fn the_leading_dot_and_include_subdomains_never_disagree() {
        // Written with the dot but FALSE in the flag column: the dot wins, and
        // a re-export makes the two agree.
        let odd = ".example.com\tFALSE\t/\tFALSE\t0\ta\t1\n";
        let parsed = parse_netscape_jar(odd).unwrap();
        assert_eq!(parsed[0].domain, ".example.com");
        assert!(format_netscape_jar(&parsed).contains(".example.com\tTRUE\t"));

        // Written without the dot but TRUE in the flag column: the flag is
        // honoured by ADDING the dot, so there is one representation.
        let other = "example.com\tTRUE\t/\tFALSE\t0\ta\t1\n";
        let parsed = parse_netscape_jar(other).unwrap();
        assert_eq!(parsed[0].domain, ".example.com");
    }

    #[test]
    fn junk_lines_are_skipped_but_a_garbled_expiry_is_an_error() {
        // A header, a blank line and a short line are ordinary in the wild.
        let jar = "# comment\n\nnot\tenough\tfields\nexample.com\tFALSE\t/\tFALSE\t0\ta\t1\n";
        assert_eq!(parse_netscape_jar(jar).unwrap().len(), 1);

        // A well-shaped line with a bad number is NOT ordinary: treating it as
        // a session cookie would silently change what gets transplanted.
        let bad = "example.com\tFALSE\t/\tFALSE\tsoon\ta\t1\n";
        let err = parse_netscape_jar(bad).expect_err("a garbled expiry must be an error");
        assert!(err.contains("bad expires"), "{err}");
    }

    #[test]
    fn a_value_containing_separators_survives_the_round_trip() {
        // Base64 and JWT values carry `=`, `.` and `/`; only a TAB is illegal,
        // and a value that somehow contains one is rejoined rather than cut.
        let jar = "example.com\tFALSE\t/\tTRUE\t0\ttoken\teyJhbGciOi.J9=/+\n";
        let parsed = parse_netscape_jar(jar).unwrap();
        assert_eq!(parsed[0].value, "eyJhbGciOi.J9=/+");
        assert_eq!(parse_netscape_jar(&format_netscape_jar(&parsed)).unwrap(), parsed);
    }

    #[test]
    fn an_empty_path_is_written_as_root_rather_than_an_empty_field() {
        let cookie = CookieSpec {
            name: "a".into(),
            value: "1".into(),
            domain: "example.com".into(),
            path: String::new(),
            expires_unix: None,
            secure: false,
            http_only: false,
        };
        assert!(format_netscape_jar(&[cookie]).contains("example.com\tFALSE\t/\tFALSE\t0\ta\t1"));
    }

    /// An exported jar is a CREDENTIAL FILE, so it is written owner-only.
    ///
    /// `fs::write` lands it at the ambient umask — typically `0644` — and the
    /// bytes are live session cookies: the module's own premise is that
    /// transplanting one `PHPSESSID` logs you in. Both cases are covered
    /// because they need different mechanisms: a NEW file gets its mode from
    /// the open, and an EXISTING one has to be tightened after it.
    ///
    /// Replace `write_jar_file`'s body with `std::fs::write(path, text)` and
    /// this fails.
    #[cfg(unix)]
    #[test]
    fn an_exported_jar_is_written_owner_only() {
        use std::os::unix::fs::PermissionsExt as _;

        let dir = std::env::temp_dir().join(format!(
            "yggterm-jar-mode-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or_default()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("session.jar");

        let cookies = vec![CookieSpec {
            name: "PHPSESSID".into(),
            value: "a-live-session".into(),
            domain: ".example.com".into(),
            path: "/".into(),
            expires_unix: None,
            secure: true,
            http_only: true,
        }];
        write_jar_file(&path, &format_netscape_jar(&cookies)).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "a fresh jar must not be world-readable");
        // …and it is still a jar the round trip accepts.
        assert_eq!(
            parse_netscape_jar(&std::fs::read_to_string(&path).unwrap()).unwrap(),
            cookies
        );

        // Exporting OVER a jar that already exists at 0644 must tighten it —
        // `mode()` on the open applies only at creation.
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        write_jar_file(&path, &format_netscape_jar(&cookies)).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "re-exporting must not inherit a loose mode");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
