use std::ffi::OsStr;
use std::path::Path;

const MATCH_BONUS: i32 = 1;
const CONTIGUOUS_BONUS: i32 = 8;
const START_BONUS: i32 = 12;
const BOUNDARY_BONUS: i32 = 6;
const BASENAME_BONUS: i32 = 40;
const MAX_LEAD_PENALTY: usize = 10;

/// Case-insensitive ordered-subsequence score. `None` when `query` is not a
/// subsequence of `candidate`. An empty query matches everything with 0.
pub fn fuzzy_score(query: &str, candidate: &str) -> Option<i32> {
    if query.is_empty() {
        return Some(0);
    }
    let query: Vec<char> = query
        .chars()
        .filter_map(|c| c.to_lowercase().next())
        .collect();
    let candidate: Vec<char> = candidate
        .chars()
        .filter_map(|c| c.to_lowercase().next())
        .collect();
    let mut score: i32 = 0;
    let mut ci = 0usize;
    let mut previous: Option<usize> = None;
    let mut first: Option<usize> = None;
    for &qc in &query {
        while ci < candidate.len() && candidate[ci] != qc {
            ci += 1;
        }
        if ci >= candidate.len() {
            return None;
        }
        score += MATCH_BONUS;
        if matches!(previous, Some(p) if p + 1 == ci) {
            score += CONTIGUOUS_BONUS;
        }
        if ci == 0 {
            score += START_BONUS;
        } else if matches!(candidate[ci - 1], '/' | '-' | '_' | '.' | ' ') {
            score += BOUNDARY_BONUS;
        }
        if first.is_none() {
            first = Some(ci);
        }
        previous = Some(ci);
        ci += 1;
    }
    score -= first.unwrap_or(0).min(MAX_LEAD_PENALTY) as i32;
    Some(score)
}

/// Ranks a bookmark: the basename score wins a fixed bonus over the same
/// query scored against the whole path, so `docs` ranks `/srv/docs` above
/// `/docs-archive/notes`.
pub fn score_bookmark(query: &str, path: &Path) -> Option<i32> {
    let full = path.to_string_lossy();
    let base = path
        .file_name()
        .map(OsStr::to_string_lossy)
        .unwrap_or_else(|| full.clone());
    match (
        fuzzy_score(query, &base).map(|s| s + BASENAME_BONUS),
        fuzzy_score(query, &full),
    ) {
        (Some(a), Some(b)) => Some(a.max(b)),
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        (None, None) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_query_matches_everything() {
        assert_eq!(fuzzy_score("", "anything"), Some(0));
        assert_eq!(
            score_bookmark("", Path::new("/srv/docs")),
            Some(BASENAME_BONUS)
        );
    }

    #[test]
    fn subsequence_and_miss() {
        assert!(fuzzy_score("abc", "axbxc").is_some());
        assert_eq!(fuzzy_score("zz", "abc"), None);
        assert_eq!(fuzzy_score("abc", "abc"), Some(31));
    }

    #[test]
    fn basename_outranks_path() {
        assert!(
            score_bookmark("docs", Path::new("/srv/docs"))
                > score_bookmark("docs", Path::new("/docs-archive/notes/x"))
        );
    }

    #[test]
    fn case_insensitive() {
        assert_eq!(
            score_bookmark("DOCS", Path::new("/srv/docs")),
            score_bookmark("docs", Path::new("/srv/docs"))
        );
    }

    #[test]
    fn contiguity_beats_scattered() {
        assert!(fuzzy_score("ab", "ab") > fuzzy_score("ab", "axxxb"));
    }
}
