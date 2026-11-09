//! The user's pre-approval rules: matching them, and noticing the ones that are too broad.

use super::classify::*;

/// Read-tool rules that match everything (`*`, `**`, `?*`): they would pre-approve every file in reach.
pub fn dangerous_read_rules(rules: &[String]) -> Vec<(String, String)> {
    rules
        .iter()
        .filter(|r| {
            r.chars()
                .all(|c| c == '*' || c == '?' || c == '/' || c == '.')
        })
        .map(|r| {
            (
                r.clone(),
                "it matches every file the tool may read".to_string(),
            )
        })
        .collect()
}

/// `*` and `?` glob match over one token.
pub fn glob_match(pattern: &str, text: &str) -> bool {
    let (p, t): (Vec<char>, Vec<char>) = (pattern.chars().collect(), text.chars().collect());
    let (mut pi, mut ti, mut star, mut mark) = (0, 0, None, 0);
    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            mark = ti;
            pi += 1;
        } else if let Some(s) = star {
            pi = s + 1;
            mark += 1;
            ti = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

/// Does `rule` (the user's own text from the config, for example `git status` or `ls *`) cover `argv`?
/// Tokens are matched one to one; a trailing `*` matches any remaining arguments, none included.
pub fn rule_matches(rule: &str, argv: &[String]) -> bool {
    let tokens: Vec<&str> = rule.split_whitespace().collect();
    let Some((last, _)) = tokens.split_last() else {
        return false;
    };
    // the program itself must be named: `*` or `?*` would approve every command
    if tokens[0].chars().all(|c| c == '*' || c == '?') {
        return false;
    }
    let trailing_star = *last == "*" && tokens.len() > 1;
    let fixed = if trailing_star {
        &tokens[..tokens.len() - 1]
    } else {
        &tokens[..]
    };
    if trailing_star {
        argv.len() >= fixed.len() && fixed.iter().zip(argv).all(|(p, a)| glob_match(p, a))
    } else {
        argv.len() == fixed.len() && fixed.iter().zip(argv).all(|(p, a)| glob_match(p, a))
    }
}

/// Rules that are too broad to be safe: they match programs the classifier flags, or everything.
/// `config check` reports these; they still work, because the classifier keeps its veto.
pub fn dangerous_rules(rules: &[String]) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for r in rules {
        let first = r.split_whitespace().next().unwrap_or("");
        if first.is_empty() || first.contains('*') || first.contains('?') {
            out.push((
                r.clone(),
                "the program is a wildcard, so it matches every command".into(),
            ));
            continue;
        }
        let probe = r.replace(['*', '?'], "x");
        let a = classify(&probe);
        if a.verdict == Verdict::Unsafe {
            out.push((
                r.clone(),
                format!(
                    "the classifier flags it ({}), so it will still ask",
                    a.reasons.join("; ")
                ),
            ));
        }
    }
    out
}
