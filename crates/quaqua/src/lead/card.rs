//! G2 card freshness check:
//! Refuses when `projects/<slug>.md` `## Now` has an open `- [ ]` item naming
//! a sha or PR already reachable from the repo's main.
//! `--allow-stale-card` overrides.

use crate::git::GitOps;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StaleCardItem {
    pub line: String,
    pub target: String,
}

/// Extract lines from the `## Now` section of a project card content.
pub fn parse_now_lines(content: &str) -> Vec<String> {
    let mut in_now = false;
    let mut lines = Vec::new();
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("## ") {
            if trimmed == "## Now" {
                in_now = true;
                continue;
            } else if in_now {
                break;
            }
        }
        if in_now {
            lines.push(line.to_string());
        }
    }
    lines
}

/// Extract open checkbox items (`- [ ]`) from `## Now` lines.
pub fn open_now_items(content: &str) -> Vec<String> {
    parse_now_lines(content)
        .into_iter()
        .filter(|line| line.trim_start().starts_with("- [ ]"))
        .collect()
}

/// Extract candidate PR numbers and commit SHAs from an open item line.
pub fn extract_candidates(line: &str) -> (Vec<String>, Vec<String>) {
    let mut prs = Vec::new();
    let mut shas = Vec::new();

    for word in line.split_whitespace() {
        let trimmed = word.trim_matches(|c: char| {
            c == '('
                || c == ')'
                || c == '['
                || c == ']'
                || c == '{'
                || c == '}'
                || c == ','
                || c == '.'
                || c == ';'
                || c == ':'
                || c == '\''
                || c == '"'
                || c == '`'
                || c == '~'
                || c == '*'
                || c == '·'
        });
        if trimmed.is_empty() {
            continue;
        }

        if let Some(rest) = trimmed.strip_prefix('#') {
            if !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit()) {
                prs.push(rest.to_string());
                continue;
            }
        }

        if trimmed.len() >= 7
            && trimmed.len() <= 40
            && trimmed.chars().all(|c| c.is_ascii_hexdigit())
        {
            shas.push(trimmed.to_string());
        }
    }

    (prs, shas)
}

fn is_commit_object<G: GitOps>(repo: &Path, git: &G, sha: &str) -> bool {
    let target = format!("{}^{{commit}}", sha);
    git.run_in(repo, &["rev-parse", "--verify", "--quiet", &target])
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn is_reachable_from_main<G: GitOps>(repo: &Path, git: &G, target_sha: &str) -> bool {
    for branch in ["main", "origin/main", "master", "origin/master"] {
        if let Ok(out) = git.run_in(repo, &["merge-base", "--is-ancestor", target_sha, branch]) {
            if out.status.success() {
                return true;
            }
        }
    }
    false
}

fn pr_merged_in_main<G: GitOps>(repo: &Path, git: &G, pr_num: &str) -> Option<String> {
    for branch in ["main", "origin/main", "master", "origin/master"] {
        let grep_arg = format!("--grep=#[0-9]*{}\\b", pr_num);
        let args = ["log", "-E", "-n", "1", "--format=%H", &grep_arg, branch];
        if let Ok(out) = git.run_in(repo, &args) {
            if out.status.success() {
                let sha = String::from_utf8_lossy(&out.stdout).trim().to_string();
                if !sha.is_empty() {
                    return Some(sha);
                }
            }
        }
    }
    None
}

/// Find all open items in `content` that refer to a PR or SHA already reachable from `main`.
pub fn find_stale_items<G: GitOps>(content: &str, repo: &Path, git: &G) -> Vec<StaleCardItem> {
    let mut stale = Vec::new();
    for line in open_now_items(content) {
        let (prs, shas) = extract_candidates(&line);
        let mut found_stale = false;

        for pr in prs {
            if pr_merged_in_main(repo, git, &pr).is_some() {
                stale.push(StaleCardItem {
                    line: line.clone(),
                    target: format!("PR #{pr}"),
                });
                found_stale = true;
                break;
            }
        }

        if !found_stale {
            for sha in shas {
                if is_commit_object(repo, git, &sha) && is_reachable_from_main(repo, git, &sha) {
                    stale.push(StaleCardItem {
                        line: line.clone(),
                        target: format!("commit {sha}"),
                    });
                    break;
                }
            }
        }
    }
    stale
}

/// Resolve the project repo directory from `projects/INDEX.md` or `projects/<slug>.md`.
pub fn resolve_project_repo(workspace: &Path, slug: &str, card_content: &str) -> Option<PathBuf> {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());

    // 1. Try projects/INDEX.md
    let index = workspace.join("projects/INDEX.md");
    if let Ok(index_content) = std::fs::read_to_string(&index) {
        for line in index_content.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with('|') {
                let parts: Vec<&str> = trimmed.split('|').map(|s| s.trim()).collect();
                if parts.len() >= 3 && parts[1] == slug {
                    let r = parts[2];
                    let repo_str = if let Some(stripped) = r.strip_prefix("~/") {
                        format!("{}/{}", home, stripped)
                    } else if r == "~" {
                        home.clone()
                    } else {
                        r.to_string()
                    };
                    return Some(PathBuf::from(repo_str));
                }
            }
        }
    }

    // 2. Try repo: line in card content
    for line in card_content.lines() {
        let trimmed = line.trim();
        if let Some(r) = trimmed.strip_prefix("repo:") {
            let r = r.trim();
            if !r.is_empty() {
                let repo_str = if let Some(stripped) = r.strip_prefix("~/") {
                    format!("{}/{}", home, stripped)
                } else if r == "~" {
                    home.clone()
                } else {
                    r.to_string()
                };
                return Some(PathBuf::from(repo_str));
            }
        }
    }

    None
}

/// Check that `projects/<slug>.md` has no open items in `## Now` naming an
/// already-merged sha or PR. Refuses with exit 1 if any stale item is found,
/// unless `allow_stale` is true.
pub fn check_card_freshness<G: GitOps>(
    workspace: &Path,
    slug: &str,
    repo_override: Option<&Path>,
    allow_stale: bool,
    git: &G,
) -> Result<(), (i32, String)> {
    if allow_stale {
        return Ok(());
    }

    let card_path = workspace.join("projects").join(format!("{slug}.md"));
    if !card_path.is_file() {
        return Ok(());
    }

    let content = match std::fs::read_to_string(&card_path) {
        Ok(c) => c,
        Err(_) => return Ok(()),
    };

    let repo_path = match repo_override {
        Some(p) => p.to_path_buf(),
        None => match resolve_project_repo(workspace, slug, &content) {
            Some(p) => p,
            None => return Ok(()),
        },
    };

    let stale_items = find_stale_items(&content, &repo_path, git);
    if let Some(first) = stale_items.first() {
        return Err((
            1,
            format!(
                "error: projects/{slug}.md ## Now has open item naming already-merged {}:\n  {}\n  (override with --allow-stale-card)\n",
                first.target,
                first.line
            ),
        ));
    }

    Ok(())
}

/// Convenience wrapper for callers that don't already have root/git.
pub fn check_card_freshness_default(slug: &str, allow_stale: bool) -> Result<(), (i32, String)> {
    if allow_stale {
        return Ok(());
    }
    let root = match crate::workspace::workspace_root_checked() {
        Ok(r) => r,
        Err(_) => return Ok(()),
    };
    let git = crate::git::Git::new();
    check_card_freshness(&root, slug, None, allow_stale, &git)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_now_extracts_only_now_section() {
        let content = r#"# project
repo: ~/Lab/test

## Vision
Some vision

## Now
- [x] already done #100
- [ ] open item #153
- bullet item without checkbox

## Worker rules
Rules here
"#;
        let open = open_now_items(content);
        assert_eq!(open, vec!["- [ ] open item #153".to_string()]);
    }

    #[test]
    fn extract_candidates_finds_prs_and_shas() {
        let line = "- [ ] gatep4 running after #153 merged (commit c457227, also 5801330) · note";
        let (prs, shas) = extract_candidates(line);
        assert_eq!(prs, vec!["153".to_string()]);
        assert_eq!(shas, vec!["c457227".to_string(), "5801330".to_string()]);
    }

    #[test]
    fn find_stale_items_detects_merged_prs_and_commits() {
        struct DirGuard(PathBuf);
        impl Drop for DirGuard {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }

        let base_dir = std::env::temp_dir().join(format!(
            "quaqua-card-fixture-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&base_dir);
        let repo_path = base_dir.join("repo");
        let workspace_dir = base_dir.join("workspace");
        std::fs::create_dir_all(&repo_path).unwrap();
        std::fs::create_dir_all(&workspace_dir).unwrap();
        let _guard = DirGuard(base_dir.clone());

        let run_git = |args: &[&str]| {
            let out = std::process::Command::new("git")
                .arg("-C")
                .arg(&repo_path)
                .args(args)
                .env("GIT_AUTHOR_NAME", "Quaqua Test")
                .env("GIT_AUTHOR_EMAIL", "test@quaqua.local")
                .env("GIT_COMMITTER_NAME", "Quaqua Test")
                .env("GIT_COMMITTER_EMAIL", "test@quaqua.local")
                .output()
                .expect("run git command");
            assert!(
                out.status.success(),
                "git {:?} failed: {}",
                args,
                String::from_utf8_lossy(&out.stderr)
            );
            out
        };

        run_git(&["init", "-b", "main"]);
        run_git(&["config", "user.name", "Quaqua Test"]);
        run_git(&["config", "user.email", "test@quaqua.local"]);
        run_git(&["config", "commit.gpgsign", "false"]);

        // 1. Initial commit on main (known sha)
        std::fs::write(repo_path.join("init.txt"), "initial commit\n").unwrap();
        run_git(&["add", "init.txt"]);
        run_git(&["commit", "-m", "initial commit"]);
        let rev_out = run_git(&["rev-parse", "HEAD"]);
        let full_sha = String::from_utf8_lossy(&rev_out.stdout).trim().to_string();
        let known_sha = full_sha[..7].to_string();

        // 2. Merge commit on main referencing PR #153
        std::fs::write(repo_path.join("pr153.txt"), "PR 153\n").unwrap();
        run_git(&["add", "pr153.txt"]);
        run_git(&["commit", "-m", "Merge pull request #153 from feature"]);

        let git = crate::git::Git::new();

        // 1. Stale open PR item (#153 is merged on main)
        let stale_pr_card = r#"# project
## Now
- [ ] gatep4 running after #153 merged
"#;
        let stale_prs = find_stale_items(stale_pr_card, &repo_path, &git);
        assert_eq!(stale_prs.len(), 1, "{stale_prs:?}");
        assert_eq!(stale_prs[0].target, "PR #153");
        assert!(stale_prs[0].line.contains("#153"));

        // 2. Completed item with same PR (#153) is NOT flagged
        let checked_card = r#"# project
## Now
- [x] gatep4 running after #153 merged
"#;
        let checked_stale = find_stale_items(checked_card, &repo_path, &git);
        assert!(checked_stale.is_empty(), "{checked_stale:?}");

        // 3. Open item with unmerged PR (#999999) is NOT flagged
        let unmerged_card = r#"# project
## Now
- [ ] new feature #999999 in progress
"#;
        let unmerged_stale = find_stale_items(unmerged_card, &repo_path, &git);
        assert!(unmerged_stale.is_empty(), "{unmerged_stale:?}");

        // 4. Stale open SHA item (known_sha is reachable from main)
        let stale_sha_card = format!(
            r#"# project
## Now
- [ ] building on integ {known_sha}
"#
        );
        let stale_shas = find_stale_items(&stale_sha_card, &repo_path, &git);
        assert_eq!(stale_shas.len(), 1, "{stale_shas:?}");
        assert_eq!(stale_shas[0].target, format!("commit {known_sha}"));

        // 5. check_card_freshness refuses unless allow_stale is true
        let projects_dir = workspace_dir.join("projects");
        std::fs::create_dir_all(&projects_dir).unwrap();
        std::fs::write(projects_dir.join("test-slug.md"), stale_pr_card).unwrap();

        // Without --allow-stale-card: Err(1, ...)
        let res = check_card_freshness(&workspace_dir, "test-slug", Some(&repo_path), false, &git);
        assert!(res.is_err());
        let (rc, err) = res.unwrap_err();
        assert_eq!(rc, 1);
        assert!(
            err.contains(
                "projects/test-slug.md ## Now has open item naming already-merged PR #153:"
            ),
            "{err}"
        );
        assert!(
            err.contains("- [ ] gatep4 running after #153 merged"),
            "{err}"
        );
        assert!(err.contains("--allow-stale-card"), "{err}");

        // With --allow-stale-card: Ok(())
        let res_override =
            check_card_freshness(&workspace_dir, "test-slug", Some(&repo_path), true, &git);
        assert!(res_override.is_ok());
    }
}
