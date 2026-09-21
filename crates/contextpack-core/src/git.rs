use std::process::{Command, Output};

use crate::evidence::git_evidence;
use crate::path::{Repository, normalize_newlines};
use crate::plan::{CollectionPlan, GitOperation, GitQuery, GitTarget};
use crate::{
    Diagnostic, EvidenceOrigin, EvidenceRecord, Metrics, QueryResult, QueryType, Severity, Status,
};

pub(crate) struct GitCollection {
    pub result: QueryResult,
    pub evidence: Vec<EvidenceRecord>,
}

pub(crate) fn collect_git(
    plan: &CollectionPlan,
    repository: &Repository,
    query: &GitQuery,
) -> GitCollection {
    if run(repository, &["--version"]).is_err() {
        return unavailable(query, "git_unavailable", "Git executable is unavailable");
    }
    match run(repository, &["rev-parse", "--is-inside-work-tree"]) {
        Ok(output)
            if output.status.success()
                && String::from_utf8_lossy(&output.stdout).trim() == "true" => {}
        _ => {
            return unavailable(query, "git_unavailable", "repository is not a Git worktree");
        }
    }

    if let Some(revision) = revision_to_verify(query)
        && !verify_revision(repository, revision)
    {
        return failed(
            query,
            "git_invalid_revision",
            format!("Git could not resolve revision `{revision}`"),
        );
    }
    if matches!(query.operation, GitOperation::MergeBase)
        && let Some(left) = query.left.as_deref()
        && !verify_revision(repository, left)
    {
        return failed(
            query,
            "git_invalid_revision",
            format!("Git could not resolve revision `{left}`"),
        );
    }
    if matches!(
        query.operation,
        GitOperation::Diff
            | GitOperation::DiffStat
            | GitOperation::ChangedFiles
            | GitOperation::DiffCheck
    ) {
        for revision in [query.base.as_deref(), query.head.as_deref()]
            .into_iter()
            .flatten()
        {
            if !verify_revision(repository, revision) {
                return failed(
                    query,
                    "git_invalid_revision",
                    format!("Git could not resolve revision `{revision}`"),
                );
            }
        }
    }

    let output = match execute_operation(repository, query) {
        Ok(output) => output,
        Err(message) => return failed(query, "git_command_failed", message),
    };
    if !output.status.success() {
        let stderr = normalize_output(&output.stderr);
        let code = if stderr.to_ascii_lowercase().contains("revision")
            || stderr.to_ascii_lowercase().contains("bad object")
            || stderr.to_ascii_lowercase().contains("unknown revision")
        {
            "git_invalid_revision"
        } else {
            "git_command_failed"
        };
        return failed(query, code, stderr.trim().to_string());
    }
    let content = match std::str::from_utf8(&output.stdout) {
        Ok(content) => normalize_newlines(content),
        Err(_) => {
            return unavailable(
                query,
                "unsupported_text_encoding",
                "Git output is not valid UTF-8",
            );
        }
    };
    let original_bytes = content.len();
    let max_lines = query
        .limits
        .max_evidence_lines
        .unwrap_or(plan.limits.max_evidence_lines);
    let max_bytes = query
        .limits
        .max_evidence_bytes
        .unwrap_or(plan.limits.max_evidence_bytes);
    let origin = EvidenceOrigin::Git {
        operation: operation_name(&query.operation).into(),
        target: query.target.as_ref().map(target_name).map(str::to_owned),
        base: query.base.clone(),
        head: query.head.clone(),
        paths: query.paths.clone(),
    };
    let evidence = git_evidence(&query.id, origin, &content, max_lines, max_bytes);
    let mut diagnostics = Vec::new();
    if !evidence.truncations.is_empty() {
        diagnostics.push(Diagnostic::query(
            "evidence_limit_reached",
            Severity::Warning,
            "Git output exceeded its evidence limit",
            &query.id,
        ));
    }
    let empty_is_negative = matches!(
        query.operation,
        GitOperation::Diff
            | GitOperation::DiffStat
            | GitOperation::ChangedFiles
            | GitOperation::DiffCheck
    );
    let status = if content.is_empty() && empty_is_negative {
        Status::Empty
    } else if !evidence.truncations.is_empty() {
        Status::Partial
    } else {
        Status::Ok
    };
    let included_bytes = evidence.payload_bytes();
    let evidence_id = evidence.evidence_id.clone();
    GitCollection {
        result: QueryResult {
            query_id: query.id.clone(),
            query_type: QueryType::Git,
            status,
            evidence_ids: vec![evidence_id],
            candidates: vec![],
            diagnostics,
            metrics: Metrics::Git {
                output_bytes: original_bytes,
                included_bytes,
            },
            truncations: vec![],
        },
        evidence: vec![evidence],
    }
}

fn execute_operation(repository: &Repository, query: &GitQuery) -> Result<Output, String> {
    let mut args = Vec::<String>::new();
    match query.operation {
        GitOperation::Status => {
            args.extend(
                [
                    "status",
                    "--porcelain=v1",
                    "--branch",
                    "--untracked-files=all",
                ]
                .map(str::to_owned),
            );
        }
        GitOperation::Branch => {
            let head = run_success(repository, &["rev-parse", "HEAD"])?;
            let branch = run(repository, &["symbolic-ref", "--short", "-q", "HEAD"])
                .ok()
                .filter(|o| o.status.success())
                .map(|o| normalize_output(&o.stdout).trim().to_string());
            let upstream = run(repository, &["rev-parse", "--abbrev-ref", "@{upstream}"])
                .ok()
                .filter(|o| o.status.success())
                .map(|o| normalize_output(&o.stdout).trim().to_string());
            let text = format!(
                "head {}\nbranch {}\nupstream {}\n",
                head.trim(),
                branch.as_deref().unwrap_or("(detached)"),
                upstream.as_deref().unwrap_or("(none)")
            );
            return Ok(success_output(text));
        }
        GitOperation::Log => {
            args.extend([
                "log".into(),
                "--no-color".into(),
                "--format=%H%x09%P%x09%an%x09%aI%x09%s".into(),
                "-n".into(),
                query.max_entries.unwrap_or(10).to_string(),
                query.revision.as_deref().unwrap_or("HEAD").into(),
                "--".into(),
            ]);
        }
        GitOperation::Show => {
            args.extend([
                "show".into(),
                "--no-color".into(),
                "--no-ext-diff".into(),
                "--no-textconv".into(),
                "--format=fuller".into(),
                query.revision.as_deref().unwrap_or("HEAD").into(),
                "--".into(),
            ]);
            add_paths(&mut args, &query.paths);
        }
        GitOperation::Blame => {
            args.extend(["blame".into(), "--line-porcelain".into()]);
            if let (Some(start), Some(end)) = (query.start_line, query.end_line) {
                args.extend(["-L".into(), format!("{start},{end}")]);
            }
            args.push(query.revision.as_deref().unwrap_or("HEAD").into());
            args.push("--".into());
            args.push(literal_pathspec(query.path.as_deref().unwrap()));
        }
        GitOperation::MergeBase => {
            args.extend([
                "merge-base".into(),
                query.left.as_deref().unwrap_or("HEAD").into(),
                query.right.as_deref().unwrap().into(),
            ]);
        }
        GitOperation::Diff
        | GitOperation::DiffStat
        | GitOperation::ChangedFiles
        | GitOperation::DiffCheck => {
            args.extend([
                "diff".into(),
                "--no-color".into(),
                "--no-ext-diff".into(),
                "--no-textconv".into(),
            ]);
            match query.operation {
                GitOperation::DiffStat => args.push("--stat".into()),
                GitOperation::ChangedFiles => args.push("--name-status".into()),
                GitOperation::DiffCheck => args.push("--check".into()),
                _ => {}
            }
            match query.target.as_ref().unwrap() {
                GitTarget::WorkingTree => {}
                GitTarget::Staged => args.push("--cached".into()),
                GitTarget::Baseline => args.push(query.base.as_ref().unwrap().clone()),
                GitTarget::Revisions => {
                    args.push(query.base.as_ref().unwrap().clone());
                    args.push(query.head.as_deref().unwrap_or("HEAD").into());
                }
            }
            args.push("--".into());
            add_paths(&mut args, &query.paths);
        }
    }
    let borrowed = args.iter().map(String::as_str).collect::<Vec<_>>();
    run(repository, &borrowed).map_err(|e| e.to_string())
}

fn run(repository: &Repository, args: &[&str]) -> std::io::Result<Output> {
    let mut command = Command::new("git");
    command
        .current_dir(repository.root())
        .arg("--no-pager")
        .arg("--literal-pathspecs")
        .args(["-c", "color.ui=false", "-c", "core.quotepath=false"])
        .args([
            "-c",
            "core.fsmonitor=false",
            "-c",
            "core.untrackedCache=false",
        ])
        .args(args)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("LC_ALL", "C")
        .env("LANG", "C");
    command.output()
}

fn run_success(repository: &Repository, args: &[&str]) -> Result<String, String> {
    let output = run(repository, args).map_err(|e| e.to_string())?;
    if output.status.success() {
        Ok(normalize_output(&output.stdout))
    } else {
        Err(normalize_output(&output.stderr))
    }
}

fn verify_revision(repository: &Repository, revision: &str) -> bool {
    run(
        repository,
        &[
            "rev-parse",
            "--verify",
            "--quiet",
            "--end-of-options",
            revision,
        ],
    )
    .is_ok_and(|output| output.status.success())
}

fn revision_to_verify(query: &GitQuery) -> Option<&str> {
    match query.operation {
        GitOperation::Log | GitOperation::Show | GitOperation::Blame => {
            Some(query.revision.as_deref().unwrap_or("HEAD"))
        }
        GitOperation::MergeBase => query.right.as_deref(),
        _ => None,
    }
}

fn add_paths(args: &mut Vec<String>, paths: &[String]) {
    args.extend(paths.iter().map(|path| literal_pathspec(path)));
}

fn literal_pathspec(path: &str) -> String {
    path.to_owned()
}

fn normalize_output(bytes: &[u8]) -> String {
    normalize_newlines(&String::from_utf8_lossy(bytes))
}

fn operation_name(operation: &GitOperation) -> &'static str {
    match operation {
        GitOperation::Status => "status",
        GitOperation::Branch => "branch",
        GitOperation::Log => "log",
        GitOperation::Diff => "diff",
        GitOperation::DiffStat => "diff_stat",
        GitOperation::Show => "show",
        GitOperation::Blame => "blame",
        GitOperation::MergeBase => "merge_base",
        GitOperation::ChangedFiles => "changed_files",
        GitOperation::DiffCheck => "diff_check",
    }
}

fn target_name(target: &GitTarget) -> &'static str {
    match target {
        GitTarget::WorkingTree => "working_tree",
        GitTarget::Staged => "staged",
        GitTarget::Baseline => "baseline",
        GitTarget::Revisions => "revisions",
    }
}

fn unavailable(query: &GitQuery, code: &str, message: &str) -> GitCollection {
    terminal(query, Status::Unavailable, code, message.into())
}

fn failed(query: &GitQuery, code: &str, message: String) -> GitCollection {
    terminal(query, Status::Failed, code, message)
}

fn terminal(query: &GitQuery, status: Status, code: &str, message: String) -> GitCollection {
    GitCollection {
        result: QueryResult {
            query_id: query.id.clone(),
            query_type: QueryType::Git,
            status,
            evidence_ids: vec![],
            candidates: vec![],
            diagnostics: vec![Diagnostic::query(code, Severity::Error, message, &query.id)],
            metrics: Metrics::Git {
                output_bytes: 0,
                included_bytes: 0,
            },
            truncations: vec![],
        },
        evidence: vec![],
    }
}

#[cfg(windows)]
fn success_output(text: String) -> Output {
    use std::os::windows::process::ExitStatusExt;
    Output {
        status: std::process::ExitStatus::from_raw(0),
        stdout: text.into_bytes(),
        stderr: vec![],
    }
}

#[cfg(unix)]
fn success_output(text: String) -> Output {
    use std::os::unix::process::ExitStatusExt;
    Output {
        status: std::process::ExitStatus::from_raw(0),
        stdout: text.into_bytes(),
        stderr: vec![],
    }
}
