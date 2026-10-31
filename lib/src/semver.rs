//! Minimal semver: version parsing, requirement ranges, tag selection.
//!
//! Just what a git-tag package manager needs -- `^ ~ >= > <= < =`, exact,
//! `x`-ranges and `*` -- so no external crate has to be pulled in.

use std::cmp::Ordering;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Version {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
    pub pre: Option<String>,
}

impl Version {
    /// `1`, `1.2` or `1.2.3`, optional leading `v`, `-pre`, `+build` suffix.
    pub fn parse(text: &str) -> Result<Version, String> {
        let t = text.trim();
        let t = t
            .strip_prefix('v')
            .or_else(|| t.strip_prefix('V'))
            .unwrap_or(t);
        let t = t.split('+').next().unwrap_or(t);
        let (main, pre) = match t.split_once('-') {
            Some((m, p)) => (m, Some(p.to_owned())),
            None => (t, None),
        };
        let segs: Vec<&str> = main.split('.').collect();
        if segs.is_empty() || segs.len() > 3 {
            return Err(format!("invalid version '{text}'"));
        }
        let mut nums = [0u64; 3];
        for (i, label) in ["major", "minor", "patch"].iter().enumerate() {
            if i >= segs.len() {
                break;
            }
            nums[i] = segs[i]
                .parse::<u64>()
                .map_err(|_| format!("invalid {label} in version '{text}'"))?;
        }
        Ok(Version {
            major: nums[0],
            minor: nums[1],
            patch: nums[2],
            pre,
        })
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> Ordering {
        (self.major, self.minor, self.patch)
            .cmp(&(other.major, other.minor, other.patch))
            .then_with(|| match (&self.pre, &other.pre) {
                (None, None) => Ordering::Equal,
                // a release outranks its own prereleases
                (None, Some(_)) => Ordering::Greater,
                (Some(_), None) => Ordering::Less,
                (Some(a), Some(b)) => cmp_pre(a, b),
            })
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Numeric identifiers compare numerically and rank below alphanumeric ones.
fn cmp_pre(a: &str, b: &str) -> Ordering {
    for (x, y) in a.split('.').zip(b.split('.')) {
        let ord = match (x.parse::<u64>(), y.parse::<u64>()) {
            (Ok(nx), Ok(ny)) => nx.cmp(&ny),
            (Ok(_), Err(_)) => Ordering::Less,
            (Err(_), Ok(_)) => Ordering::Greater,
            (Err(_), Err(_)) => x.cmp(y),
        };
        if ord != Ordering::Equal {
            return ord;
        }
    }
    a.split('.').count().cmp(&b.split('.').count())
}

#[derive(Debug, Clone)]
struct Bound {
    ver: Version,
    inclusive: bool,
}

/// One comparator, already expanded into its lower and upper bounds.
#[derive(Debug, Clone)]
struct Comparator {
    lo: Option<Bound>,
    hi: Option<Bound>,
}

/// A version with optional components: `1`, `1.x`, `1.2.*`.
#[derive(Debug, Default, Clone)]
struct Partial {
    major: Option<u64>,
    minor: Option<u64>,
    patch: Option<u64>,
    pre: Option<String>,
}

impl Partial {
    fn parse(text: &str) -> Result<Partial, String> {
        let t = text
            .trim()
            .strip_prefix('v')
            .or_else(|| text.trim().strip_prefix('V'))
            .unwrap_or(text.trim());
        let t = t.split('+').next().unwrap_or(t);
        let (main, pre) = match t.split_once('-') {
            Some((m, p)) => (m, Some(p.to_owned())),
            None => (t, None),
        };
        let segs: Vec<&str> = main.split('.').collect();
        if segs.is_empty() || segs.len() > 3 {
            return Err(format!("invalid version '{text}'"));
        }
        let mut nums: [Option<u64>; 3] = [None; 3];
        for (i, seg) in segs.iter().enumerate() {
            if seg.is_empty() || matches!(*seg, "x" | "X" | "*") {
                break;
            }
            nums[i] = Some(
                seg.parse::<u64>()
                    .map_err(|_| format!("invalid version '{text}'"))?,
            );
        }
        Ok(Partial {
            major: nums[0],
            minor: nums[1],
            patch: nums[2],
            pre,
        })
    }

    /// The lowest version this partial could mean: missing parts count as 0.
    fn floor(&self) -> Version {
        Version {
            major: self.major.unwrap_or(0),
            minor: self.minor.unwrap_or(0),
            patch: self.patch.unwrap_or(0),
            pre: self.pre.clone(),
        }
    }

    /// One past this partial's x-range: `1.2` -> `1.3.0`, `1` -> `2.0.0`.
    fn x_upper(&self) -> Version {
        if self.minor.is_none() {
            Version {
                major: self.major.unwrap_or(0) + 1,
                minor: 0,
                patch: 0,
                pre: None,
            }
        } else if self.patch.is_none() {
            Version {
                major: self.major.unwrap_or(0),
                minor: self.minor.unwrap_or(0) + 1,
                patch: 0,
                pre: None,
            }
        } else {
            Version {
                major: self.major.unwrap_or(0),
                minor: self.minor.unwrap_or(0),
                patch: self.patch.unwrap_or(0) + 1,
                pre: None,
            }
        }
    }

    fn is_wildcard(&self) -> bool {
        self.major.is_none()
    }
}

fn bound(ver: Version, inclusive: bool) -> Option<Bound> {
    Some(Bound { ver, inclusive })
}

fn parse_comparator(token: &str) -> Result<Comparator, String> {
    let t = token.trim();
    let (op, ver_str) = if let Some(rest) = t.strip_prefix("^") {
        ("^", rest)
    } else if let Some(rest) = t.strip_prefix("~") {
        ("~", rest)
    } else if let Some(rest) = t.strip_prefix(">=") {
        (">=", rest)
    } else if let Some(rest) = t.strip_prefix("<=") {
        ("<=", rest)
    } else if let Some(rest) = t.strip_prefix('>') {
        (">", rest)
    } else if let Some(rest) = t.strip_prefix('<') {
        ("<", rest)
    } else if let Some(rest) = t.strip_prefix('=') {
        ("=", rest)
    } else {
        ("=", t)
    };

    let partial = Partial::parse(ver_str)?;
    if partial.is_wildcard() {
        return Ok(Comparator { lo: None, hi: None });
    }
    let floor = partial.floor();
    let x_upper = partial.x_upper();

    let (lo, hi) = match op {
        // bare and `x`-ranges are the same: inside the named span
        "=" => {
            if partial.patch.is_some() {
                (bound(floor.clone(), true), bound(floor, true))
            } else {
                (bound(floor, true), bound(x_upper, false))
            }
        }
        "^" => {
            let hi = match (floor.major, partial.minor, partial.patch) {
                // 0.x locks to the minor, 0.0.x to the patch
                (0, None, _) => Version {
                    major: 1,
                    minor: 0,
                    patch: 0,
                    pre: None,
                },
                (0, Some(0), None) => Version {
                    major: 0,
                    minor: 1,
                    patch: 0,
                    pre: None,
                },
                (0, Some(0), Some(p)) => Version {
                    major: 0,
                    minor: 0,
                    patch: p + 1,
                    pre: None,
                },
                (0, Some(m), _) => Version {
                    major: 0,
                    minor: m + 1,
                    patch: 0,
                    pre: None,
                },
                (maj, _, _) => Version {
                    major: maj + 1,
                    minor: 0,
                    patch: 0,
                    pre: None,
                },
            };
            (bound(floor, true), bound(hi, false))
        }
        "~" => {
            let hi = if partial.minor.is_some() {
                Version {
                    major: floor.major,
                    minor: floor.minor + 1,
                    patch: 0,
                    pre: None,
                }
            } else {
                Version {
                    major: floor.major + 1,
                    minor: 0,
                    patch: 0,
                    pre: None,
                }
            };
            (bound(floor, true), bound(hi, false))
        }
        ">=" => (bound(floor, true), None),
        ">" => (bound(floor, false), None),
        "<" => (None, bound(floor, false)),
        // `<=2.1` still covers all of 2.1.x, so it opens the x-range instead
        "<=" => (None, bound(x_upper, false)),
        other => return Err(format!("unsupported version operator '{other}'")),
    };
    Ok(Comparator { lo, hi })
}

impl Comparator {
    fn matches(&self, v: &Version) -> bool {
        if let Some(lo) = &self.lo {
            match v.cmp(&lo.ver) {
                Ordering::Less => return false,
                Ordering::Equal if !lo.inclusive => return false,
                _ => {}
            }
        }
        if let Some(hi) = &self.hi {
            match v.cmp(&hi.ver) {
                Ordering::Greater => return false,
                Ordering::Equal if !hi.inclusive => return false,
                _ => {}
            }
        }
        // semver: a prerelease only satisfies a range that names one of the
        // same x.y.z, so `^1.0.0` never picks up `1.1.0-rc.1`
        if v.pre.is_some() {
            let named = [self.lo.as_ref(), self.hi.as_ref()]
                .into_iter()
                .flatten()
                .any(|b| {
                    b.ver.pre.is_some()
                        && (b.ver.major, b.ver.minor, b.ver.patch) == (v.major, v.minor, v.patch)
                });
            if !named {
                return false;
            }
        }
        true
    }
}

/// A version requirement like `^1.2`, `>=1.0, <2.0` or `*` (comma or space
/// separated, every part must match).
#[derive(Debug, Clone)]
pub struct VersionReq {
    comparators: Vec<Comparator>,
}

impl VersionReq {
    pub fn parse(text: &str) -> Result<VersionReq, String> {
        if text.trim().is_empty() {
            return Err("empty version requirement".to_owned());
        }
        let mut comparators = vec![];
        for token in text.replace(',', " ").split_whitespace() {
            comparators.push(parse_comparator(token)?);
        }
        Ok(VersionReq { comparators })
    }

    pub fn matches(&self, v: &Version) -> bool {
        self.comparators.iter().all(|c| c.matches(v))
    }

    /// The tag itself parsed as a version, when it satisfies the requirement.
    pub fn matches_tag(&self, tag: &str) -> Option<Version> {
        let v = Version::parse(tag).ok()?;
        self.matches(&v).then_some(v)
    }
}

/// The highest tag satisfying `req`, with the tag name it came from.
pub fn best_tag<'a>(
    tags: impl IntoIterator<Item = &'a str>,
    req: &VersionReq,
) -> Option<(String, Version)> {
    let mut best: Option<(String, Version)> = None;
    for tag in tags {
        let Some(v) = req.matches_tag(tag) else {
            continue;
        };
        if best.as_ref().map_or(true, |(_, cur)| v > *cur) {
            best = Some((tag.to_owned(), v));
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(text: &str) -> Version {
        Version::parse(text).unwrap()
    }

    fn ok(req: &str, ver: &str) -> bool {
        VersionReq::parse(req).unwrap().matches(&v(ver))
    }

    fn no(req: &str, ver: &str) -> bool {
        !ok(req, ver)
    }

    #[test]
    fn parse_partial_and_decorations() {
        assert_eq!(v("1"), v("1.0.0"));
        assert_eq!(v("1.2"), v("1.2.0"));
        assert_eq!(v("v2.3.4"), v("2.3.4"));
        assert_eq!(v("1.2.3-rc.1+build").pre.as_deref(), Some("rc.1"));
        assert!(Version::parse("a.b.c").is_err());
        assert!(Version::parse("").is_err());
    }

    #[test]
    fn prerelease_ranks_below_release() {
        assert!(v("1.0.0-rc.1") < v("1.0.0"));
        assert!(v("1.0.0-alpha") < v("1.0.0-beta"));
        assert!(v("1.0.0-alpha.1") < v("1.0.0-alpha.2"));
        assert!(v("1.0.0-alpha") < v("1.0.0-alpha.1"));
        assert!(v("1.0.0-2") < v("1.0.0-10"));
        assert!(v("1.0.0-10") < v("1.0.0-spam"));
    }

    #[test]
    fn caret_ranges() {
        assert!(ok("^1.2.3", "1.2.3"));
        assert!(ok("^1.2.3", "1.9.0"));
        assert!(no("^1.2.3", "2.0.0"));
        assert!(no("^1.2.3", "1.2.2"));
        assert!(ok("^1.2", "1.2.0"));
        assert!(no("^1.2", "1.1.9"));
        // 0.x is locked to the minor, 0.0.x to the patch
        assert!(ok("^0.2.1", "0.2.9"));
        assert!(no("^0.2.1", "0.3.0"));
        assert!(ok("^0.0.3", "0.0.3"));
        assert!(no("^0.0.3", "0.0.4"));
        assert!(ok("^0.0", "0.0.9"));
        assert!(no("^0.0", "0.1.0"));
        assert!(ok("^0", "0.9.0"));
        assert!(no("^0", "1.0.0"));
    }

    #[test]
    fn tilde_x_range_and_exact() {
        assert!(ok("~1.2.3", "1.2.9"));
        assert!(no("~1.2.3", "1.3.0"));
        assert!(ok("~1.2", "1.2.7"));
        assert!(no("~1.2", "1.3.0"));
        assert!(ok("~1", "1.9.0"));
        assert!(no("~1", "2.0.0"));
        assert!(ok("1.2", "1.2.9"));
        assert!(no("1.2", "1.3.0"));
        assert!(ok("1", "1.9.9"));
        assert!(no("1", "2.0.0"));
        assert!(ok("1.2.3", "1.2.3"));
        assert!(no("1.2.3", "1.2.4"));
        assert!(ok("1.2.x", "1.2.9"));
        assert!(ok("*", "9.9.9"));
        assert!(ok("1.2.x, <1.2.9", "1.2.5"));
        assert!(no("1.2.x, <1.2.9", "1.2.9"));
        assert!(ok(">=1.2.3, <1.2.9", "1.2.5"));
        assert!(ok(">=1.0, <2.0", "1.5.0"));
        assert!(no(">=1.0, <2.0", "2.0.0"));
        assert!(ok(">1.2.3", "1.2.4"));
        assert!(no(">1.2.3", "1.2.3"));
        assert!(ok("<=2.1", "2.1.7"));
        assert!(no("<=2.1", "2.2.0"));
    }

    #[test]
    fn prerelease_never_sneaks_into_a_stable_range() {
        assert!(no("^1.0.0", "1.5.0-rc.1"));
        assert!(no(">=1.0.0", "1.5.0-rc.1"));
        assert!(ok("^1.0.0-rc.1", "1.0.0-rc.2"));
        assert!(ok("=1.0.0-rc.1", "1.0.0-rc.1"));
        assert!(no("^1.0.0-rc.1", "1.0.1-rc.1"));
    }

    #[test]
    fn best_tag_picks_highest_match() {
        let tags = ["v1.0.0", "v1.2.3", "v2.0.0", "latest", "v1.9.0"];
        let req = VersionReq::parse("^1").unwrap();
        let (name, ver) = best_tag(tags, &req).unwrap();
        assert_eq!(name, "v1.9.0");
        assert_eq!(ver, v("1.9.0"));

        let req = VersionReq::parse(">=2").unwrap();
        assert_eq!(best_tag(tags, &req).unwrap().0, "v2.0.0");

        let req = VersionReq::parse("^9").unwrap();
        assert!(best_tag(tags, &req).is_none());

        let req = VersionReq::parse("*").unwrap();
        assert_eq!(best_tag(tags, &req).unwrap().0, "v2.0.0");
    }

    #[test]
    fn bad_requirement_is_rejected() {
        assert!(VersionReq::parse("").is_err());
        assert!(VersionReq::parse("banana").is_err());
        assert!(VersionReq::parse("^^1").is_err());
        assert!(VersionReq::parse("1.2.3.4").is_err());
    }
}
