//! How a Recipe becomes structured commands. No shell string, no inherited environment, and git
//! never reads the Machine's git config.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::Value;
use soulcaster_core::command::Command;
use soulcaster_core::config::SettingValue;
use soulcaster_core::lifecycle::FailureKind;
use soulcaster_core::recipe::{
    ArchiveKind, Bootstrap, BuiltinProbe, BuiltinStep, Effect, Probe, Step, Store, StoredType,
    VersionFlag,
};

use crate::record::support_dir;

pub const BREW: &str = "/opt/homebrew/bin/brew";
const PATH: &str = "/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/sbin";

/// `installer` and `xcodebuild -runFirstLaunch` print little while they work.
const QUIET_ADMIN_STEP: Duration = Duration::from_secs(20 * 60);

pub fn base_command(home: &Path, program: &str) -> Command {
    // Every command can hang. Longer steps replace this.
    Command::new(program)
        .env("HOME", home_string(home))
        .env("PATH", PATH)
        .cwd(home)
        .stall_limit(Duration::from_secs(5 * 60))
}

pub fn git_command(home: &Path) -> Command {
    base_command(home, "/usr/bin/git")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("PATH", "/usr/bin:/bin")
}

fn home_string(home: &Path) -> String {
    home.to_string_lossy().into_owned()
}

pub fn expand_home(home: &Path, path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        home.join(rest)
    } else if path == "~" {
        home.to_path_buf()
    } else {
        // Only a leading `~` means the home folder. Anywhere else it's part of a name.
        PathBuf::from(path)
    }
}

fn leaf(name: &str) -> &str {
    name.rsplit('/').next().unwrap_or(name)
}

pub fn probe_commands(home: &Path, probe: &Probe) -> Vec<Command> {
    match probe {
        Probe::Unknowable { .. } => Vec::new(),
        Probe::BrewFormula { name } => {
            vec![test_flag(
                home,
                "-d",
                &format!("/opt/homebrew/Cellar/{}", leaf(name)),
            )]
        }
        Probe::BrewCask { name } => {
            vec![test_flag(
                home,
                "-d",
                &format!("/opt/homebrew/Caskroom/{}", leaf(name)),
            )]
        }
        Probe::AppBundle { path, .. } => vec![test_flag(home, "-d", path)],
        Probe::File { path } => {
            vec![test_flag(
                home,
                "-e",
                &expand_home(home, path).to_string_lossy(),
            )]
        }
        Probe::PkgReceipt { pkg_id } => {
            vec![base_command(home, "/usr/sbin/pkgutil").args(["--pkg-info", pkg_id])]
        }
        Probe::ManagedTool {
            tool,
            version,
            file,
        } => {
            let path = support_dir(home)
                .join("tools")
                .join(tool)
                .join(version)
                .join(file);
            vec![test_flag(home, "-f", &path.to_string_lossy())]
        }
        Probe::ToolVersion { path, flag } => {
            let program = expand_home(home, path);
            vec![base_command(home, &program.to_string_lossy()).args([flag_arg(*flag)])]
        }
        Probe::Builtin { name } => match name {
            BuiltinProbe::CommandLineTools => {
                vec![test_flag(
                    home,
                    "-x",
                    "/Library/Developer/CommandLineTools/usr/bin/git",
                )]
            }
            BuiltinProbe::Homebrew => vec![test_flag(home, "-x", BREW)],
            BuiltinProbe::GhHandedOff => vec![
                test_flag(home, "-x", "/opt/homebrew/bin/gh"),
                test_flag(
                    home,
                    "!",
                    &support_dir(home).join("bootstrap-gh").to_string_lossy(),
                ),
            ],
            // Sign-in is a fact. Probing it is the gh sign-in ticket.
            BuiltinProbe::GithubSignedIn => Vec::new(),
            BuiltinProbe::XcodeReady => {
                vec![base_command(home, "/usr/bin/xcodebuild").args(["-checkFirstLaunchStatus"])]
            }
        },
    }
}

fn test_flag(home: &Path, flag: &str, path: &str) -> Command {
    if flag == "!" {
        base_command(home, "/usr/bin/test").args(["!", "-e", path])
    } else {
        base_command(home, "/usr/bin/test").args([flag, path])
    }
}

fn flag_arg(flag: VersionFlag) -> &'static str {
    match flag {
        VersionFlag::DoubleDash => "--version",
        VersionFlag::SingleDash => "-version",
        VersionFlag::Word => "version",
    }
}

pub fn brew_command(home: &Path, name: &str, cask: bool, step_is_admin: bool) -> Command {
    let mut args = vec!["install".to_string()];
    if cask {
        args.push("--cask".to_string());
    }
    args.push(name.to_string());
    let mut command = base_command(home, BREW)
        .args(args)
        .env("HOMEBREW_NO_AUTO_UPDATE", "1")
        .stall_limit(Duration::from_secs(10 * 60));
    // A formula is never privileged. Holding the password does not change that.
    if !(cask && step_is_admin) {
        command = command.env("HOMEBREW_NO_SUDO", "1");
    }
    command
}

/// A question for Homebrew that changes nothing and doesn't look for updates.
pub fn brew_read(home: &Path, args: &[&str]) -> Command {
    base_command(home, BREW)
        .args(args.iter().copied())
        .env("HOMEBREW_NO_AUTO_UPDATE", "1")
        .env("HOMEBREW_NO_ANALYTICS", "1")
        .env("HOMEBREW_NO_ENV_HINTS", "1")
        .env("HOMEBREW_NO_SUDO", "1")
}

/// Never privileged: a removal doesn't ask for the password.
pub fn brew_uninstall(home: &Path, name: &str, cask: bool) -> Command {
    let mut args = vec!["uninstall".to_string()];
    if cask {
        args.push("--cask".to_string());
    }
    args.push(name.to_string());
    base_command(home, BREW)
        .args(args)
        .env("HOMEBREW_NO_AUTO_UPDATE", "1")
        .env("HOMEBREW_NO_SUDO", "1")
        .env("HOMEBREW_NO_INSTALL_CLEANUP", "1")
        // Otherwise Homebrew also removes whatever else it thinks is unused.
        .env("HOMEBREW_NO_AUTOREMOVE", "1")
}

/// One thing the engine does while performing a step, in order.
pub enum Beat {
    /// Run through `sudo -A`, with one answer available.
    Admin(Command),
    /// An admin `brew` cask, which calls `sudo -A` itself as often as it needs.
    AdminBrew(Command),
    Run(Command),
    /// The previous command's stdout must start with this SHA-256.
    Sha256(String),
    /// The previous command's output must contain this Team ID.
    Team(String),
}

pub fn step_beats(home: &Path, step: &Step) -> Vec<Beat> {
    match step {
        Step::BrewFormula { name, .. } => {
            vec![Beat::Run(brew_command(home, name, false, false))]
        }
        Step::BrewCask { name, admin, .. } => {
            let command = brew_command(home, name, true, *admin);
            if *admin {
                // The download can take a long time, and it needs no helper. Nothing answers
                // until the install itself.
                vec![
                    Beat::Run(brew_fetch_cask(home, name)),
                    Beat::AdminBrew(command),
                ]
            } else {
                vec![Beat::Run(command)]
            }
        }
        Step::Download {
            tool,
            version,
            url,
            sha256,
            archive,
            signed_by,
            signed_file,
            links,
            ..
        } => download_beats(
            home,
            tool,
            version,
            url,
            sha256,
            *archive,
            signed_by,
            signed_file,
            links,
        ),
        Step::PkgInstall {
            url,
            sha256,
            signed_by,
            ..
        } => pkg_beats(home, url, sha256, signed_by),
        Step::Sdkmanager { packages, .. } => vec![Beat::Run(sdkmanager(home, packages))],
        Step::Builtin { name, .. } => builtin_beats(home, *name),
        Step::NeedsYou { .. } | Step::AcceptLicense { .. } => Vec::new(),
    }
}

// One argument per field of a download step. A wrapper struct would only rename that step.
#[allow(clippy::too_many_arguments)]
fn download_beats(
    home: &Path,
    tool: &str,
    version: &str,
    url: &str,
    sha256: &str,
    archive: ArchiveKind,
    team: &str,
    signed_file: &str,
    links: &[soulcaster_core::recipe::ToolLink],
) -> Vec<Beat> {
    let dest = support_dir(home).join("downloads").join(tool).join(version);
    // Extract beside the download. The tools directory is where the check looks, and a refused
    // signer must not leave a binary there for the next Run to call satisfied.
    let staging = support_dir(home).join("staging").join(tool).join(version);
    let tools = support_dir(home).join("tools").join(tool).join(version);
    let mut beats = download_into(
        home,
        url,
        sha256,
        archive,
        team,
        signed_file,
        &dest,
        &staging,
        &tools,
    );
    for link in links {
        if link.target.contains("..") {
            continue;
            // A refused or interrupted attempt must not leave files that travel with this one.
        }
        let target = tools.join(&link.target);
        let path = home.join(".local").join("bin").join(&link.name);
        beats.push(Beat::Run(
            base_command(home, "/bin/mkdir")
                .args(["-p", &home.join(".local/bin").to_string_lossy()]),
        ));
        beats.push(Beat::Run(base_command(home, "/bin/ln").args([
            "-sfn",
            &target.to_string_lossy(),
            &path.to_string_lossy(),
        ])));
    }
    beats
}

fn pkg_beats(home: &Path, url: &str, sha256: &str, team: &str) -> Vec<Beat> {
    let dest = support_dir(home).join("downloads").join("pkg");
    let file_name = url.rsplit('/').next().unwrap_or("install.pkg");
    let pkg = dest.join(file_name);
    let mut beats = fetch_beats(home, url, &pkg, sha256);
    beats.push(Beat::Run(
        base_command(home, "/usr/sbin/pkgutil").args(["--check-signature", &pkg.to_string_lossy()]),
    ));
    beats.push(Beat::Team(team.to_string()));
    beats.push(Beat::Admin(
        base_command(home, "/usr/sbin/installer")
            .args(["-pkg", &pkg.to_string_lossy(), "-target", "/"])
            .stall_limit(QUIET_ADMIN_STEP),
    ));
    beats
}

fn brew_fetch_cask(home: &Path, name: &str) -> Command {
    base_command(home, BREW)
        .args(["fetch", "--cask", name])
        .env("HOMEBREW_NO_AUTO_UPDATE", "1")
        .env("HOMEBREW_NO_SUDO", "1")
        .stall_limit(Duration::from_secs(10 * 60))
}

/// The copy on disk against its pin. Its signature seals its contents, so this covers both.
pub fn bootstrap_check_beats(home: &Path, bootstrap: &Bootstrap) -> Vec<Beat> {
    let (_, binary) = bootstrap_paths(home, bootstrap);
    vec![
        Beat::Run(signer_check(home, &bootstrap.signed_by, &binary)),
        Beat::Team(bootstrap.signed_by.clone()),
    ]
}

fn signer_check(home: &Path, team: &str, file: &Path) -> Command {
    let requirement = format!("anchor apple generic and certificate leaf[subject.OU] = \"{team}\"");
    base_command(home, "/usr/bin/codesign").args([
        "--verify",
        "--strict",
        &format!("-R={requirement}"),
        &file.to_string_lossy(),
    ])
}

/// Fetch, check the SHA-256, extract, check the signer, then swap into place.
#[allow(clippy::too_many_arguments)]
fn download_into(
    home: &Path,
    url: &str,
    sha256: &str,
    archive: ArchiveKind,
    team: &str,
    signed_file: &str,
    downloads: &Path,
    staging: &Path,
    place: &Path,
) -> Vec<Beat> {
    let file_name = url.rsplit('/').next().unwrap_or("download");
    let archive_path = downloads.join(file_name);
    let tools = place;
    let mut beats = fetch_beats(home, url, &archive_path, sha256);
    beats.push(Beat::Run(
        base_command(home, "/bin/rm").args(["-rf", &staging.to_string_lossy()]),
    ));
    beats.push(Beat::Run(
        base_command(home, "/bin/mkdir").args(["-p", &staging.to_string_lossy()]),
    ));
    beats.push(Beat::Run(extract(home, archive, &archive_path, staging)));
    let signed = staging.join(signed_file);
    beats.push(Beat::Run(signer_check(home, team, &signed)));
    beats.push(Beat::Team(team.to_string()));
    if let Some(parent) = tools.parent() {
        beats.push(Beat::Run(
            base_command(home, "/bin/mkdir").args(["-p", &parent.to_string_lossy()]),
        ));
    }
    let staging_path = staging.to_string_lossy().into_owned();
    let tools_path = tools.to_string_lossy().into_owned();
    // `mv` into an existing folder would nest there, and the check would never pass.
    beats.push(Beat::Run(
        base_command(home, "/bin/rm").args(["-rf", tools_path.as_str()]),
    ));
    beats.push(Beat::Run(
        base_command(home, "/bin/mv").args([staging_path, tools_path]),
    ));
    beats
}

fn fetch_beats(home: &Path, url: &str, dest: &Path, sha256: &str) -> Vec<Beat> {
    let parent = dest.parent().unwrap_or(dest);
    vec![
        Beat::Run(base_command(home, "/bin/mkdir").args(["-p", &parent.to_string_lossy()])),
        Beat::Run(
            base_command(home, "/usr/bin/curl")
                .args(["-fsSL", "--output", &dest.to_string_lossy(), url])
                .stall_limit(Duration::from_secs(10 * 60)),
        ),
        Beat::Run(base_command(home, "/usr/bin/shasum").args([
            "-a",
            "256",
            &dest.to_string_lossy(),
        ])),
        Beat::Sha256(sha256.to_string()),
    ]
}

fn extract(home: &Path, archive: ArchiveKind, file: &Path, dest: &Path) -> Command {
    match archive {
        ArchiveKind::Zip => base_command(home, "/usr/bin/ditto").args([
            "-x",
            "-k",
            &file.to_string_lossy(),
            &dest.to_string_lossy(),
        ]),
        ArchiveKind::TarGz => base_command(home, "/usr/bin/tar").args([
            "-xzf",
            &file.to_string_lossy(),
            "-C",
            &dest.to_string_lossy(),
        ]),
    }
}

fn sdkmanager(home: &Path, packages: &[soulcaster_core::recipe::SdkPackage]) -> Command {
    let program =
        "/opt/homebrew/share/android-commandlinetools/cmdline-tools/latest/bin/sdkmanager";
    let sdk = home.join("Library/Android/sdk");
    let mut args = vec![format!("--sdk_root={}", sdk.display())];
    args.extend(packages.iter().map(|package| package.literal.clone()));
    base_command(home, program)
        .args(args)
        .env(
            "JAVA_HOME",
            "/opt/homebrew/opt/openjdk/libexec/openjdk.jdk/Contents/Home",
        )
        .env("ANDROID_SDK_ROOT", sdk.to_string_lossy().into_owned())
}

fn builtin_beats(home: &Path, name: BuiltinStep) -> Vec<Beat> {
    match name {
        // Both of these need what a command returned, so the Run does them itself.
        BuiltinStep::CommandLineTools | BuiltinStep::GhHandOff => Vec::new(),
        BuiltinStep::XcodeFirstLaunch => vec![Beat::Admin(
            base_command(home, "/usr/bin/xcodebuild")
                .args(["-runFirstLaunch"])
                .stall_limit(QUIET_ADMIN_STEP),
        )],
    }
}

/// While this file exists, `softwareupdate -l` lists Command Line Tools.
const CLT_PLACEHOLDER: &str = "/tmp/.com.apple.dt.CommandLineTools.installondemand.in-progress";

pub fn clt_placeholder_on(home: &Path) -> Command {
    base_command(home, "/usr/bin/touch").args([CLT_PLACEHOLDER])
}

pub fn clt_placeholder_off(home: &Path) -> Command {
    base_command(home, "/bin/rm").args(["-f", CLT_PLACEHOLDER])
}

pub fn clt_list(home: &Path) -> Command {
    base_command(home, "/usr/sbin/softwareupdate")
        .args(["-l"])
        .stall_limit(Duration::from_secs(10 * 60))
}

pub fn clt_install(home: &Path, label: &str) -> Command {
    base_command(home, "/usr/sbin/softwareupdate")
        .args(["-i", label])
        .stall_limit(Duration::from_secs(30 * 60))
}

/// Opens Apple's own dialog, for when the quiet route fails.
pub fn clt_dialog(home: &Path) -> Command {
    base_command(home, "/usr/bin/xcode-select").args(["--install"])
}

/// The newest release in `softwareupdate -l`, such as `Command Line Tools for Xcode-16.4`. A beta
/// isn't a release, and anything else in the list isn't ours to install.
pub fn newest_clt_label(listing: &str) -> Option<String> {
    let version = |label: &str| -> Option<Vec<u32>> {
        let number = label.strip_prefix("Command Line Tools for Xcode-")?;
        number
            .split('.')
            .map(|part| part.parse().ok())
            .collect::<Option<Vec<u32>>>()
    };
    listing
        .lines()
        .filter_map(|line| line.trim().strip_prefix("* Label: "))
        .filter_map(|label| Some((version(label)?, label.to_string())))
        .max_by(|a, b| a.0.cmp(&b.0))
        .map(|(_, label)| label)
}

const BREW_GH: &str = "/opt/homebrew/bin/gh";

pub fn brew_gh_path() -> &'static str {
    BREW_GH
}

/// The bootstrap copy's folder, and its binary inside. Never on the person's `PATH`.
pub fn bootstrap_paths(home: &Path, bootstrap: &Bootstrap) -> (PathBuf, PathBuf) {
    let folder = support_dir(home)
        .join("bootstrap-gh")
        .join(&bootstrap.version);
    let binary = folder.join(&bootstrap.binary);
    (folder, binary)
}

pub fn bootstrap_beats(home: &Path, bootstrap: &Bootstrap) -> Vec<Beat> {
    let (folder, _) = bootstrap_paths(home, bootstrap);
    download_into(
        home,
        &bootstrap.url,
        &bootstrap.sha256,
        ArchiveKind::Zip,
        &bootstrap.signed_by,
        &bootstrap.binary,
        &support_dir(home).join("downloads").join("bootstrap-gh"),
        &support_dir(home).join("staging").join("bootstrap-gh"),
        &folder,
    )
}

/// `gh` for reading what it knows. An empty `PATH` folder, because sign-in calls `git` otherwise,
/// and on a bare Mac `/usr/bin/git` is the stub that opens Apple's dialog.
pub fn quiet_gh(home: &Path, program: &Path) -> Command {
    let empty = support_dir(home).join("empty-path");
    Command::new(program)
        .env("HOME", home_string(home))
        .env("PATH", empty.to_string_lossy())
        .env("GH_PROMPT_DISABLED", "1")
        .env("GH_TELEMETRY", "false")
        .env("GH_NO_UPDATE_NOTIFIER", "1")
        .cwd(home)
        .stall_limit(Duration::from_secs(60))
}

pub fn gh_sign_in(home: &Path, program: &Path) -> Command {
    quiet_gh(home, program)
        .args([
            "auth",
            "login",
            "--web",
            "--clipboard=false",
            "--hostname",
            "github.com",
            "--git-protocol",
            "https",
            "--skip-ssh-key",
        ])
        // It prints the code, then says nothing until the person has finished in the browser.
        .stall_limit(Duration::from_secs(15 * 60))
}

/// The one-time code in a line of `gh auth login`'s output, such as `ABCD-1234`.
pub fn gh_code_in(line: &str) -> Option<String> {
    let (_, rest) = line.split_once("one-time code:")?;
    let code = rest.split_whitespace().next()?;
    let shaped = code.len() == 9
        && code.char_indices().all(|(at, c)| {
            if at == 4 {
                c == '-'
            } else {
                c.is_ascii_uppercase() || c.is_ascii_digit()
            }
        });
    shaped.then(|| code.to_string())
}

/// GitHub's device page, and nothing else `gh` might print.
pub const GITHUB_DEVICE_URL: &str = "https://github.com/login/device";

pub fn xcode_license(home: &Path) -> Command {
    base_command(home, "/usr/bin/xcodebuild").args(["-license", "accept"])
}

pub fn android_licenses(home: &Path) -> Command {
    let mut command = sdkmanager(home, &[]);
    command.args.push("--licenses".to_string());
    command
}

pub fn restart_command(home: &Path, effect: Effect) -> Option<Command> {
    let name = match effect {
        Effect::RestartDock => "Dock",
        Effect::RestartFinder => "Finder",
        Effect::RestartSystemuiserver => "SystemUIServer",
        Effect::RestartControlcenter => "ControlCenter",
        Effect::None | Effect::RestartAffectedApps | Effect::LogOut => return None,
    };
    Some(base_command(home, "/usr/bin/killall").args([name]))
}

pub fn defaults_read(home: &Path, store: &Store) -> Option<Command> {
    let Store::Defaults {
        domain,
        key,
        host_scope,
        ..
    } = store
    else {
        return None;
    };
    let mut args = vec!["read".to_string()];
    if matches!(
        host_scope,
        Some(soulcaster_core::recipe::HostScope::Current)
    ) {
        args.push("-currentHost".to_string());
    }
    args.push(domain.as_str().to_string());
    args.push(key.clone());
    Some(base_command(home, "/usr/bin/defaults").args(args))
}

/// A store as it is on this Machine: the Catalog says `<home>` where a value holds the home folder.
pub fn with_home(home: &Path, store: &Store) -> Store {
    let mut store = store.clone();
    if let Store::Defaults {
        encode: Some(encode),
        ..
    } = &mut store
    {
        for stored in encode.values_mut() {
            if let Value::String(text) = stored {
                *text = text.replace("<home>", &home_string(home));
            }
        }
    }
    store
}

/// What's written for a yes-or-no value. `encode` can invert it, as "show" stored as "hidden" does.
pub(crate) fn stored_bool(
    value: &SettingValue,
    encode: &Option<std::collections::BTreeMap<String, Value>>,
) -> bool {
    let SettingValue::Bool(flag) = value else {
        return false;
    };
    encode
        .as_ref()
        .and_then(|map| map.get(&flag.to_string()))
        .and_then(Value::as_bool)
        .unwrap_or(*flag)
}

pub fn defaults_write(home: &Path, store: &Store, value: &SettingValue) -> Option<Command> {
    let store = &with_home(home, store);
    let Store::Defaults {
        domain,
        key,
        stored_type,
        host_scope,
        encode,
    } = store
    else {
        return None;
    };
    let mut args = vec!["write".to_string()];
    if matches!(
        host_scope,
        Some(soulcaster_core::recipe::HostScope::Current)
    ) {
        args.push("-currentHost".to_string());
    }
    args.push(domain.as_str().to_string());
    args.push(key.clone());
    match stored_type {
        StoredType::Bool => {
            args.push("-bool".into());
            args.push(
                if stored_bool(value, encode) {
                    "true"
                } else {
                    "false"
                }
                .into(),
            );
        }
        StoredType::Int => {
            args.push("-int".into());
            args.push(stored_number(value, encode));
        }
        StoredType::Float => {
            args.push("-float".into());
            args.push(stored_number(value, encode));
        }
        StoredType::String => {
            args.push("-string".into());
            args.push(encoded_string(value, encode));
        }
        StoredType::StringArray => {
            args.push("-array".into());
            if let SettingValue::List(list) = value {
                args.extend(list.iter().map(|item| encoded_one(item, encode)));
            }
        }
        StoredType::Dict => {
            args.push("-dict".into());
            if let SettingValue::List(list) = value
                && let Some(encode) = encode
            {
                for (name, encoded) in encode {
                    let on = list.iter().any(|item| item == name);
                    let word = encoded
                        .as_str()
                        .map(str::to_string)
                        .unwrap_or_else(|| encoded.to_string());
                    args.push(word);
                    args.push("-bool".into());
                    args.push(if on { "true" } else { "false" }.into());
                }
            }
        }
    }
    Some(base_command(home, "/usr/bin/defaults").args(args))
}

/// The text `defaults` stores for an int or float, including a boolean's encoded number.
pub(crate) fn stored_number(
    value: &SettingValue,
    encode: &Option<std::collections::BTreeMap<String, Value>>,
) -> String {
    let token = match value {
        SettingValue::Bool(flag) => flag.to_string(),
        SettingValue::Number(number) => number.to_string(),
        SettingValue::String(text) => text.clone(),
        SettingValue::List(list) => list.join("\n"),
    };
    encode
        .as_ref()
        .and_then(|map| map.get(&token))
        .map(|encoded| match encoded {
            Value::Number(number) => number.to_string(),
            Value::String(text) => text.clone(),
            Value::Bool(true) => "1".to_string(),
            Value::Bool(false) => "0".to_string(),
            other => other.to_string(),
        })
        .unwrap_or(match value {
            SettingValue::Bool(true) => "1".to_string(),
            SettingValue::Bool(false) => "0".to_string(),
            _ => token,
        })
}

fn encoded_string(
    value: &SettingValue,
    encode: &Option<std::collections::BTreeMap<String, Value>>,
) -> String {
    match value {
        SettingValue::String(text) => encoded_one(text, encode),
        SettingValue::Bool(value) => value.to_string(),
        SettingValue::Number(value) => value.to_string(),
        SettingValue::List(list) => list.join("\n"),
    }
}

fn encoded_one(text: &str, encode: &Option<std::collections::BTreeMap<String, Value>>) -> String {
    encode
        .as_ref()
        .and_then(|map| map.get(text))
        .and_then(Value::as_str)
        .unwrap_or(text)
        .to_string()
}

pub fn git_file(home: &Path) -> PathBuf {
    home.join(".gitconfig")
}

pub fn library_read(home: &Path) -> Command {
    base_command(home, "/bin/ls").args(["-ldO", &home.join("Library").to_string_lossy()])
}

pub fn library_write(home: &Path, visible: bool) -> Command {
    let flag = if visible { "nohidden" } else { "hidden" };
    base_command(home, "/usr/bin/chflags").args([flag, &home.join("Library").to_string_lossy()])
}

pub fn dock_list(home: &Path) -> Command {
    base_command(home, "/opt/homebrew/bin/dockutil").args(["--list"])
}

/// Takes the app tiles out one by one and adds the wanted ones in order. `--remove all` would
/// also empty the folders beside them, which a Config can't name and no Run puts back.
pub fn dock_write(home: &Path, tiles: &[String], apps: &[String]) -> Vec<Command> {
    let mut commands: Vec<Command> = tiles
        .iter()
        .map(|tile| {
            base_command(home, "/opt/homebrew/bin/dockutil").args([
                "--remove",
                tile,
                "--no-restart",
            ])
        })
        .collect();
    for app in apps {
        // No shell reads this, so nothing else would expand a `~`.
        let app = expand_home(home, app).to_string_lossy().into_owned();
        commands.push(base_command(home, "/opt/homebrew/bin/dockutil").args([
            "--add",
            &app,
            "--no-restart",
        ]));
    }
    commands
}

/// Reads of the file a profile writes, for the Machine and for this user. A file that isn't there
/// is a failed read, which means nothing forces the key.
pub fn managed_preference_reads(home: &Path, domain: &str, key: &str) -> Vec<Command> {
    let file = match domain {
        "NSGlobalDomain" => ".GlobalPreferences.plist".to_string(),
        other => format!("{other}.plist"),
    };
    let machine = Path::new("/Library/Managed Preferences");
    let mut paths = vec![machine.join(&file)];
    if let Some(user) = home.file_name() {
        paths.push(machine.join(user).join(&file));
    }
    // A dot separates a plutil key path, and keys like `com.apple.swipescrolldirection` have them.
    let key = key.replace('.', "\\.");
    paths
        .iter()
        .map(|path| {
            base_command(home, "/usr/bin/plutil").args([
                "-extract",
                &key,
                "raw",
                "-o",
                "-",
                &path.to_string_lossy(),
            ])
        })
        .collect()
}

pub fn id_command(home: &Path) -> Command {
    base_command(home, "/usr/bin/id").args(["-Gn"])
}

pub fn sw_vers(home: &Path) -> Command {
    base_command(home, "/usr/bin/sw_vers").args(["-productVersion"])
}

pub fn failure_kind(text: &str, stalled: bool) -> FailureKind {
    if stalled {
        return FailureKind::Stalled;
    }
    let lower = text.to_lowercase();
    if is_network(&lower) {
        return FailureKind::Network;
    }
    if lower.contains("404") {
        return FailureKind::NotFound;
    }
    if is_needs_admin(&lower) {
        return FailureKind::NeedsAdmin;
    }
    FailureKind::ToolExit
}

pub fn is_network(text: &str) -> bool {
    if text.lines().any(line_reports_http_5xx) {
        return true;
    }
    const MARKERS: &[&str] = &[
        "curl: (6)",
        "curl: (7)",
        "curl: (18)",
        "curl: (28)",
        "curl: (35)",
        "curl: (52)",
        "curl: (56)",
        "could not resolve host",
        "network is unreachable",
        "connection reset by peer",
        "connection refused",
        "operation timed out",
        "timed out",
        "temporary failure in name resolution",
        "502 bad gateway",
        "503 service",
        "504 gateway",
    ];
    let lower = text.to_lowercase();
    MARKERS.iter().any(|marker| lower.contains(marker))
}

/// A 5xx status reported as one: `returned error: 503`, `HTTP 503`, or `HTTP/1.1 503`. A 404 isn't
/// one of these, and neither is any other number in a tool's output, such as `removed 512 files`.
fn line_reports_http_5xx(line: &str) -> bool {
    let lower = line.to_lowercase();
    let returned_error = lower.contains("returned error:");
    let words: Vec<&str> = lower.split_whitespace().collect();
    words.windows(2).any(|pair| {
        let names_a_status = pair[0] == "http"
            || pair[0].starts_with("http/")
            || (returned_error && pair[0] == "error:");
        let digits = pair[1].trim_end_matches(|c: char| !c.is_ascii_digit());
        names_a_status
            && digits.len() == 3
            && digits.starts_with('5')
            && digits.bytes().all(|b| b.is_ascii_digit())
    })
}

fn is_needs_admin(text: &str) -> bool {
    const MARKERS: &[&str] = &[
        "homebrew_no_sudo",
        "sudo: a terminal is required",
        "sudo: a password is required",
        "need sudo",
        "must be run as root",
    ];
    MARKERS.iter().any(|marker| text.contains(marker))
}

pub fn sha_matches(stdout: &str, expected: &str) -> bool {
    stdout
        .split_whitespace()
        .next()
        .is_some_and(|got| got.eq_ignore_ascii_case(expected))
}

/// A pkg is accepted only when `pkgutil` says Apple issued the certificate and the leaf is that
/// team's Developer ID Installer. Anyone can self-sign a certificate with that name, and then the
/// status line reads `signed by untrusted certificate`.
pub fn pkg_signer_has_team(stdout: &str, team: &str) -> bool {
    let issued_by_apple = stdout.lines().any(|line| {
        line.trim()
            .starts_with("Status: signed by a developer certificate issued by Apple")
    });
    let suffix = format!("({team})");
    let leaf_is_team = stdout.lines().any(|line| {
        let line = line.trim();
        line.starts_with("1. Developer ID Installer:") && line.ends_with(&suffix)
    });
    issued_by_apple && leaf_is_team
}

/// `codesign --verify` is quiet when the requirement holds. A display that merely mentions the
/// team, or an ad-hoc signature, is not acceptance.
pub fn codesign_team_ok(stdout: &str, stderr: &str, team: &str) -> bool {
    let text = format!("{stdout}\n{stderr}");
    if text.contains("Signature=adhoc") || text.contains("TeamIdentifier=not set") {
        return false;
    }
    if text.contains("TeamIdentifier=") {
        return text
            .lines()
            .any(|line| line.trim() == format!("TeamIdentifier={team}"));
    }
    text.trim().is_empty() || !text.contains("Identifier=")
}

pub fn mask_secrets(line: &str) -> String {
    let mut result = line.to_string();
    for marker in ["ghp_", "github_pat_", "gho_", "ghu_", "ghs_", "ghr_"] {
        while let Some(start) = result.find(marker) {
            let rest = &result[start..];
            let end = rest
                .find(|ch: char| ch.is_whitespace())
                .map(|offset| start + offset)
                .unwrap_or(result.len());
            result.replace_range(start..end, "***");
        }
    }
    result
}

pub fn version_below(machine: &str, minimum: &str) -> bool {
    fn parts(value: &str) -> Vec<u64> {
        value
            .trim()
            .split('.')
            .map(|part| part.parse::<u64>().unwrap_or(0))
            .collect()
    }
    let machine = parts(machine);
    let minimum = parts(minimum);
    let width = machine.len().max(minimum.len());
    for index in 0..width {
        let left = machine.get(index).copied().unwrap_or(0);
        let right = minimum.get(index).copied().unwrap_or(0);
        if left < right {
            return true;
        }
        if left > right {
            return false;
        }
    }
    false
}

/// `SettingValue`'s display is not stable. Tests and the record use JSON instead.
pub fn setting_json(value: &SettingValue) -> Value {
    serde_json::to_value(value).unwrap_or(Value::Null)
}

#[cfg(test)]
mod tests {
    use super::*;
    use soulcaster_core::command::Command;
    use std::path::Path;
    use std::time::Duration;

    fn env_value<'a>(command: &'a Command, key: &str) -> Option<&'a str> {
        command
            .env
            .iter()
            .rev()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value.as_str())
    }

    #[test]
    fn git_never_reads_the_machine_config() {
        let command = git_command(Path::new("/tmp/soulcaster-sandbox"));
        assert_eq!(command.program, Path::new("/usr/bin/git"));
        assert_eq!(env_value(&command, "GIT_CONFIG_GLOBAL"), Some("/dev/null"));
        assert_eq!(env_value(&command, "GIT_CONFIG_NOSYSTEM"), Some("1"));
        assert!(
            command
                .env
                .iter()
                .all(|(_, value)| value != "s3cret-plover")
        );
    }

    #[test]
    fn a_formula_stays_unprivileged_even_if_a_caller_marks_the_step_admin() {
        let user = brew_command(Path::new("/tmp/soulcaster-sandbox"), "ripgrep", false, true);
        assert_eq!(env_value(&user, "HOMEBREW_NO_SUDO"), Some("1"));
        assert_eq!(env_value(&user, "HOMEBREW_NO_AUTO_UPDATE"), Some("1"));
        assert!(!user.env.iter().any(|(key, _)| key == "SUDO_ASKPASS"));

        let held = brew_command(
            Path::new("/tmp/soulcaster-sandbox"),
            "tailscale",
            true,
            true,
        );
        assert_eq!(env_value(&held, "HOMEBREW_NO_SUDO"), None);
        assert_eq!(env_value(&held, "HOMEBREW_NO_AUTO_UPDATE"), Some("1"));
        assert_eq!(held.args, ["install", "--cask", "tailscale"]);
    }

    #[test]
    fn a_brew_command_is_structured_and_carries_its_own_stall_limit() {
        let command = brew_command(
            Path::new("/tmp/soulcaster-sandbox"),
            "ripgrep",
            false,
            false,
        );
        assert_eq!(command.program, Path::new(BREW));
        assert_eq!(command.stall_limit, Some(Duration::from_secs(10 * 60)));
        assert!(command.cwd.is_some());
    }

    #[test]
    fn only_a_reported_5xx_counts_as_a_network_failure() {
        assert!(is_network(
            "curl: (22) The requested URL returned error: 503"
        ));
        assert!(is_network("gh: HTTP 502 Bad Gateway"));
        assert!(!is_network(
            "curl: (22) The requested URL returned error: 404"
        ));
        assert!(!is_network("Error: removed 512 files, then gave up"));
        assert!(!is_network("HTTP 404, tried 500 mirrors of 5000"));
        assert!(is_network("< HTTP/1.1 503 Service Unavailable"));
        assert!(!is_network("Error: 500 files could not be linked"));
    }

    #[test]
    fn a_download_swaps_into_place_only_after_the_signer_passes() {
        let beats = download_beats(
            Path::new("/tmp/soulcaster-sandbox"),
            "kotlin-lsp",
            "1",
            "https://example.com/k.zip",
            "abc",
            ArchiveKind::Zip,
            "2ZEFAR8TH3",
            "bin/k",
            &[],
        );
        let names: Vec<String> = beats
            .iter()
            .filter_map(|beat| match beat {
                Beat::Run(command) => Some(command.program.to_string_lossy().into_owned()),
                _ => None,
            })
            .collect();
        let at = |name: &str| names.iter().rposition(|n| n.ends_with(name)).unwrap();
        assert!(at("codesign") < at("rm"));
        assert!(at("rm") < at("mv"));
        let verify = beats
            .iter()
            .find_map(|beat| match beat {
                Beat::Run(command) if command.program.ends_with("codesign") => Some(command),
                _ => None,
            })
            .unwrap();
        assert!(verify.args.iter().any(|arg| {
            arg == "-R=anchor apple generic and certificate leaf[subject.OU] = \"2ZEFAR8TH3\""
        }));
        assert!(verify.args.iter().any(|arg| arg == "--strict"));
    }

    #[test]
    fn a_managed_preference_is_read_from_the_file_a_profile_writes() {
        let home = Path::new("/Users/someone");
        let reads = managed_preference_reads(home, "NSGlobalDomain", "com.apple.mouse.scaling");
        assert_eq!(
            reads[0].args,
            [
                "-extract",
                "com\\.apple\\.mouse\\.scaling",
                "raw",
                "-o",
                "-",
                "/Library/Managed Preferences/.GlobalPreferences.plist",
            ]
        );
        assert_eq!(
            reads[1].args.last().unwrap(),
            "/Library/Managed Preferences/someone/.GlobalPreferences.plist"
        );
        let dock = managed_preference_reads(home, "com.apple.dock", "autohide");
        assert_eq!(
            dock[0].args.last().unwrap(),
            "/Library/Managed Preferences/com.apple.dock.plist"
        );
    }

    #[test]
    fn checksum_and_team_checks_read_the_tool_output() {
        assert!(sha_matches("abc123  tool.zip\n", "abc123"));
        assert!(!sha_matches("ffff  tool.zip\n", "abc123"));
        let signed = "Status: signed by a developer certificate issued by Apple for distribution\n\
             Certificate Chain:\n 1. Developer ID Installer: Example (ABCDEFGHIJ)\n";
        assert!(pkg_signer_has_team(signed, "ABCDEFGHIJ"));
        assert!(!pkg_signer_has_team(signed, "ZZZZZZZZZZ"));
        assert!(!pkg_signer_has_team(
            "note: team ABCDEFGHIJ appears in the readme",
            "ABCDEFGHIJ"
        ));
        // A self-signed certificate can carry any name. Only the status line tells them apart.
        assert!(!pkg_signer_has_team(
            "Status: signed by untrusted certificate\n\
             Certificate Chain:\n 1. Developer ID Installer: Example (ABCDEFGHIJ)\n",
            "ABCDEFGHIJ"
        ));
        // Apple-issued, but the team is further up the chain than the certificate that signed it.
        assert!(!pkg_signer_has_team(
            "Status: signed by a developer certificate issued by Apple for distribution\n\
             1. Developer ID Installer: Attacker (ATTACKER00)\n\
             2. Example Intermediate (ABCDEFGHIJ)\n",
            "ABCDEFGHIJ"
        ));
        assert!(!codesign_team_ok(
            "",
            "Identifier=ABCDEFGHIJ\nSignature=adhoc\nTeamIdentifier=not set\n",
            "ABCDEFGHIJ"
        ));
        assert!(codesign_team_ok("", "", "ABCDEFGHIJ"));
    }

    #[test]
    fn token_shaped_text_is_masked_and_versions_compare_numerically() {
        assert_eq!(
            mask_secrets("token ghp_abcdefghijklmnopqrstuvwxyz"),
            "token ***"
        );
        assert!(version_below("15.6", "26"));
        assert!(!version_below("15.6.1", "15.6"));
    }
}
