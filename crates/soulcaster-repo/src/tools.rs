//! Where the outside programs live and how each is started. A command's environment is complete, so
//! what's listed here is all a process gets: no inherited `GH_TOKEN`, `SUDO_ASKPASS` or proxy.

use std::path::{Path, PathBuf};
use std::time::Duration;

use soulcaster_core::command::Command;

const SAFE_PATH: &str = "/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/sbin";

/// Every `-c` the engine's git runs with. A file in a Dotfiles Repo or a person's own config can't
/// change any of these, because a command-line setting wins over all of them.
const GIT_HARDENING: &[&str] = &[
    "core.hooksPath=/dev/null",
    "core.fsmonitor=false",
    "core.sshCommand=/usr/bin/false",
    "core.pager=cat",
    "core.attributesFile=/dev/null",
    "core.excludesFile=/dev/null",
    "core.symlinks=true",
    "core.protectHFS=true",
    "core.protectNTFS=true",
    "diff.external=",
    "submodule.recurse=false",
    "fetch.recurseSubmodules=false",
    "protocol.allow=never",
    // A replace ref shows a different tree under the approved commit's id. Read the real objects.
    "core.useReplaceRefs=false",
    // A commit-graph can name another tree for a commit. Read the commit objects themselves.
    "core.commitGraph=false",
    "core.untrackedCache=false",
    "core.sparseCheckout=false",
    "core.splitIndex=false",
    "protocol.https.allow=always",
    "transfer.fsckObjects=true",
    "http.followRedirects=false",
    "safe.bareRepository=explicit",
    "commit.gpgSign=false",
    "tag.gpgSign=false",
    "gc.auto=0",
    "maintenance.auto=false",
    "advice.detachedHead=false",
    "credential.helper=",
];

/// Whether a git command may ask `gh` for the person's GitHub sign-in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GitCred {
    /// Only for `https://github.com`. `gh` hands the token to git over a pipe.
    GitHub,
    None,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tools {
    pub gh: Option<PathBuf>,
    pub git: Option<PathBuf>,
    pub curl: PathBuf,
    pub zsh: PathBuf,
    pub home: PathBuf,
}

impl Tools {
    /// What is on this Mac right now. `/usr/bin/git` isn't used: without Command Line Tools it is a
    /// stub that opens an installer window.
    pub fn for_mac(home: PathBuf) -> Self {
        let first = |paths: &[&str]| {
            paths
                .iter()
                .map(PathBuf::from)
                .find(|path: &PathBuf| path.is_file())
        };
        Self {
            gh: first(&["/opt/homebrew/bin/gh"]),
            git: first(&[
                "/Library/Developer/CommandLineTools/usr/bin/git",
                "/opt/homebrew/bin/git",
                "/Applications/Xcode.app/Contents/Developer/usr/bin/git",
            ]),
            curl: PathBuf::from("/usr/bin/curl"),
            zsh: PathBuf::from("/bin/zsh"),
            home,
        }
    }

    /// Fixed paths, so tests and the simulated window never depend on this Mac.
    pub fn fixed(home: PathBuf) -> Self {
        Self {
            gh: Some(PathBuf::from("/fake/gh")),
            git: Some(PathBuf::from("/fake/git")),
            curl: PathBuf::from("/fake/curl"),
            zsh: PathBuf::from("/fake/zsh"),
            home,
        }
    }

    fn home_text(&self) -> String {
        self.home.to_string_lossy().into_owned()
    }

    fn gh_env(&self, command: Command) -> Command {
        command
            .env("HOME", self.home_text())
            .env("PATH", SAFE_PATH)
            .env("LC_ALL", "C")
            .env("GH_HOST", "github.com")
            .env("GH_PROMPT_DISABLED", "1")
            .env("GH_NO_UPDATE_NOTIFIER", "1")
            .env("GH_NO_EXTENSION_UPDATE_NOTIFIER", "1")
            .env("GH_PAGER", "cat")
            .env("NO_COLOR", "1")
    }

    pub(crate) fn gh<I, S>(&self, args: I) -> Option<Command>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let gh = self.gh.as_ref()?;
        Some(
            self.gh_env(Command::new(gh))
                .args(args)
                .stall_limit(Duration::from_secs(60)),
        )
    }

    /// An anonymous read of the API, for when nobody is signed in or `gh` isn't here yet. `-q` comes
    /// first because curl only honours it there, and it stops a `~/.curlrc` being read.
    pub(crate) fn curl_get(&self, api_path: &str, max_bytes: usize) -> Command {
        Command::new(&self.curl)
            .args([
                "-q",
                "--silent",
                "--show-error",
                "--proto",
                "=https",
                "--proto-redir",
                "=https",
                "--max-redirs",
                "0",
                "--max-time",
                "30",
                "--max-filesize",
                &max_bytes.to_string(),
                "--header",
                "Accept: application/vnd.github+json",
                "--header",
                "User-Agent: soulcaster",
                "--output",
                "-",
                "--write-out",
                "\n%{http_code}",
                "--url",
                &format!("https://api.github.com/{api_path}"),
            ])
            .env("PATH", SAFE_PATH)
            .env("LC_ALL", "C")
            .stall_limit(Duration::from_secs(60))
    }

    /// A hardened git. `cwd` is where it runs. Add the subcommand and its arguments after this.
    pub(crate) fn git(&self, cred: GitCred, cwd: Option<&Path>) -> Option<Command> {
        let git = self.git.as_ref()?;
        let mut args: Vec<String> = Vec::new();
        for setting in GIT_HARDENING {
            args.push("-c".to_string());
            args.push((*setting).to_string());
        }
        if let (GitCred::GitHub, Some(gh)) = (cred, &self.gh) {
            args.push("-c".to_string());
            args.push(format!(
                "credential.https://github.com.helper=!'{}' auth git-credential",
                gh.to_string_lossy().replace('\'', r"'\''")
            ));
        }
        let mut command = self
            .gh_env(Command::new(git))
            .args(args)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_ATTR_NOSYSTEM", "1")
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_ALLOW_PROTOCOL", "https")
            .env("GIT_NO_REPLACE_OBJECTS", "1")
            .env("GIT_LFS_SKIP_SMUDGE", "1")
            .stall_limit(Duration::from_secs(120));
        if let Some(cwd) = cwd {
            command = command.cwd(cwd);
            if let Some(parent) = cwd.parent() {
                command = command.env("GIT_CEILING_DIRECTORIES", parent.to_string_lossy());
            }
        }
        Some(command)
    }

    /// The approved install script: `zsh -f`, so a linked `~/.zshenv` can't act on the call.
    pub(crate) fn script(&self, script: &Path, cwd: &Path) -> Command {
        Command::new(&self.zsh)
            .args(["-f", "--", &script.to_string_lossy()])
            .cwd(cwd)
            .env("HOME", self.home_text())
            .env("PATH", SAFE_PATH)
            .env("LANG", "en_US.UTF-8")
            .stall_limit(Duration::from_secs(900))
    }
}
