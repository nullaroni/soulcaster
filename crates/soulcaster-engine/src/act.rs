//! How a Recipe becomes structured commands. No shell string, no inherited environment, and git
//! never reads the Machine's git config.

use std::path::{Path, PathBuf};

use serde_json::Value;
use soulcaster_core::command::Command;
use soulcaster_core::config::SettingValue;
use soulcaster_core::lifecycle::FailureKind;
use soulcaster_core::recipe::{
    ArchiveKind, BuiltinProbe, BuiltinStep, Effect, Probe, Step, Store, StoredType, VersionFlag,
};

use crate::record::support_dir;

pub const BREW: &str = "/opt/homebrew/bin/brew";
const PATH: &str = "/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/sbin";

pub fn base_command(home: &Path, program: &str) -> Command {
    Command::new(program)
        .env("HOME", home_string(home))
        .env("PATH", PATH)
        .cwd(home)
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
    let home_str = home_string(home);
    if let Some(rest) = path.strip_prefix("~/") {
        home.join(rest)
    } else if path == "~" {
        home.to_path_buf()
    } else {
        PathBuf::from(path.replace("~", &home_str))
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

pub fn brew_command(home: &Path, name: &str, cask: bool, admin_held: bool) -> Command {
    let mut args = vec!["install".to_string()];
    if cask {
        args.push("--cask".to_string());
    }
    args.push(name.to_string());
    let mut command = base_command(home, BREW)
        .args(args)
        .env("HOMEBREW_NO_AUTO_UPDATE", "1")
        .stall_limit(std::time::Duration::from_secs(10 * 60));
    if !admin_held {
        command = command.env("HOMEBREW_NO_SUDO", "1");
    }
    command
}

/// One thing the engine does while performing a step, in order.
pub enum Beat {
    Run(Command),
    /// The previous command's stdout must start with this SHA-256.
    Sha256(String),
    /// The previous command's output must contain this Team ID.
    Team(String),
}

pub fn step_beats(home: &Path, step: &Step, admin_held: bool) -> Vec<Beat> {
    match step {
        Step::BrewFormula { name, .. } => {
            vec![Beat::Run(brew_command(home, name, false, admin_held))]
        }
        Step::BrewCask { name, .. } => vec![Beat::Run(brew_command(home, name, true, admin_held))],
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
    let file_name = url.rsplit('/').next().unwrap_or("download");
    let archive_path = dest.join(file_name);
    let tools = support_dir(home).join("tools").join(tool).join(version);
    let mut beats = fetch_beats(home, url, &archive_path, sha256);
    beats.push(Beat::Run(extract(home, archive, &archive_path, &tools)));
    let signed = tools.join(signed_file);
    beats.push(Beat::Run(base_command(home, "/usr/bin/codesign").args([
        "-dv",
        "--verbose=2",
        &signed.to_string_lossy(),
    ])));
    beats.push(Beat::Team(team.to_string()));
    for link in links {
        if link.target.contains("..") {
            continue;
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
    // `sudo -A` is the password-helper ticket. This command runs only while the password is held;
    // the helper attaches itself. The password is never an argument or an environment entry.
    beats.push(Beat::Run(base_command(home, "/usr/sbin/installer").args([
        "-pkg",
        &pkg.to_string_lossy(),
        "-target",
        "/",
    ])));
    beats
}

fn fetch_beats(home: &Path, url: &str, dest: &Path, sha256: &str) -> Vec<Beat> {
    let parent = dest.parent().unwrap_or(dest);
    vec![
        Beat::Run(base_command(home, "/bin/mkdir").args(["-p", &parent.to_string_lossy()])),
        Beat::Run(
            base_command(home, "/usr/bin/curl")
                .args(["-fsSL", "--output", &dest.to_string_lossy(), url])
                .stall_limit(std::time::Duration::from_secs(10 * 60)),
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
        // The placeholder-file route and Apple's dialog are the base-bootstrap ticket. A failure
        // here is queued instead of opening that dialog.
        BuiltinStep::CommandLineTools => vec![Beat::Run(
            base_command(home, "/usr/sbin/softwareupdate")
                .args(["-i", "Command Line Tools for Xcode"])
                .stall_limit(std::time::Duration::from_secs(30 * 60)),
        )],
        BuiltinStep::GhHandOff => Vec::new(),
        BuiltinStep::XcodeFirstLaunch => vec![Beat::Run(
            base_command(home, "/usr/bin/xcodebuild").args(["-runFirstLaunch"]),
        )],
    }
}

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

pub fn defaults_write(home: &Path, store: &Store, value: &SettingValue) -> Option<Command> {
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
            args.push(bool_word(value).into());
        }
        StoredType::Int => {
            args.push("-int".into());
            args.push(number_word(value));
        }
        StoredType::Float => {
            args.push("-float".into());
            args.push(number_word(value));
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

fn bool_word(value: &SettingValue) -> &'static str {
    match value {
        SettingValue::Bool(true) => "true",
        _ => "false",
    }
}

fn number_word(value: &SettingValue) -> String {
    match value {
        SettingValue::Number(number) => number.to_string(),
        SettingValue::String(text) => text.clone(),
        _ => "0".to_string(),
    }
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

pub fn dock_write(home: &Path, apps: &[String]) -> Vec<Command> {
    let mut commands = vec![base_command(home, "/opt/homebrew/bin/dockutil").args([
        "--remove",
        "all",
        "--no-restart",
    ])];
    for app in apps {
        commands.push(base_command(home, "/opt/homebrew/bin/dockutil").args([
            "--add",
            app,
            "--no-restart",
        ]));
    }
    commands
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

pub fn output_has_team(stdout: &str, stderr: &str, team: &str) -> bool {
    stdout.contains(team) || stderr.contains(team)
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
    fn brew_clears_sudo_unless_the_password_is_held_and_never_auto_updates() {
        let user = brew_command(
            Path::new("/tmp/soulcaster-sandbox"),
            "ripgrep",
            false,
            false,
        );
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
    fn checksum_and_team_checks_read_the_tool_output() {
        assert!(sha_matches("abc123  tool.zip\n", "abc123"));
        assert!(!sha_matches("ffff  tool.zip\n", "abc123"));
        assert!(output_has_team("Authority=ABCDEFGHIJ", "", "ABCDEFGHIJ"));
        assert!(!output_has_team("nope", "nope", "ABCDEFGHIJ"));
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
