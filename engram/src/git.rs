use std::io::{ErrorKind, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

pub const COMMIT_BODY_MAX_CHARS: usize = 800;
pub const GIT_TIMEOUT_MS: u64 = 120_000;
pub const COMMIT_FTS_CAP: usize = 20;
pub const DECISION_SYMBOL_CAP: usize = 10;
pub const SUPERSEDES_NEIGHBOR_CAP: usize = 10;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitCommit {
    pub sha: String,
    pub author: String,
    pub authored_at: String,
    pub subject: String,
    pub body: String,
    pub files: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitError {
    NotInstalled,
    NoRepo,
    Timeout,
    Unparseable,
    Io(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitRange {
    Head,
    After(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitIndexStatus {
    Ok,
    Absent,
    NotInstalled,
    Timeout,
    Unparseable,
}

impl GitIndexStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            GitIndexStatus::Ok => "ok",
            GitIndexStatus::Absent => "absent",
            GitIndexStatus::NotInstalled => "not_installed",
            GitIndexStatus::Timeout => "timeout",
            GitIndexStatus::Unparseable => "unparseable",
        }
    }
}

pub trait GitSource: Send + Sync {
    fn head_sha(&self, root: &Path) -> Result<String, GitError>;
    fn is_ancestor(&self, root: &Path, ancestor: &str, head: &str) -> Result<bool, GitError>;
    fn log(&self, root: &Path, range: GitRange) -> Result<Vec<GitCommit>, GitError>;
}

#[derive(Debug, Clone)]
pub struct CliGitSource {
    pub bin: PathBuf,
    pub timeout_ms: u64,
}

impl Default for CliGitSource {
    fn default() -> Self {
        Self {
            bin: PathBuf::from(std::env::var("ENGRAM_GIT_BIN").unwrap_or_else(|_| "git".into())),
            timeout_ms: GIT_TIMEOUT_MS,
        }
    }
}

#[derive(Debug, Clone)]
pub struct FakeGitSource {
    pub head: String,
    pub ancestor: bool,
    pub commits: Vec<GitCommit>,
    pub head_err: Option<GitError>,
    pub log_err: Option<GitError>,
}

pub fn parse_git_log(stdout: &str) -> Result<Vec<GitCommit>, GitError> {
    if stdout.is_empty() {
        return Ok(Vec::new());
    }
    let mut commits = Vec::new();
    for chunk in stdout.split('\u{1e}') {
        if chunk.is_empty() {
            continue;
        }
        let mut parts = chunk.splitn(5, '\u{1f}');
        let Some(sha) = parts.next() else {
            continue;
        };
        let Some(author) = parts.next() else {
            continue;
        };
        let Some(authored_at) = parts.next() else {
            continue;
        };
        let Some(subject) = parts.next() else {
            continue;
        };
        let rest = parts.next().unwrap_or("");
        let (body, files) = split_body_and_files(rest);
        commits.push(GitCommit {
            sha: sha.to_string(),
            author: author.to_string(),
            authored_at: authored_at.to_string(),
            subject: subject.to_string(),
            body,
            files,
        });
    }
    if commits.is_empty() {
        Err(GitError::Unparseable)
    } else {
        Ok(commits)
    }
}

fn split_body_and_files(rest: &str) -> (String, Vec<String>) {
    let lines: Vec<&str> = rest.lines().collect();
    let mut files = Vec::new();
    let mut body_end = lines.len();
    for (i, line) in lines.iter().enumerate().rev() {
        if !line.is_empty() && !line.chars().any(char::is_whitespace) {
            files.push((*line).to_string());
            body_end = i;
        } else {
            break;
        }
    }
    files.reverse();
    let mut end = body_end;
    while end > 0 && lines[end - 1].is_empty() {
        end -= 1;
    }
    (lines[..end].join("\n"), files)
}

fn wait_with_timeout(
    child: &mut std::process::Child,
    timeout_ms: u64,
) -> Result<std::process::Output, GitError> {
    let stdout_pipe = child.stdout.take();
    let stderr_pipe = child.stderr.take();
    let stdout_thread = thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(mut pipe) = stdout_pipe {
            let _ = pipe.read_to_end(&mut buf);
        }
        buf
    });
    let stderr_thread = thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(mut pipe) = stderr_pipe {
            let _ = pipe.read_to_end(&mut buf);
        }
        buf
    });

    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        thread::sleep(Duration::from_millis(timeout_ms));
        let _ = tx.send(());
    });

    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => match rx.recv_timeout(Duration::from_millis(20)) {
                Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    let _ = stdout_thread.join();
                    let _ = stderr_thread.join();
                    return Err(GitError::Timeout);
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            },
            Err(e) => return Err(GitError::Io(e.to_string())),
        }
    };

    let stdout = stdout_thread.join().unwrap_or_default();
    let stderr = stderr_thread.join().unwrap_or_default();
    Ok(std::process::Output {
        status,
        stdout,
        stderr,
    })
}

impl CliGitSource {
    fn spawn(&self, root: &Path, args: &[&str]) -> Result<std::process::Child, GitError> {
        match Command::new(&self.bin)
            .arg("-C")
            .arg(root)
            .args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
        {
            Ok(child) => Ok(child),
            Err(e) if e.kind() == ErrorKind::NotFound => Err(GitError::NotInstalled),
            Err(e) => Err(GitError::Io(e.to_string())),
        }
    }

    fn run(&self, root: &Path, args: &[&str]) -> Result<std::process::Output, GitError> {
        let mut child = self.spawn(root, args)?;
        wait_with_timeout(&mut child, self.timeout_ms)
    }
}

impl GitSource for CliGitSource {
    fn head_sha(&self, root: &Path) -> Result<String, GitError> {
        let output = self.run(root, &["rev-parse", "HEAD"])?;
        if !output.status.success() {
            return Err(GitError::NoRepo);
        }
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    fn is_ancestor(&self, root: &Path, ancestor: &str, head: &str) -> Result<bool, GitError> {
        let output = self.run(root, &["merge-base", "--is-ancestor", ancestor, head])?;
        match output.status.code() {
            Some(0) => Ok(true),
            Some(1) => Ok(false),
            _ => Err(GitError::NoRepo),
        }
    }

    fn log(&self, root: &Path, range: GitRange) -> Result<Vec<GitCommit>, GitError> {
        let range_arg = match &range {
            GitRange::Head => "HEAD".to_string(),
            GitRange::After(sha) => format!("{sha}..HEAD"),
        };
        let output = self.run(
            root,
            &[
                "log",
                &range_arg,
                "--reverse",
                "--date=iso-strict",
                "--format=%x1e%H%x1f%an%x1f%aI%x1f%s%x1f%b",
                "--name-only",
                "--no-color",
            ],
        )?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        let parsed = parse_git_log(&stdout);
        if !output.status.success() {
            if parsed.as_ref().map(|c| c.is_empty()).unwrap_or(true) {
                return Err(GitError::Unparseable);
            }
        }
        parsed
    }
}

impl GitSource for FakeGitSource {
    fn head_sha(&self, _root: &Path) -> Result<String, GitError> {
        match &self.head_err {
            Some(err) => Err(err.clone()),
            None => Ok(self.head.clone()),
        }
    }

    fn is_ancestor(&self, _root: &Path, _ancestor: &str, _head: &str) -> Result<bool, GitError> {
        Ok(self.ancestor)
    }

    fn log(&self, _root: &Path, _range: GitRange) -> Result<Vec<GitCommit>, GitError> {
        match &self.log_err {
            Some(err) => Err(err.clone()),
            None => Ok(self.commits.clone()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn parse_two_commits_with_files() {
        let raw = format!(
            "\x1e{sha1}\x1fAda\x1f2026-01-01T00:00:00Z\x1fuse websockets\x1freplace polling\n\nsrc/ws.ts\n\x1e{sha2}\x1fAda\x1f2026-01-02T00:00:00Z\x1ffix\x1f\n\nREADME.md\n",
            sha1 = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            sha2 = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        );
        let commits = parse_git_log(&raw).unwrap();
        assert_eq!(commits.len(), 2);
        assert_eq!(commits[0].subject, "use websockets");
        assert_eq!(commits[0].body, "replace polling");
        assert_eq!(commits[0].files, vec!["src/ws.ts"]);
        assert_eq!(commits[1].files, vec!["README.md"]);
    }

    #[test]
    fn parse_empty_is_ok() {
        assert!(parse_git_log("").unwrap().is_empty());
    }

    #[test]
    fn fake_git_returns_configured_log() {
        let fake = FakeGitSource {
            head: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
            ancestor: true,
            commits: vec![GitCommit {
                sha: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
                author: "Ada".into(),
                authored_at: "2026-01-01T00:00:00Z".into(),
                subject: "s".into(),
                body: "b".into(),
                files: vec!["a.ts".into()],
            }],
            head_err: None,
            log_err: None,
        };
        let root = Path::new(".");
        assert_eq!(fake.head_sha(root).unwrap().len(), 40);
        assert!(fake.is_ancestor(root, "a", "b").unwrap());
        assert_eq!(fake.log(root, GitRange::Head).unwrap().len(), 1);
    }

    #[test]
    fn cli_missing_binary_is_not_installed() {
        let cli = CliGitSource {
            bin: PathBuf::from("/definitely/not/a/git-binary-engram-test"),
            timeout_ms: 200,
        };
        let err = cli.head_sha(Path::new(".")).unwrap_err();
        assert_eq!(err, GitError::NotInstalled);
    }
}
