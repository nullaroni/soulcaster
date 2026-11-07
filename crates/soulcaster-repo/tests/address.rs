//! What a person pastes, or a Config names, before anything reaches a command line.

use soulcaster_repo::address::{AddressError, ConfigPath, DotfilesSource, GitRef, ImportTarget, RepoId};

fn refused(text: &str) -> AddressError {
    ImportTarget::parse(text).expect_err(text)
}

#[test]
fn the_ordinary_forms_are_accepted() {
    for text in [
        "example-dev/mac-setup",
        "  example-dev/mac-setup\n",
        "https://github.com/example-dev/mac-setup",
        "https://github.com/example-dev/mac-setup/",
        "https://github.com/example-dev/mac-setup.git",
        "HTTPS://GitHub.COM/example-dev/mac-setup",
        "github.com/example-dev/mac-setup",
    ] {
        let ImportTarget::Repo(repo) = ImportTarget::parse(text).expect(text) else {
            panic!("{text}");
        };
        assert_eq!(repo.full_name(), "example-dev/mac-setup", "{text}");
    }
    let ImportTarget::File { repo, git_ref, path } = ImportTarget::parse(
        "https://github.com/example-dev/mac-setup/blob/main/configs/mobile.json",
    )
    .unwrap() else {
        panic!("a file link");
    };
    assert_eq!(repo.full_name(), "example-dev/mac-setup");
    assert_eq!(git_ref.as_str(), "main");
    assert_eq!(path.as_str(), "configs/mobile.json");
}

#[test]
fn anything_but_https_is_refused() {
    for text in [
        "http://github.com/a/b",
        "ssh://git@github.com/a/b",
        "git://github.com/a/b",
        "file:///etc/passwd",
        "ftp://github.com/a/b",
        "git@github.com:a/b.git",
        "ext::sh -c evil",
    ] {
        assert!(
            matches!(
                refused(text),
                AddressError::NotHttps | AddressError::HiddenCharacters
            ),
            "{text}"
        );
    }
}

#[test]
fn credentials_in_an_address_are_refused() {
    for text in [
        "https://ghp_abcdefghijklmnop@github.com/a/b",
        "https://user:hunter2@github.com/a/b",
        "https://github.com@evil.example/a/b",
        "user:hunter2@github.com/a/b",
    ] {
        assert_eq!(refused(text), AddressError::Credentials, "{text}");
    }
}

#[test]
fn a_look_alike_host_is_refused_and_named() {
    for text in [
        "https://github.com.evil.example/a/b",
        "https://evil-github.com/a/b",
        "https://githubusercontent.com/a/b",
        "https://www.github.com/a/b",
        "https://github.co/a/b",
        "https://github.com./a/b",
        "https://github.com:8443/a/b",
        "https://gist.github.com/a/b",
        "github.com.evil.example/a/b",
    ] {
        assert!(
            matches!(refused(text), AddressError::NotGitHub { .. }),
            "{text}"
        );
    }
    // Cyrillic letters, and a zero-width character inside the host.
    assert_eq!(
        refused("https://g\u{0456}thub.com/a/b"),
        AddressError::HiddenCharacters
    );
    assert_eq!(
        refused("https://git\u{200b}hub.com/a/b"),
        AddressError::HiddenCharacters
    );
}

#[test]
fn newline_bidi_and_zero_width_characters_are_refused() {
    for text in [
        "a/b\nc",
        "a\n/b",
        "a/b\u{202e}",
        "a\u{200d}/b",
        "a/b\u{0000}",
        "a/b c",
        "a\t/b",
    ] {
        assert_eq!(refused(text), AddressError::HiddenCharacters, "{text:?}");
    }
}

#[test]
fn a_value_starting_with_a_dash_never_becomes_a_flag() {
    for text in [
        "-oProxyCommand=evil/b",
        "--upload-pack=evil/b",
        "a/--upload-pack=evil",
        "a/-b",
        "https://github.com/-a/b",
        "https://github.com/a/-b",
        "https://github.com/a/b/blob/--output=x/configs/x.json",
        "https://github.com/a/b/blob/main/-x.json",
        "https://github.com/a/b/blob/main/configs/-x.json",
    ] {
        assert!(ImportTarget::parse(text).is_err(), "{text}");
    }
    assert!(RepoId::new("-a", "b").is_err());
    assert!(RepoId::new("a", "-b").is_err());
    assert!(GitRef::parse("-b").is_none());
    assert!(ConfigPath::parse("-x.json").is_none());
}

#[test]
fn traversal_and_encoded_paths_are_refused() {
    for text in [
        "../b",
        "a/..",
        "a/.",
        "a/.git",
        "https://github.com/a/b/blob/main/../../etc/passwd.json",
        "https://github.com/a/b/blob/main/.git/config.json",
        "https://github.com/a/b/blob/main/.GIT/config.json",
        "https://github.com/a/b/blob/main/configs/%2e%2e/x.json",
        "https://github.com/a/b/blob/main/configs/x.json?raw=1",
        "https://github.com/a/b/blob/main/configs/x.json#L1",
        "https://github.com/a/b/blob/..%2f/x.json",
        "https://github.com/a%2fb/c",
        "a\\b/c",
        "https://github.com/a/b/blob/main/configs/x.txt",
        "https://github.com/a/b/tree/main",
        "https://github.com/a/b/issues",
        "a/b/c",
        "a",
        "/",
        "",
    ] {
        assert!(ImportTarget::parse(text).is_err(), "{text}");
    }
    for bad in ["..", ".", "a..b", "x.lock", "a/b", "a b", ""] {
        assert!(GitRef::parse(bad).is_none(), "{bad}");
    }
}

#[test]
fn a_dotfiles_repo_on_github_is_recognized_only_at_exactly_that_host() {
    let own = DotfilesSource::parse("example-you/dotfiles").unwrap();
    assert_eq!(own.github().unwrap().full_name(), "example-you/dotfiles");
    let url = DotfilesSource::parse("https://GitHub.com/example-you/dotfiles.git").unwrap();
    assert_eq!(url.key(), "github.com/example-you/dotfiles");
    assert_eq!(url.clone_url(), "https://github.com/example-you/dotfiles.git");

    for other in [
        "https://github.com.evil.example/example-you/dotfiles",
        "https://github.com:8443/example-you/dotfiles",
        "https://www.github.com/example-you/dotfiles",
    ] {
        let source = DotfilesSource::parse(other).unwrap();
        assert!(source.github().is_none(), "{other}");
        assert!(source.clone_url().starts_with("https://"), "{other}");
    }
    assert_eq!(
        DotfilesSource::parse("https://Git.Example.COM:8443/a/b")
            .unwrap()
            .host(),
        "git.example.com:8443"
    );
    // A longer path on GitHub isn't a repo, so it can't be counted as anyone's.
    assert!(DotfilesSource::parse("https://github.com/you/dotfiles/extra").is_none());
    assert!(DotfilesSource::parse("-a/b").is_none());
    assert!(DotfilesSource::parse("https://user@github.com/a/b").is_none());
    assert!(DotfilesSource::parse("http://github.com/a/b").is_none());
}
