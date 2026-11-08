//! Review round 3. A test here fails on this commit for the reason in its name.

use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use soulcaster_core::approval::{
    ApprovalStore, Approvals, CommitSha, DotfilesApproval, ScriptApproval,
};
use soulcaster_core::command::{Command, CommandRunner};
use soulcaster_core::lint::DEFAULT_CLONE_LOCATION;
use soulcaster_core::plan::DotfilesPlan;
use soulcaster_repo::dotfiles::{DotfilesStep, LinkOutcome, LinkRefusal, ScriptOutcome, UserStage};
use soulcaster_repo::fake::FileApprovals;
use soulcaster_repo::tools::Tools;

const URL: &str = "https://github.com/example-stranger/dotfiles.git";
const KEY: &str = "github.com/example-stranger/dotfiles";

fn git(home: &Path, cwd: &Path, args: &[&str]) -> String {
    let output = soulcaster_repo::process::runner()
        .run(
            &Command::new("/usr/bin/git")
                .args(args.iter().copied())
                .cwd(cwd)
                .env("HOME", home.display().to_string())
                .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .env("GIT_AUTHOR_NAME", "Test")
                .env("GIT_AUTHOR_EMAIL", "test@example.invalid")
                .env("GIT_COMMITTER_NAME", "Test")
                .env("GIT_COMMITTER_EMAIL", "test@example.invalid"),
            &mut |_, _| {},
        )
        .expect("git starts");
    assert!(
        output.is_success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout_text()
}

struct RealRepo {
    _dir: tempfile::TempDir,
    home: PathBuf,
    tools: Tools,
    store: FileApprovals,
    commit: CommitSha,
}

fn real_repo(files: &[(&str, &str)]) -> RealRepo {
    let dir = tempfile::tempdir().unwrap();
    let home = fs::canonicalize(dir.path()).unwrap();
    let clone = home.join("Developer/dotfiles");
    fs::create_dir_all(&clone).unwrap();
    git(&home, &clone, &["init", "--quiet", "--initial-branch=main"]);
    git(&home, &clone, &["remote", "add", "origin", URL]);
    for (path, text) in files {
        let at = clone.join(path);
        fs::create_dir_all(at.parent().unwrap()).unwrap();
        fs::write(at, text).unwrap();
    }
    git(&home, &clone, &["add", "--all"]);
    git(&home, &clone, &["commit", "--quiet", "-m", "approved"]);
    let commit = CommitSha::parse(git(&home, &clone, &["rev-parse", "HEAD"]).trim()).unwrap();
    let mut tools = Tools::fixed(home.clone());
    tools.git = Some(PathBuf::from("/usr/bin/git"));
    tools.zsh = PathBuf::from("/bin/zsh");
    let store = FileApprovals::new(home.join("approvals.json"));
    RealRepo {
        _dir: dir,
        home,
        tools,
        store,
        commit,
    }
}

fn step<'a>(repo: &'a RealRepo, runner: &'a dyn CommandRunner) -> DotfilesStep<'a> {
    DotfilesStep {
        runner,
        tools: &repo.tools,
        store: &repo.store,
        clone_location: DEFAULT_CLONE_LOCATION,
        stamp: "run-1",
    }
}

fn plan() -> DotfilesPlan {
    DotfilesPlan {
        repo: "example-stranger/dotfiles".to_string(),
        links: [("~/.zshrc".to_string(), "zsh/zshrc".to_string())].into(),
        refused_links: vec![],
        install_script: Some("setup.sh".to_string()),
    }
}

fn attempt(repo: &RealRepo, links: &DotfilesPlan) -> (LinkOutcome, ScriptOutcome, bool) {
    repo.store
        .write(&Approvals {
            dotfiles: Some(DotfilesApproval {
                repo: KEY.to_string(),
                commit: repo.commit.clone(),
                links: links.links.clone(),
            }),
            script: Some(ScriptApproval {
                repo: KEY.to_string(),
                commit: repo.commit.clone(),
                path: "setup.sh".to_string(),
                ran: false,
            }),
            ..Approvals::default()
        })
        .unwrap();
    let runner = soulcaster_repo::process::runner();
    let report = step(repo, &runner)
        .apply_links(links, UserStage::password_is_gone())
        .expect("the clone's commit id is the approved one, so linking is attempted");
    let script =
        step(repo, &runner).run_script(links, UserStage::password_is_gone(), &mut |_, _| {});
    let marker = repo.home.join("Developer/dotfiles/pwned-marker").exists();
    (report.rows[0].outcome.clone(), script, marker)
}

fn refused() -> (LinkOutcome, ScriptOutcome, bool) {
    (
        LinkOutcome::Refused(LinkRefusal::RepoChanged),
        ScriptOutcome::HeldForReview,
        false,
    )
}

fn decode_hex(text: &str) -> Vec<u8> {
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).expect("hex"))
        .collect()
}

fn sha1(bytes: &[u8]) -> [u8; 20] {
    let mut child = std::process::Command::new("/usr/bin/shasum")
        .args(["-a", "1"])
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("shasum starts");
    child
        .stdin
        .take()
        .expect("shasum stdin")
        .write_all(bytes)
        .expect("shasum accepts the bytes");
    let output = child.wait_with_output().expect("shasum finishes");
    assert!(
        output.status.success(),
        "shasum: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8(output.stdout).expect("shasum text");
    let hex = text.split_whitespace().next().expect("shasum digest");
    decode_hex(hex).try_into().expect("sha1 is 20 bytes")
}

/// The commit-graph stores a tree id for each commit. Point `commit` at `tree` and repair the
/// trailing checksum, which is the only check git applies before trusting the file.
fn point_commit_graph_at(graph: &Path, commit_hex: &str, tree_hex: &str) {
    let mut data = fs::read(graph).expect("commit-graph");
    assert_eq!(&data[..4], b"CGPH", "commit-graph signature");
    assert_eq!(data[5], 1, "this git writes a sha-1 commit-graph");
    let nchunks = usize::from(data[6]);
    let mut chunks = Vec::with_capacity(nchunks + 1);
    for i in 0..=nchunks {
        let at = 8 + i * 12;
        let id = data[at..at + 4].to_vec();
        let pos = u64::from_be_bytes(data[at + 4..at + 12].try_into().unwrap()) as usize;
        chunks.push((id, pos));
    }
    let mut ordered = chunks.clone();
    ordered.sort_by_key(|(_, pos)| *pos);
    let start_of = |name: &[u8]| ordered.iter().find(|(id, _)| id == name).expect("chunk").1;
    let end_of = |name: &[u8]| {
        let index = ordered
            .iter()
            .position(|(id, _)| id == name)
            .expect("chunk");
        ordered[index + 1].1
    };
    let width = 20;
    let oidl = start_of(b"OIDL");
    let count = (end_of(b"OIDL") - oidl) / width;
    let commit = decode_hex(commit_hex);
    let index = (0..count)
        .find(|i| data[oidl + i * width..oidl + (i + 1) * width] == commit)
        .expect("approved commit is in the graph");
    let record = start_of(b"CDAT") + index * (width + 16);
    data[record..record + width].copy_from_slice(&decode_hex(tree_hex));
    let trailer = data.len() - width;
    let digest = sha1(&data[..trailer]);
    data[trailer..].copy_from_slice(&digest);
    let mut perms = fs::metadata(graph).unwrap().permissions();
    perms.set_mode(0o644);
    fs::set_permissions(graph, perms).unwrap();
    fs::write(graph, data).unwrap();
}

/// `core.worktree` sends `status` and `ls-files` to a clean copy. `rev-parse HEAD` still prints
/// the approved commit, and the clone directory itself holds the other tree, which is what gets
/// linked and run.
#[test]
fn a_core_worktree_links_a_tree_that_is_not_the_approved_commit() {
    let repo = real_repo(&[("zsh/zshrc", "approved\n"), ("setup.sh", "exit 0\n")]);
    let clone = repo.home.join("Developer/dotfiles");
    let decoy = repo.home.join("Developer/clean-tree");
    fs::create_dir_all(decoy.join("zsh")).unwrap();
    fs::copy(clone.join("zsh/zshrc"), decoy.join("zsh/zshrc")).unwrap();
    fs::copy(clone.join("setup.sh"), decoy.join("setup.sh")).unwrap();
    let decoy = fs::canonicalize(&decoy).unwrap();
    let decoy_text = decoy.display().to_string();
    git(
        &repo.home,
        &clone,
        &["config", "core.worktree", decoy_text.as_str()],
    );
    fs::write(clone.join("zsh/zshrc"), "EVIL\n").unwrap();
    fs::write(clone.join("setup.sh"), "touch pwned-marker\n").unwrap();

    let head = git(&repo.home, &clone, &["rev-parse", "HEAD^{commit}"]);
    assert_eq!(
        head.trim(),
        repo.commit.as_str(),
        "fixture: the commit id should still be the approved one"
    );
    let top = git(&repo.home, &clone, &["rev-parse", "--show-toplevel"]);
    assert_eq!(
        fs::canonicalize(top.trim()).unwrap(),
        decoy,
        "fixture: git should be inspecting the clean copy"
    );
    assert!(
        git(&repo.home, &clone, &["status", "--porcelain=v1"]).is_empty(),
        "fixture: status should be clean"
    );
    assert_eq!(
        fs::read_to_string(clone.join("zsh/zshrc")).unwrap(),
        "EVIL\n",
        "fixture: the clone directory should hold the other tree"
    );

    let links = plan();
    let (outcome, script, marker) = attempt(&repo, &links);
    assert_eq!(
        (outcome.clone(), script, marker),
        refused(),
        "core.worktree hid a different tree at {}: linked {outcome:?}, script {script:?}, marker {marker}",
        repo.commit
    );
}

/// A commit-graph can name a different tree for the approved commit. `status` trusts it, so the
/// index looks clean, while `rev-parse HEAD` still prints the approved id. Turning the graph off
/// shows the staged tree that was linked and run.
#[test]
fn a_commit_graph_links_a_tree_that_is_not_the_approved_commit() {
    let repo = real_repo(&[("zsh/zshrc", "approved\n"), ("setup.sh", "exit 0\n")]);
    let clone = repo.home.join("Developer/dotfiles");
    fs::write(clone.join("zsh/zshrc"), "EVIL\n").unwrap();
    fs::write(clone.join("setup.sh"), "touch pwned-marker\n").unwrap();
    git(&repo.home, &clone, &["add", "--all"]);
    git(
        &repo.home,
        &clone,
        &["commit", "--quiet", "-m", "not approved"],
    );
    let evil_tree = git(&repo.home, &clone, &["rev-parse", "HEAD^{tree}"]);
    git(
        &repo.home,
        &clone,
        &["reset", "--soft", "--quiet", repo.commit.as_str()],
    );
    git(
        &repo.home,
        &clone,
        &["commit-graph", "write", "--reachable"],
    );
    point_commit_graph_at(
        &clone.join(".git/objects/info/commit-graph"),
        repo.commit.as_str(),
        evil_tree.trim(),
    );

    let head = git(&repo.home, &clone, &["rev-parse", "HEAD^{commit}"]);
    assert_eq!(
        head.trim(),
        repo.commit.as_str(),
        "fixture: the commit id should still be the approved one"
    );
    assert_eq!(
        fs::read_to_string(clone.join("setup.sh")).unwrap(),
        "touch pwned-marker\n",
        "fixture: the worktree should be the other tree"
    );
    assert!(
        git(&repo.home, &clone, &["status", "--porcelain=v1"]).is_empty(),
        "fixture: status should trust the commit-graph"
    );
    let real = git(
        &repo.home,
        &clone,
        &["-c", "core.commitGraph=false", "status", "--porcelain=v1"],
    );
    assert!(
        !real.is_empty(),
        "fixture: without the graph, this tree is not the approved commit"
    );
    assert!(
        git(&repo.home, &clone, &["ls-files", "-v"])
            .lines()
            .all(|line| line.starts_with("H ")),
        "fixture: the index tags should all be plain"
    );

    let links = plan();
    let (outcome, script, marker) = attempt(&repo, &links);
    assert_eq!(
        (outcome.clone(), script, marker),
        refused(),
        "commit-graph hid a different tree at {}: linked {outcome:?}, script {script:?}, marker {marker}",
        repo.commit
    );
}
