//! Writing inside a folder without ever following a symlink to do it. Each step holds an open
//! folder and works relative to it, so swapping a parent for a symlink partway through can't redirect
//! a write. Right before writing, the path is walked again and must lead to the same folder.

use std::fs::File;
use std::io::{Read, Write};
use std::os::fd::OwnedFd;
use std::path::{Path, PathBuf};

use rustix::fs::{
    AtFlags, FileType, Mode, OFlags, RawMode, fstat, mkdirat, openat, readlinkat, renameat, statat,
    symlinkat, unlinkat,
};
use rustix::io::Errno;

const FOLDER: OFlags = OFlags::RDONLY
    .union(OFlags::DIRECTORY)
    .union(OFlags::NOFOLLOW)
    .union(OFlags::CLOEXEC);

/// Why a path couldn't be used. `at` is the part of the path where it stopped.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum PathTrouble {
    SymlinkInPath {
        at: String,
    },
    NotAFolder {
        at: String,
    },
    /// It stopped being what was checked between the check and the write.
    Changed,
    Io,
}

impl From<Errno> for PathTrouble {
    fn from(_: Errno) -> Self {
        Self::Io
    }
}

/// What a link landed on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Placed {
    Linked {
        backup: Option<PathBuf>,
    },
    AlreadyLinked,
    /// A link into the repo for some other file is already there.
    AnotherLink,
}

fn identity(fd: &OwnedFd) -> Result<(u64, u64), PathTrouble> {
    let stat = fstat(fd)?;
    #[allow(clippy::useless_conversion, clippy::unnecessary_cast)]
    Ok((stat.st_dev as u64, stat.st_ino as u64))
}

/// A name in this folder with nothing at it, so a backup never replaces an earlier one.
fn free_name(folder: &OwnedFd, name: &str) -> Result<String, PathTrouble> {
    let taken = |candidate: &str| match statat(folder, candidate, AtFlags::SYMLINK_NOFOLLOW) {
        Ok(_) => Ok(true),
        Err(Errno::NOENT) => Ok(false),
        Err(_) => Err(PathTrouble::Io),
    };
    if !taken(name)? {
        return Ok(name.to_string());
    }
    for n in 1..1000 {
        let candidate = format!("{name}.{n}");
        if !taken(&candidate)? {
            return Ok(candidate);
        }
    }
    Err(PathTrouble::Io)
}

/// A folder, opened once, that every path is relative to.
pub(crate) struct SafeRoot {
    fd: OwnedFd,
    path: PathBuf,
}

impl SafeRoot {
    /// Symlinks above the root are the Mac's own business, such as `/var`, so they're resolved here
    /// and nowhere below it.
    pub(crate) fn open(path: &Path) -> Result<Self, PathTrouble> {
        let path = std::fs::canonicalize(path).map_err(|_| PathTrouble::Io)?;
        let fd = rustix::fs::open(&path, FOLDER, Mode::empty())?;
        Ok(Self { fd, path })
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    /// Opens the folder at `parts` under `base`, without following a symlink at any step.
    fn walk(
        &self,
        base: Option<&OwnedFd>,
        parts: &[&str],
        create: bool,
    ) -> Result<OwnedFd, PathTrouble> {
        let mut current: Option<OwnedFd> = None;
        for (depth, part) in parts.iter().enumerate() {
            let parent = current.as_ref().or(base).unwrap_or(&self.fd);
            let opened = match openat(parent, *part, FOLDER, Mode::empty()) {
                Err(Errno::NOENT) if create => {
                    match mkdirat(parent, *part, Mode::RWXU) {
                        Ok(()) | Err(Errno::EXIST) => {}
                        Err(_) => return Err(PathTrouble::Io),
                    }
                    openat(parent, *part, FOLDER, Mode::empty())
                }
                other => other,
            };
            let next = opened.map_err(|error| {
                let at = parts[..=depth].join("/");
                match (error, statat(parent, *part, AtFlags::SYMLINK_NOFOLLOW)) {
                    (Errno::NOENT, _) => PathTrouble::Changed,
                    (_, Ok(stat))
                        if FileType::from_raw_mode(stat.st_mode as RawMode).is_symlink() =>
                    {
                        PathTrouble::SymlinkInPath { at }
                    }
                    (Errno::LOOP | Errno::NOTDIR, _) => PathTrouble::NotAFolder { at },
                    _ => PathTrouble::Io,
                }
            })?;
            current = Some(next);
        }
        match current {
            Some(fd) => Ok(fd),
            None => {
                let fd = base.unwrap_or(&self.fd);
                Ok(rustix::io::dup(fd)?)
            }
        }
    }

    /// Makes the folder and any missing parents, each private to this user, and opens it.
    pub(crate) fn make_folder(&self, parts: &[&str]) -> Result<(), PathTrouble> {
        self.walk(None, parts, true).map(|_| ())
    }

    /// Reads a regular file, up to `limit` bytes. `None` if nothing is there.
    pub(crate) fn read_file(
        &self,
        parts: &[&str],
        limit: u64,
    ) -> Result<Option<Vec<u8>>, PathTrouble> {
        let Some((leaf, parents)) = parts.split_last() else {
            return Err(PathTrouble::Io);
        };
        let parent = match self.walk(None, parents, false) {
            Ok(fd) => fd,
            Err(PathTrouble::Changed) => return Ok(None),
            Err(other) => return Err(other),
        };
        let flags = OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
        let fd = match openat(&parent, *leaf, flags, Mode::empty()) {
            Ok(fd) => fd,
            Err(Errno::NOENT) => return Ok(None),
            Err(Errno::LOOP) => {
                return Err(PathTrouble::SymlinkInPath {
                    at: parts.join("/"),
                });
            }
            Err(_) => return Err(PathTrouble::Io),
        };
        let mut bytes = Vec::new();
        File::from(fd)
            .take(limit)
            .read_to_end(&mut bytes)
            .map_err(|_| PathTrouble::Io)?;
        Ok(Some(bytes))
    }

    /// Writes a file whole or not at all. A symlink at the path is replaced, never followed.
    pub(crate) fn write_file(
        &self,
        parts: &[&str],
        bytes: &[u8],
        mode: u32,
    ) -> Result<(), PathTrouble> {
        let Some((leaf, parents)) = parts.split_last() else {
            return Err(PathTrouble::Io);
        };
        let parent = self.walk(None, parents, true)?;
        let temp = format!(".macflayer-{}-{leaf}", std::process::id());
        let flags =
            OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC;
        // A leftover from a crash with this process's ID is ours to replace.
        let _ = unlinkat(&parent, temp.as_str(), AtFlags::empty());
        let fd = openat(
            &parent,
            temp.as_str(),
            flags,
            Mode::from_raw_mode(mode as RawMode),
        )?;
        let mut file = File::from(fd);
        let wrote = file.write_all(bytes).and_then(|()| file.sync_all());
        drop(file);
        if wrote.is_err() {
            let _ = unlinkat(&parent, temp.as_str(), AtFlags::empty());
            return Err(PathTrouble::Io);
        }
        renameat(&parent, temp.as_str(), &parent, *leaf).map_err(|_| {
            let _ = unlinkat(&parent, temp.as_str(), AtFlags::empty());
            PathTrouble::Io
        })
    }

    /// Puts a symlink at `parts`, moving whatever is there into `backup_parts` first. `repo` is the
    /// Dotfiles Repo: an existing link into it for another file is left alone, because that's
    /// another link of ours, reached by a spelling the volume treats as the same.
    pub(crate) fn place_link(
        &self,
        parts: &[&str],
        link_text: &Path,
        repo: &Path,
        backup_parts: &[&str],
        before_write: &mut dyn FnMut(),
    ) -> Result<Placed, PathTrouble> {
        let Some((leaf, parents)) = parts.split_last() else {
            return Err(PathTrouble::Io);
        };
        let parent = self.walk(None, parents, true)?;
        let found = match statat(&parent, *leaf, AtFlags::SYMLINK_NOFOLLOW) {
            Ok(stat) => Some(stat),
            Err(Errno::NOENT) => None,
            Err(_) => return Err(PathTrouble::Io),
        };
        if let Some(stat) = &found
            && FileType::from_raw_mode(stat.st_mode as RawMode).is_symlink()
        {
            let current = readlinkat(&parent, *leaf, Vec::new())?;
            let current = PathBuf::from(current.to_string_lossy().into_owned());
            if current == link_text {
                return Ok(Placed::AlreadyLinked);
            }
            if current.starts_with(repo) {
                return Ok(Placed::AnotherLink);
            }
        }
        before_write();

        // The race guard: the same folder, reached the same way, holding the same thing.
        let again = self.walk(None, parents, false)?;
        if identity(&again)? != identity(&parent)? {
            return Err(PathTrouble::Changed);
        }
        match (&found, statat(&parent, *leaf, AtFlags::SYMLINK_NOFOLLOW)) {
            (None, Err(Errno::NOENT)) => {}
            (Some(before), Ok(now)) if before.st_ino == now.st_ino => {}
            _ => return Err(PathTrouble::Changed),
        }

        let mut backup = None;
        if found.is_some() {
            let Some((backup_leaf, backup_parents)) = backup_parts.split_last() else {
                return Err(PathTrouble::Io);
            };
            let backup_parent = self.walk(None, backup_parents, true)?;
            let free = free_name(&backup_parent, backup_leaf)?;
            renameat(&parent, *leaf, &backup_parent, free.as_str())?;
            let folder = backup_parents.join("/");
            backup = Some((self.path.join(&folder).join(&free), folder, free));
        }
        if let Err(error) = symlinkat(link_text, &parent, *leaf) {
            if let Some((_, folder, free)) = &backup {
                let folder: Vec<&str> = folder.split('/').collect();
                let backup_parent = self.walk(None, &folder, false)?;
                let _ = renameat(&backup_parent, free.as_str(), &parent, *leaf);
            }
            return Err(if error == Errno::EXIST {
                PathTrouble::Changed
            } else {
                PathTrouble::Io
            });
        }
        Ok(Placed::Linked {
            backup: backup.map(|(path, _, _)| path),
        })
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::os::unix::fs::symlink;

    use super::*;

    struct Home {
        _dir: tempfile::TempDir,
        path: PathBuf,
        repo: PathBuf,
        source: PathBuf,
    }

    fn home() -> Home {
        let dir = tempfile::tempdir().unwrap();
        let path = fs::canonicalize(dir.path()).unwrap();
        let repo = path.join("repo");
        fs::create_dir_all(&repo).unwrap();
        let source = repo.join("zshrc");
        fs::write(&source, "repo file").unwrap();
        Home {
            _dir: dir,
            path,
            repo,
            source,
        }
    }

    const BACKUP: &[&str] = &["backups", "one", ".zshrc"];

    #[test]
    fn a_parent_swapped_for_a_symlink_right_before_the_write_stops_the_write() {
        let home = home();
        let outside = tempfile::tempdir().unwrap();
        let config = home.path.join(".config");
        fs::create_dir_all(&config).unwrap();
        let root = SafeRoot::open(&home.path).unwrap();
        let moved = home.path.join("moved-aside");
        let result = root.place_link(
            &[".config", "nvim"],
            &home.source,
            &home.repo,
            BACKUP,
            &mut || {
                // The attack: between the check and the write, the parent becomes a symlink.
                fs::rename(&config, &moved).unwrap();
                symlink(outside.path(), &config).unwrap();
            },
        );
        assert!(
            matches!(result, Err(PathTrouble::SymlinkInPath { .. })),
            "{result:?}"
        );
        assert_eq!(fs::read_dir(outside.path()).unwrap().count(), 0);
        assert_eq!(
            fs::read_dir(&moved).unwrap().count(),
            0,
            "nothing was written"
        );
    }

    #[test]
    fn a_parent_swapped_for_another_folder_stops_the_write() {
        let home = home();
        let config = home.path.join(".config");
        fs::create_dir_all(&config).unwrap();
        let root = SafeRoot::open(&home.path).unwrap();
        let result = root.place_link(
            &[".config", "nvim"],
            &home.source,
            &home.repo,
            BACKUP,
            &mut || {
                fs::rename(&config, home.path.join("moved-aside")).unwrap();
                fs::create_dir_all(&config).unwrap();
            },
        );
        assert_eq!(result, Err(PathTrouble::Changed));
        assert_eq!(fs::read_dir(&config).unwrap().count(), 0);
    }

    #[test]
    fn a_file_appearing_at_the_location_right_before_the_write_is_not_replaced() {
        let home = home();
        let root = SafeRoot::open(&home.path).unwrap();
        let target = home.path.join(".zshrc");
        let result = root.place_link(&[".zshrc"], &home.source, &home.repo, BACKUP, &mut || {
            fs::write(&target, "written by something else").unwrap();
        });
        assert_eq!(result, Err(PathTrouble::Changed));
        assert_eq!(
            fs::read_to_string(&target).unwrap(),
            "written by something else"
        );
    }

    #[test]
    fn a_file_swapped_for_a_symlink_right_before_the_write_is_not_backed_up_as_if_it_were_the_file()
    {
        let home = home();
        let root = SafeRoot::open(&home.path).unwrap();
        let target = home.path.join(".zshrc");
        fs::write(&target, "mine").unwrap();
        let victim = home.path.join("victim");
        fs::write(&victim, "victim").unwrap();
        let result = root.place_link(&[".zshrc"], &home.source, &home.repo, BACKUP, &mut || {
            fs::remove_file(&target).unwrap();
            symlink(&victim, &target).unwrap();
        });
        assert_eq!(result, Err(PathTrouble::Changed));
        assert_eq!(fs::read_to_string(&victim).unwrap(), "victim");
    }

    #[test]
    fn a_second_backup_of_the_same_location_keeps_the_first() {
        let home = home();
        let root = SafeRoot::open(&home.path).unwrap();
        let target = home.path.join(".zshrc");
        let mut backups = Vec::new();
        for words in ["first", "second"] {
            let _ = fs::remove_file(&target);
            fs::write(&target, words).unwrap();
            let Placed::Linked {
                backup: Some(backup),
            } = root
                .place_link(&[".zshrc"], &home.source, &home.repo, BACKUP, &mut || {})
                .unwrap()
            else {
                panic!("expected a backup");
            };
            backups.push(backup);
        }
        assert_ne!(backups[0], backups[1]);
        assert_eq!(fs::read_to_string(&backups[0]).unwrap(), "first");
        assert_eq!(fs::read_to_string(&backups[1]).unwrap(), "second");
    }

    #[test]
    fn writing_a_file_replaces_a_symlink_and_never_follows_one() {
        let home = home();
        let root = SafeRoot::open(&home.path).unwrap();
        let victim = home.path.join("victim");
        fs::write(&victim, "victim").unwrap();
        symlink(&victim, home.path.join("copy.json")).unwrap();
        root.write_file(&["copy.json"], b"new", 0o600).unwrap();
        assert_eq!(fs::read_to_string(&victim).unwrap(), "victim");
        assert_eq!(
            fs::read_to_string(home.path.join("copy.json")).unwrap(),
            "new"
        );
        assert!(
            !fs::symlink_metadata(home.path.join("copy.json"))
                .unwrap()
                .is_symlink()
        );

        symlink(home.path.join("repo"), home.path.join("linked")).unwrap();
        assert_eq!(
            root.write_file(&["linked", "x"], b"new", 0o600),
            Err(PathTrouble::SymlinkInPath {
                at: "linked".to_string()
            })
        );
        assert!(!home.repo.join("x").exists());
    }

    #[test]
    fn reading_never_follows_a_symlink() {
        let home = home();
        let root = SafeRoot::open(&home.path).unwrap();
        symlink(&home.source, home.path.join("link")).unwrap();
        assert!(matches!(
            root.read_file(&["link"], 100),
            Err(PathTrouble::SymlinkInPath { .. })
        ));
        assert_eq!(root.read_file(&["none"], 100), Ok(None));
        assert_eq!(
            root.read_file(&["repo", "zshrc"], 4).unwrap().unwrap(),
            b"repo"
        );
    }
}
