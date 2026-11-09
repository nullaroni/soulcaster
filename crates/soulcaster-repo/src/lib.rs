//! The Config Repo, Import and the Dotfiles Repo: everything that reads or writes GitHub, or links
//! files into a home folder. It starts every process through
//! [`CommandRunner`](soulcaster_core::command::CommandRunner), and has no GUI and no engine in it.
//!
//! Everything from a Config, a repo or a GitHub response is treated as hostile: it is checked by a
//! type in [`address`] before it reaches a command line, and shown through [`text`].

#![forbid(unsafe_code)]

pub mod address;
pub mod adopt;
pub mod copies;
pub mod dotfiles;
pub mod fake;
pub mod fleet;
pub mod github;
pub mod import;
pub mod in_run;
pub mod process;
pub mod review;
mod safe_fs;
pub mod save;
pub mod text;
pub mod tools;

/// The words that never reach a screen about GitHub or the Dotfiles Repo, from the sync design.
pub const GIT_WORDS: &[&str] = &[
    "merge",
    "conflict",
    "commit",
    "push",
    "pull",
    "rebase",
    "branch",
    "head",
    "remote",
    "origin",
    "fetch",
    "upstream",
    "stash",
    "sha",
    "diverge",
    "force",
    "clone",
    "checkout",
    "ours",
    "theirs",
    "fast-forward",
];

/// For tests of on-screen words: panics if `words` uses one of [`GIT_WORDS`].
pub fn assert_no_git_words(words: &str) {
    let lowered = words.to_lowercase();
    for word in lowered.split(|c: char| !(c.is_alphanumeric() || c == '-')) {
        assert!(
            !GIT_WORDS.contains(&word),
            "{word:?} is a Git word, in: {words}"
        );
    }
}
