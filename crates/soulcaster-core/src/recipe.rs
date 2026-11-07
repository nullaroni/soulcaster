//! Recipes as data. A Recipe chooses from step and check types built into the signed engine, so no
//! field can name a program, arguments or environment variables. The JSON contract is
//! `schema/recipe-1.schema.json`; every struct here is closed so a field it doesn't name is refused.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::config::SettingValue;
use crate::lifecycle::Stage;

fn is_false(value: &bool) -> bool {
    !*value
}

/// `owner/tap` for a third-party name, or `None` for official Homebrew. A bare name always means
/// official Homebrew.
pub fn tap_of(name: &str) -> Option<String> {
    let mut parts = name.split('/');
    match (parts.next(), parts.next(), parts.next(), parts.next()) {
        (Some(owner), Some(tap), Some(_), None) => Some(format!("{owner}/{tap}")),
        _ => None,
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Recipe {
    Install(InstallRecipe),
    Setting(SettingRecipe),
    MachineValues(MachineValuesRecipe),
}

impl Recipe {
    pub fn id(&self) -> &str {
        match self {
            Self::Install(r) => &r.id,
            Self::Setting(r) => &r.id,
            Self::MachineValues(r) => &r.id,
        }
    }

    pub fn title(&self) -> &str {
        match self {
            Self::Install(r) => &r.title,
            Self::Setting(r) => &r.title,
            Self::MachineValues(r) => &r.title,
        }
    }

    pub fn requires(&self) -> &[String] {
        match self {
            Self::Install(r) => &r.requires,
            Self::Setting(r) => &r.requires,
            Self::MachineValues(r) => &r.requires,
        }
    }

    pub fn is_base(&self) -> bool {
        matches!(self, Self::Install(r) if r.base)
    }

    pub fn steps(&self) -> &[Step] {
        match self {
            Self::Install(r) => &r.steps,
            _ => &[],
        }
    }

    pub fn has_admin_step(&self) -> bool {
        self.steps().iter().any(Step::is_admin)
    }

    /// Lifecycle rule 2: an Item is in the Admin Stage only if it would reach an admin step with
    /// no open person-gate before it. `passes` answers a step's `unless` check, so Xcode stops at
    /// its gate on a first Run and reaches its admin steps once Xcode has appeared.
    pub fn stage(&self, passes: impl Fn(&Probe) -> bool) -> Stage {
        for step in self.steps() {
            if step.unless().is_some_and(&passes) {
                continue;
            }
            if step.is_admin() {
                return Stage::Admin;
            }
            if step.is_person_gate() {
                return Stage::User;
            }
        }
        Stage::User
    }

    /// Third-party taps this Recipe's steps load, each needing its own approval.
    pub fn taps(&self) -> Vec<String> {
        let mut taps: Vec<String> = self
            .steps()
            .iter()
            .filter_map(|s| s.brew_name().and_then(tap_of))
            .collect();
        taps.sort();
        taps.dedup();
        taps
    }
}

impl<'de> Deserialize<'de> for Recipe {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde::de::Error;
        let value = serde_json::Value::deserialize(deserializer)?;
        let kind = value
            .get("kind")
            .and_then(|k| k.as_str())
            .unwrap_or_default()
            .to_string();
        match kind.as_str() {
            "app" | "cli" => serde_json::from_value(value).map(Recipe::Install),
            "setting" => serde_json::from_value(value).map(Recipe::Setting),
            "machine-values" => serde_json::from_value(value).map(Recipe::MachineValues),
            other => return Err(D::Error::custom(format!("unknown recipe kind {other:?}"))),
        }
        .map_err(D::Error::custom)
    }
}

impl Serialize for Recipe {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Install(r) => r.serialize(serializer),
            Self::Setting(r) => r.serialize(serializer),
            Self::MachineValues(r) => r.serialize(serializer),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum InstallKind {
    App,
    Cli,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct InstallRecipe {
    pub id: String,
    pub kind: InstallKind,
    pub title: String,
    #[serde(default, skip_serializing_if = "is_false")]
    pub base: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub requires: Vec<String>,
    #[serde(default, rename = "minMacOS", skip_serializing_if = "Option::is_none")]
    pub min_macos: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub conflicts: Vec<Conflict>,
    pub checks: Vec<Probe>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub facts: Vec<Fact>,
    pub steps: Vec<Step>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bootstrap: Option<Bootstrap>,
}

/// If the probe passes, nothing is installed and the Item goes to the Needs-You Queue.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Conflict {
    pub probe: Probe,
    pub message: String,
}

/// Reported, and never gates an Item, creates Drift or joins the queue.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Fact {
    pub id: String,
    pub label: String,
    pub probe: Probe,
}

/// How the engine gets a tool before Homebrew exists. Only `gh` uses it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Bootstrap {
    pub version: String,
    pub url: String,
    pub sha256: String,
    pub signed_by: String,
    pub binary: String,
}

/// A read-only check answering pass, fail or unknown.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case", deny_unknown_fields)]
pub enum Probe {
    BrewFormula {
        name: String,
    },
    BrewCask {
        name: String,
    },
    AppBundle {
        #[serde(rename = "bundleId")]
        bundle_id: String,
        path: String,
    },
    PkgReceipt {
        #[serde(rename = "pkgId")]
        pkg_id: String,
    },
    File {
        path: String,
    },
    ManagedTool {
        tool: String,
        version: String,
        file: String,
    },
    ToolVersion {
        path: String,
        flag: VersionFlag,
    },
    Builtin {
        name: BuiltinProbe,
    },
    Unknowable {
        reason: String,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum VersionFlag {
    #[serde(rename = "--version")]
    DoubleDash,
    #[serde(rename = "-version")]
    SingleDash,
    #[serde(rename = "version")]
    Word,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BuiltinProbe {
    CommandLineTools,
    Homebrew,
    GhHandedOff,
    GithubSignedIn,
    XcodeReady,
}

/// The closed set of operations. Each is implemented in the engine with a fixed program, argv and
/// environment.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case", deny_unknown_fields)]
pub enum Step {
    BrewFormula {
        name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        unless: Option<Probe>,
    },
    BrewCask {
        name: String,
        #[serde(default, skip_serializing_if = "is_false")]
        admin: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        unless: Option<Probe>,
    },
    #[serde(rename_all = "camelCase")]
    Download {
        tool: String,
        version: String,
        url: String,
        sha256: String,
        archive: ArchiveKind,
        signed_by: String,
        signed_file: String,
        links: Vec<ToolLink>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        unless: Option<Probe>,
    },
    #[serde(rename_all = "camelCase")]
    PkgInstall {
        url: String,
        sha256: String,
        signed_by: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        unless: Option<Probe>,
    },
    Sdkmanager {
        packages: Vec<SdkPackage>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        unless: Option<Probe>,
    },
    AcceptLicense {
        id: String,
        license: License,
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        link: Option<String>,
        #[serde(default, skip_serializing_if = "is_false")]
        admin: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        unless: Option<Probe>,
    },
    NeedsYou {
        id: String,
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        link: Option<String>,
        completion: Completion,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        unless: Option<Probe>,
    },
    Builtin {
        name: BuiltinStep,
        #[serde(default, skip_serializing_if = "is_false")]
        admin: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        unless: Option<Probe>,
    },
}

impl Step {
    pub fn unless(&self) -> Option<&Probe> {
        match self {
            Self::BrewFormula { unless, .. }
            | Self::BrewCask { unless, .. }
            | Self::Download { unless, .. }
            | Self::PkgInstall { unless, .. }
            | Self::Sdkmanager { unless, .. }
            | Self::AcceptLicense { unless, .. }
            | Self::NeedsYou { unless, .. }
            | Self::Builtin { unless, .. } => unless.as_ref(),
        }
    }

    pub fn is_admin(&self) -> bool {
        match self {
            Self::PkgInstall { .. } => true,
            Self::BrewCask { admin, .. }
            | Self::AcceptLicense { admin, .. }
            | Self::Builtin { admin, .. } => *admin,
            _ => false,
        }
    }

    /// A step only a person can finish. Later steps wait, and other Items carry on.
    pub fn is_person_gate(&self) -> bool {
        matches!(self, Self::NeedsYou { .. } | Self::AcceptLicense { .. })
    }

    pub fn brew_name(&self) -> Option<&str> {
        match self {
            Self::BrewFormula { name, .. } | Self::BrewCask { name, .. } => Some(name),
            _ => None,
        }
    }

    /// The name on the wire, for `item.step` events and logs.
    pub fn type_name(&self) -> &'static str {
        match self {
            Self::BrewFormula { .. } => "brew-formula",
            Self::BrewCask { .. } => "brew-cask",
            Self::Download { .. } => "download",
            Self::PkgInstall { .. } => "pkg-install",
            Self::Sdkmanager { .. } => "sdkmanager",
            Self::AcceptLicense { .. } => "accept-license",
            Self::NeedsYou { .. } => "needs-you",
            Self::Builtin { .. } => "builtin",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ArchiveKind {
    #[serde(rename = "zip")]
    Zip,
    #[serde(rename = "tar.gz")]
    TarGz,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolLink {
    pub name: String,
    pub target: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SdkPackage {
    pub literal: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum License {
    AndroidSdk,
    Xcode,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", untagged)]
pub enum Completion {
    /// The person says it's done.
    Confirm { confirm: ConfirmTrue },
    /// The engine checks.
    Until { until: Probe },
}

/// `"confirm": true`, the only value the schema allows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ConfirmTrue;

impl Serialize for ConfirmTrue {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_bool(true)
    }
}

impl<'de> Deserialize<'de> for ConfirmTrue {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        match bool::deserialize(deserializer)? {
            true => Ok(ConfirmTrue),
            false => Err(serde::de::Error::custom("confirm must be true")),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BuiltinStep {
    CommandLineTools,
    GhHandOff,
    XcodeFirstLaunch,
}

/// A setting has a read and write path instead of steps.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct SettingRecipe {
    pub id: String,
    pub kind: SettingKind,
    pub title: String,
    pub value_type: ValueType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub values: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pattern: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minimum: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub maximum: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unset_means: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub requires: Vec<String>,
    pub stores: Vec<Store>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effect: Option<Effect>,
    pub evidence: Evidence,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

impl SettingRecipe {
    /// Whether a Config's value suits this setting. A refused value skips only that entry.
    pub fn check_value(&self, value: &SettingValue) -> Result<(), String> {
        let strings: Vec<&String> = match (self.value_type, value) {
            (ValueType::Boolean, SettingValue::Bool(_)) => return Ok(()),
            (ValueType::Boolean, _) => return Err("expected a boolean".to_string()),
            (ValueType::Integer, SettingValue::Number(n)) => {
                let Some(n) = n.as_i64() else {
                    return Err("expected an integer".to_string());
                };
                return match (self.minimum, self.maximum) {
                    (Some(min), Some(max)) if !(min..=max).contains(&n) => {
                        Err(format!("{n} is outside {min} to {max}"))
                    }
                    _ => Ok(()),
                };
            }
            (ValueType::Integer, _) => return Err("expected an integer".to_string()),
            (ValueType::String, SettingValue::String(s)) => vec![s],
            (ValueType::String, _) => return Err("expected a string".to_string()),
            (ValueType::StringList, SettingValue::List(list)) => list.iter().collect(),
            (ValueType::StringList, _) => return Err("expected a list".to_string()),
        };
        let pattern = self
            .pattern
            .as_deref()
            .and_then(|p| regex::Regex::new(p).ok());
        for text in strings {
            if self
                .values
                .as_ref()
                .is_some_and(|allowed| !allowed.contains(text))
            {
                return Err(format!("{text:?} isn't an allowed value"));
            }
            if pattern.as_ref().is_some_and(|p| !p.is_match(text)) {
                return Err(format!("{text:?} doesn't match"));
            }
        }
        Ok(())
    }

    pub fn is_broken(&self) -> bool {
        self.evidence.group == EvidenceGroup::Broken
    }
}

/// `"kind": "setting"`, kept so a Recipe serializes back to the schema's shape.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SettingKind {
    Setting,
}

/// `"kind": "machine-values"`, kept so a Recipe serializes back to the schema's shape.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MachineValuesKind {
    MachineValues,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ValueType {
    Boolean,
    Integer,
    String,
    StringList,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Effect {
    None,
    RestartDock,
    RestartFinder,
    RestartSystemuiserver,
    RestartControlcenter,
    RestartAffectedApps,
    LogOut,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Evidence {
    pub group: EvidenceGroup,
    pub confirmed_on: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EvidenceGroup {
    Documented,
    Probable,
    Unconfirmed,
    Broken,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case", deny_unknown_fields)]
pub enum Store {
    #[serde(rename_all = "camelCase")]
    Defaults {
        domain: DefaultsDomain,
        key: String,
        stored_type: StoredType,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        host_scope: Option<HostScope>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        encode: Option<BTreeMap<String, serde_json::Value>>,
    },
    GitConfig {
        key: GitKey,
    },
    Builtin {
        name: BuiltinStore,
    },
}

/// A closed list so a Recipe can't reach login hooks or security settings. `com.apple.spaces` is
/// left out because of the WindowServer warning in the settings research.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DefaultsDomain {
    #[serde(rename = "NSGlobalDomain")]
    Global,
    #[serde(rename = "com.apple.dock")]
    Dock,
    #[serde(rename = "com.apple.finder")]
    Finder,
    #[serde(rename = "com.apple.AppleMultitouchTrackpad")]
    Trackpad,
    #[serde(rename = "com.apple.driver.AppleBluetoothMultitouch.trackpad")]
    BluetoothTrackpad,
    #[serde(rename = "com.apple.screencapture")]
    ScreenCapture,
    #[serde(rename = "com.apple.menuextra.clock")]
    MenuClock,
    #[serde(rename = "com.apple.systemuiserver")]
    SystemUiServer,
    #[serde(rename = "com.apple.controlcenter")]
    ControlCenter,
    #[serde(rename = "com.apple.Spotlight")]
    Spotlight,
    #[serde(rename = "com.apple.WindowManager")]
    WindowManager,
    #[serde(rename = "com.apple.desktopservices")]
    DesktopServices,
    #[serde(rename = "com.apple.HIToolbox")]
    HiToolbox,
}

impl DefaultsDomain {
    /// The spelling macOS uses, which also names the domain's plist file.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Global => "NSGlobalDomain",
            Self::Dock => "com.apple.dock",
            Self::Finder => "com.apple.finder",
            Self::Trackpad => "com.apple.AppleMultitouchTrackpad",
            Self::BluetoothTrackpad => "com.apple.driver.AppleBluetoothMultitouch.trackpad",
            Self::ScreenCapture => "com.apple.screencapture",
            Self::MenuClock => "com.apple.menuextra.clock",
            Self::SystemUiServer => "com.apple.systemuiserver",
            Self::ControlCenter => "com.apple.controlcenter",
            Self::Spotlight => "com.apple.Spotlight",
            Self::WindowManager => "com.apple.WindowManager",
            Self::DesktopServices => "com.apple.desktopservices",
            Self::HiToolbox => "com.apple.HIToolbox",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum StoredType {
    Bool,
    Int,
    Float,
    String,
    StringArray,
    Dict,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum HostScope {
    Any,
    Current,
}

/// A closed list because other Git keys run programs, such as `core.sshCommand` and `alias.*`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum GitKey {
    #[serde(rename = "user.name")]
    UserName,
    #[serde(rename = "user.email")]
    UserEmail,
    #[serde(rename = "init.defaultBranch")]
    InitDefaultBranch,
    #[serde(rename = "core.excludesfile")]
    CoreExcludesFile,
    #[serde(rename = "rerere.enabled")]
    RerereEnabled,
    #[serde(rename = "push.autoSetupRemote")]
    PushAutoSetupRemote,
    #[serde(rename = "pull.rebase")]
    PullRebase,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BuiltinStore {
    DockTiles,
    LibraryVisible,
    SidebarFavorites,
}

/// Values personal to the Machine. They live in the Machine Record, never in a Config.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct MachineValuesRecipe {
    pub id: String,
    pub kind: MachineValuesKind,
    pub title: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub requires: Vec<String>,
    pub values: Vec<MachineValue>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MachineValue {
    pub name: String,
    pub label: String,
    pub store: Store,
}
