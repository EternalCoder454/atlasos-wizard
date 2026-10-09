//! Everything the wizard asks of the system, behind one trait: `Real` talks
//! to the helper, systemd-localed/timedated/hostnamed and NetworkManager over
//! the system bus; `Demo` answers from canned data. Every method blocks, so
//! the backend calls them only from worker threads. Each D-Bus call has a
//! timeout (25 s; 120 s for CreateAccount).

use crate::data;
use crate::errors::{self, Fail};
use serde::Serialize;
use std::collections::{BTreeMap, HashMap};
use std::future::Future;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::time::Duration;
use wizard_core::choices::Value as Choice;
use zbus::zvariant::{OwnedObjectPath, Value};
use zeroize::Zeroizing;

pub const CALL: Duration = Duration::from_secs(25);
/// The helper calls get more than the helper's own limits, so the helper
/// answers (with its error) before the GUI gives up on a call that is still
/// working: CreateAccount and Finish have 120 s there (CREATE_TIMEOUT,
/// FINISH_TIMEOUT); EndSetup redoes up to two 20 s lock commands and then
/// restarts the display manager within 25 s.
pub const ACCOUNT: Duration = Duration::from_secs(130);
pub const FINISH: Duration = Duration::from_secs(130);
pub const END_SETUP: Duration = Duration::from_secs(75);
/// For the reads at start-up: a missing service must not hold the first page.
const PROBE: Duration = Duration::from_secs(5);

const HELPER: &str = "net.eterneon.telamon.WizardHelper";
const HELPER_PATH: &str = "/net/eterneon/telamon/WizardHelper";
const HELPER_IFACE: &str = "net.eterneon.telamon.WizardHelper1";
const NM: &str = "org.freedesktop.NetworkManager";
const ORCA: &str = "/usr/bin/orca";
const SCHEMES: &str = "/usr/share/color-schemes";
const HC_LIGHT: &str = "AtlasOSHighContrastLight";

/// What the first page needs to know.
#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Startup {
    pub skip_language: bool,
    pub skip_keyboard: bool,
    pub skip_wifi: bool,
    pub ask_hostname: bool,
    pub hostname: String,
    pub language: String,
    pub keyboard_layout: String,
    pub keyboard_variant: String,
    pub timezone: String,
    /// The name of an account already made and verified (resume after it).
    pub resume_account: String,
    pub high_contrast_available: bool,
}

/// A Wi-Fi network in range.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Network {
    pub ssid: String,
    pub strength: u8,
    pub secure: bool,
    pub device: String,
    pub ap: String,
}

pub type Res<T> = Result<T, Fail>;

pub trait System: Send + Sync + 'static {
    fn startup(&self) -> Startup;
    fn languages(&self) -> Res<Vec<String>>;
    fn layouts(&self) -> Res<Vec<data::Layout>>;
    fn zones(&self) -> Res<Vec<data::Zone>>;
    fn set_language(&self, locale: &str) -> Res<()>;
    fn set_keyboard(&self, layout: &str, variant: &str) -> Res<()>;
    fn set_timezone(&self, id: &str) -> Res<()>;
    fn set_hostname(&self, name: &str) -> Res<()>;
    fn wifi_scan(&self) -> Res<Vec<Network>>;
    fn wifi_connect(
        &self,
        device: &str,
        ap: &str,
        ssid: &str,
        password: Zeroizing<String>,
        hidden: bool,
    ) -> Res<()>;
    fn create_account(
        &self,
        name: &str,
        full: &str,
        password: Zeroizing<Vec<u8>>,
        autologin: bool,
    ) -> Res<u32>;
    fn finish(&self, choices: &BTreeMap<String, Choice>) -> Res<()>;
    fn end_setup(&self) -> Res<()>;
    fn screen_reader(&self, on: bool) -> Res<()>;
    fn high_contrast(&self, on: bool) -> Res<()>;
}

/// Runs `fut` on a runtime of its own (the caller is a worker thread), with a
/// timeout.
fn block<T>(limit: Duration, fut: impl Future<Output = Res<T>>) -> Res<T> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| Fail::new("io", format!("runtime: {e}")))?;
    rt.block_on(async {
        match tokio::time::timeout(limit, fut).await {
            Ok(r) => r,
            Err(_) => Err(Fail::new("timeout", "The system did not answer in time.")),
        }
    })
}

fn dbus(e: zbus::Error) -> Fail {
    match e {
        zbus::Error::MethodError(name, msg, _) => {
            errors::from_dbus(name.as_str(), msg.as_deref().unwrap_or(""))
        }
        other => Fail::new("unreachable", other.to_string()),
    }
}

async fn system_bus() -> Res<zbus::Connection> {
    zbus::Connection::system().await.map_err(dbus)
}

async fn proxy<'a>(
    c: &'a zbus::Connection,
    dest: &'a str,
    path: &'a str,
    iface: &'a str,
) -> Res<zbus::Proxy<'a>> {
    zbus::Proxy::new(c, dest, path, iface).await.map_err(dbus)
}

async fn call<B, R>(
    c: &zbus::Connection,
    dest: &str,
    path: &str,
    iface: &str,
    method: &str,
    body: &B,
) -> Res<R>
where
    B: serde::Serialize + zbus::zvariant::DynamicType,
    R: for<'d> zbus::zvariant::DynamicDeserialize<'d>,
{
    proxy(c, dest, path, iface)
        .await?
        .call(method, body)
        .await
        .map_err(dbus)
}

async fn prop<T>(c: &zbus::Connection, dest: &str, path: &str, iface: &str, name: &str) -> Res<T>
where
    T: TryFrom<zbus::zvariant::OwnedValue>,
    T::Error: Into<zbus::Error>,
{
    proxy(c, dest, path, iface)
        .await?
        .get_property(name)
        .await
        .map_err(dbus)
}

fn choice_value(v: &Choice) -> Value<'static> {
    match v {
        Choice::Str(s) => Value::from(s.clone()),
        Choice::Bool(b) => Value::from(*b),
        Choice::F64(f) => Value::from(*f),
        Choice::Map(m) => {
            let inner: HashMap<String, Value<'static>> = m
                .iter()
                .map(|(k, v)| (k.clone(), choice_value(v)))
                .collect();
            Value::from(zbus::zvariant::Dict::from(inner))
        }
    }
}

fn finish_body(c: &BTreeMap<String, Choice>) -> HashMap<String, Value<'static>> {
    c.iter()
        .map(|(k, v)| (k.clone(), choice_value(v)))
        .collect()
}

/// The real system. Holds the screen reader child.
#[derive(Default)]
pub struct Real {
    orca: Mutex<Option<Child>>,
    previous_scheme: Mutex<Option<String>>,
}

impl Real {
    async fn nm_wifi_device(c: &zbus::Connection) -> Res<String> {
        {
            let devs: Vec<OwnedObjectPath> = call(
                c,
                NM,
                "/org/freedesktop/NetworkManager",
                NM,
                "GetDevices",
                &(),
            )
            .await?;
            for d in devs {
                let t: u32 = prop(
                    c,
                    NM,
                    d.as_str(),
                    "org.freedesktop.NetworkManager.Device",
                    "DeviceType",
                )
                .await
                .unwrap_or(0);
                if t == 2 {
                    return Ok(d.as_str().to_string());
                }
            }
            Err(Fail::new("wifi-no-device", "No Wi-Fi adapter was found."))
        }
    }
}

fn current_scheme() -> Option<String> {
    let home = std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| Path::new(&h).join(".config")))?;
    let text = std::fs::read_to_string(home.join("kdeglobals")).ok()?;
    text.lines()
        .find_map(|l| l.strip_prefix("ColorScheme="))
        .map(|s| s.trim().to_string())
        .filter(|s| {
            s.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        })
}

fn apply_scheme(name: &str) -> Res<()> {
    let st = Command::new("/usr/bin/plasma-apply-colorscheme")
        .arg(name)
        .env("QT_QPA_PLATFORM", "offscreen")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|e| Fail::new("failed", format!("plasma-apply-colorscheme: {e}")))?;
    if st.success() {
        Ok(())
    } else {
        Err(Fail::new(
            "failed",
            format!("plasma-apply-colorscheme {name}: {st}"),
        ))
    }
}

impl System for Real {
    fn startup(&self) -> Startup {
        let loaded = wizard_core::installer_ini::load_default();
        if let Some(r) = &loaded.reason {
            log::info!("installer.ini: {r}");
        }
        let ans = loaded.answers;
        let net_full = block(PROBE, async {
            let c = system_bus().await?;
            prop::<u32>(
                &c,
                NM,
                "/org/freedesktop/NetworkManager",
                NM,
                "Connectivity",
            )
            .await
        })
        .map(|v| v == 4)
        .unwrap_or(false);
        let host = block(PROBE, async {
            let c = system_bus().await?;
            prop::<String>(
                &c,
                "org.freedesktop.hostname1",
                "/org/freedesktop/hostname1",
                "org.freedesktop.hostname1",
                "StaticHostname",
            )
            .await
        })
        .unwrap_or_else(|e| {
            log::warn!("hostnamed: {e}");
            String::new()
        });
        let resume = match wizard_core::state::load(Path::new(wizard_core::state::DEFAULT_PATH)) {
            Ok(l) => l
                .state
                .account
                .filter(|a| a.stage == wizard_core::state::Stage::Verified)
                .map(|a| a.name)
                .unwrap_or_default(),
            Err(e) => {
                log::warn!("state file: {e}");
                String::new()
            }
        };
        let s = Startup {
            skip_language: ans.skip_language(),
            skip_keyboard: ans.skip_keyboard(),
            skip_wifi: ans.wifi_may_skip() && net_full,
            ask_hostname: data::hostname_is_default(&host),
            hostname: host,
            language: ans.language.clone().unwrap_or_default(),
            keyboard_layout: ans.keyboard_layout.clone().unwrap_or_default(),
            keyboard_variant: ans.keyboard_variant.clone().unwrap_or_default(),
            timezone: data::current_zone().unwrap_or_default(),
            resume_account: resume,
            high_contrast_available: Path::new(SCHEMES)
                .join(format!("{HC_LIGHT}.colors"))
                .is_file(),
        };
        log::info!(
            "start: skip language {}, keyboard {}, wifi {}; hostname asked {}; resume account {}",
            s.skip_language,
            s.skip_keyboard,
            s.skip_wifi,
            s.ask_hostname,
            !s.resume_account.is_empty()
        );
        s
    }

    fn languages(&self) -> Res<Vec<String>> {
        Ok(data::load_locales())
    }
    fn layouts(&self) -> Res<Vec<data::Layout>> {
        data::load_layouts()
            .map_err(|e| Fail::new("list-unreadable", format!("keyboard layouts: {e}")))
    }
    fn zones(&self) -> Res<Vec<data::Zone>> {
        data::load_zones().map_err(|e| Fail::new("list-unreadable", format!("time zones: {e}")))
    }

    fn set_language(&self, locale: &str) -> Res<()> {
        if !locale.ends_with(".UTF-8")
            || locale.len() > 64
            || !locale
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "_.-@".contains(c))
        {
            return Err(Fail::new("bad-argument", "That language is not valid."));
        }
        let lang = format!("LANG={locale}");
        block(CALL, async {
            let c = system_bus().await?;
            call::<_, ()>(
                &c,
                "org.freedesktop.locale1",
                "/org/freedesktop/locale1",
                "org.freedesktop.locale1",
                "SetLocale",
                &(vec![lang.as_str()], false),
            )
            .await
        })
    }

    fn set_keyboard(&self, layout: &str, variant: &str) -> Res<()> {
        if !wizard_core::choices::xkb_layout(layout)
            || (!variant.is_empty() && !wizard_core::choices::xkb_variant(variant))
        {
            return Err(Fail::new(
                "bad-argument",
                "That keyboard layout is not valid.",
            ));
        }
        block(CALL, async {
            let c = system_bus().await?;
            call::<_, ()>(
                &c,
                "org.freedesktop.locale1",
                "/org/freedesktop/locale1",
                "org.freedesktop.locale1",
                "SetX11Keyboard",
                &(layout, "", variant, "", true, false),
            )
            .await
        })
    }

    fn set_timezone(&self, id: &str) -> Res<()> {
        if !data::zone_id_ok(id) {
            return Err(Fail::new("bad-argument", "That time zone is not valid."));
        }
        block(CALL, async {
            let c = system_bus().await?;
            call::<_, ()>(
                &c,
                "org.freedesktop.timedate1",
                "/org/freedesktop/timedate1",
                "org.freedesktop.timedate1",
                "SetTimezone",
                &(id, false),
            )
            .await
        })
    }

    fn set_hostname(&self, name: &str) -> Res<()> {
        if !data::hostname_ok(name) {
            return Err(Fail::new("bad-argument", "That host name is not valid."));
        }
        block(CALL, async {
            let c = system_bus().await?;
            let d = "org.freedesktop.hostname1";
            call::<_, ()>(
                &c,
                d,
                "/org/freedesktop/hostname1",
                d,
                "SetStaticHostname",
                &(name, false),
            )
            .await?;
            call::<_, ()>(
                &c,
                d,
                "/org/freedesktop/hostname1",
                d,
                "SetHostname",
                &(name, false),
            )
            .await
        })
    }

    fn wifi_scan(&self) -> Res<Vec<Network>> {
        block(CALL, async {
            let c = system_bus().await?;
            let dev = Real::nm_wifi_device(&c).await?;
            let wl = "org.freedesktop.NetworkManager.Device.Wireless";
            // A scan may be refused as too soon after the last; the cached list still helps.
            let _ = call::<_, ()>(
                &c,
                NM,
                &dev,
                wl,
                "RequestScan",
                &HashMap::<String, Value>::new(),
            )
            .await;
            tokio::time::sleep(Duration::from_secs(3)).await;
            let aps: Vec<OwnedObjectPath> =
                call(&c, NM, &dev, wl, "GetAllAccessPoints", &()).await?;
            let ap_i = "org.freedesktop.NetworkManager.AccessPoint";
            let mut best: HashMap<Vec<u8>, Network> = HashMap::new();
            for ap in aps.into_iter().take(200) {
                let Ok(ssid) = prop::<Vec<u8>>(&c, NM, ap.as_str(), ap_i, "Ssid").await else {
                    continue;
                };
                if ssid.is_empty() {
                    continue;
                }
                let strength: u8 = prop(&c, NM, ap.as_str(), ap_i, "Strength")
                    .await
                    .unwrap_or(0);
                let flags: u32 = prop(&c, NM, ap.as_str(), ap_i, "Flags").await.unwrap_or(0);
                let wpa: u32 = prop(&c, NM, ap.as_str(), ap_i, "WpaFlags")
                    .await
                    .unwrap_or(0);
                let rsn: u32 = prop(&c, NM, ap.as_str(), ap_i, "RsnFlags")
                    .await
                    .unwrap_or(0);
                let n = Network {
                    ssid: String::from_utf8_lossy(&ssid)
                        .chars()
                        .filter(|c| !c.is_control())
                        .collect(),
                    strength,
                    secure: flags & 1 != 0 || wpa != 0 || rsn != 0,
                    device: dev.clone(),
                    ap: ap.as_str().to_string(),
                };
                if best.get(&ssid).is_none_or(|o| o.strength < n.strength) {
                    best.insert(ssid, n);
                }
            }
            let mut v: Vec<Network> = best.into_values().collect();
            v.sort_by(|a, b| b.strength.cmp(&a.strength).then(a.ssid.cmp(&b.ssid)));
            Ok(v)
        })
    }

    fn wifi_connect(
        &self,
        device: &str,
        ap: &str,
        ssid: &str,
        password: Zeroizing<String>,
        hidden: bool,
    ) -> Res<()> {
        let path_ok = |p: &str| {
            p.starts_with("/org/freedesktop/NetworkManager/")
                && p.len() < 128
                && p.chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '/' || c == '_')
        };
        if ssid.is_empty() || ssid.len() > 32 || !path_ok(device) || !(ap == "/" || path_ok(ap)) {
            return Err(Fail::new("bad-argument", "That network is not valid."));
        }
        let (device, ap, ssid) = (device.to_string(), ap.to_string(), ssid.to_string());
        block(CALL, async move {
            let c = system_bus().await?;
            let mut conn: HashMap<&str, HashMap<&str, Value>> = HashMap::new();
            conn.insert(
                "connection",
                HashMap::from([
                    ("type", Value::from("802-11-wireless")),
                    ("id", Value::from(ssid.clone())),
                ]),
            );
            conn.insert(
                "802-11-wireless",
                HashMap::from([
                    ("ssid", Value::from(ssid.as_bytes().to_vec())),
                    ("mode", Value::from("infrastructure")),
                    ("hidden", Value::from(hidden)),
                ]),
            );
            if !password.is_empty() {
                conn.insert(
                    "802-11-wireless-security",
                    HashMap::from([
                        ("key-mgmt", Value::from("wpa-psk")),
                        // borrowed, so no second heap copy of the passphrase
                        // is left unwiped when the message is sent
                        ("psk", Value::from(password.as_str())),
                    ]),
                );
            }
            let dev = OwnedObjectPath::try_from(device.as_str())
                .map_err(|e| Fail::new("bad-argument", e.to_string()))?;
            let sp = OwnedObjectPath::try_from(ap.as_str())
                .map_err(|e| Fail::new("bad-argument", e.to_string()))?;
            let (_settings, active): (OwnedObjectPath, OwnedObjectPath) = call(
                &c,
                NM,
                "/org/freedesktop/NetworkManager",
                NM,
                "AddAndActivateConnection",
                &(conn, dev, sp),
            )
            .await?;
            // 2 = activated, 4 = deactivated (failed)
            for _ in 0..40 {
                let st: u32 = prop(
                    &c,
                    NM,
                    active.as_str(),
                    "org.freedesktop.NetworkManager.Connection.Active",
                    "State",
                )
                .await
                .unwrap_or(4);
                match st {
                    2 => return Ok(()),
                    3 | 4 => break,
                    _ => tokio::time::sleep(Duration::from_millis(500)).await,
                }
            }
            Err(Fail::new(
                "wifi-failed",
                "The network did not accept the connection.",
            ))
        })
    }

    fn create_account(
        &self,
        name: &str,
        full: &str,
        password: Zeroizing<Vec<u8>>,
        autologin: bool,
    ) -> Res<u32> {
        block(ACCOUNT, async {
            let c = system_bus().await?;
            call::<_, u32>(
                &c,
                HELPER,
                HELPER_PATH,
                HELPER_IFACE,
                "CreateAccount",
                &(name, full, password.as_slice(), autologin),
            )
            .await
        })
    }

    fn finish(&self, choices: &BTreeMap<String, Choice>) -> Res<()> {
        let body = finish_body(choices);
        block(FINISH, async {
            let c = system_bus().await?;
            call::<_, ()>(&c, HELPER, HELPER_PATH, HELPER_IFACE, "Finish", &(body,)).await
        })
    }

    fn end_setup(&self) -> Res<()> {
        block(END_SETUP, async {
            let c = system_bus().await?;
            call::<_, ()>(&c, HELPER, HELPER_PATH, HELPER_IFACE, "EndSetup", &()).await
        })
    }

    fn screen_reader(&self, on: bool) -> Res<()> {
        let mut slot = self.orca.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(mut child) = slot.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        if on {
            // fixed argv: /usr/bin/orca --replace
            let child = Command::new(ORCA)
                .arg("--replace")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .map_err(|e| Fail::new("orca-missing", format!("{ORCA}: {e}")))?;
            *slot = Some(child);
        }
        Ok(())
    }

    fn high_contrast(&self, on: bool) -> Res<()> {
        if on {
            if !Path::new(SCHEMES)
                .join(format!("{HC_LIGHT}.colors"))
                .is_file()
            {
                return Err(Fail::new(
                    "failed",
                    "The high contrast color scheme is not installed.",
                ));
            }
            let mut prev = self
                .previous_scheme
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            if prev.is_none() {
                *prev = current_scheme();
            }
            apply_scheme(HC_LIGHT)
        } else if let Some(p) = self
            .previous_scheme
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .take()
        {
            apply_scheme(&p)
        } else {
            Ok(())
        }
    }
}

impl Drop for Real {
    fn drop(&mut self) {
        if let Some(mut c) = self.orca.lock().unwrap_or_else(|p| p.into_inner()).take() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

/// Canned answers for `TELAMON_WIZARD_DEMO=1`: no helper, no services. A user
/// name of `fail` makes CreateAccount fail; any other waits 3 s first. A Wi-Fi
/// password of `wrong` fails.
pub struct Demo;

fn nap(ms: u64) {
    std::thread::sleep(Duration::from_millis(ms));
}

impl System for Demo {
    fn startup(&self) -> Startup {
        nap(300);
        Startup {
            ask_hostname: true,
            hostname: "localhost".into(),
            language: "en_US.UTF-8".into(),
            keyboard_layout: "us".into(),
            timezone: "Europe/Berlin".into(),
            high_contrast_available: true,
            ..Startup::default()
        }
    }
    fn languages(&self) -> Res<Vec<String>> {
        nap(300);
        Ok([
            "de_DE", "en_GB", "en_US", "es_ES", "fr_FR", "it_IT", "ja_JP", "nl_NL", "pl_PL",
            "pt_BR", "ru_RU", "sv_SE", "zh_CN",
        ]
        .iter()
        .map(|l| format!("{l}.UTF-8"))
        .collect())
    }
    fn layouts(&self) -> Res<Vec<data::Layout>> {
        nap(300);
        Ok(data::parse_layouts(DEMO_XKB))
    }
    fn zones(&self) -> Res<Vec<data::Zone>> {
        nap(300);
        Ok(data::parse_zones(DEMO_ZONES))
    }
    fn set_language(&self, _: &str) -> Res<()> {
        nap(200);
        Ok(())
    }
    fn set_keyboard(&self, _: &str, _: &str) -> Res<()> {
        nap(200);
        Ok(())
    }
    fn set_timezone(&self, _: &str) -> Res<()> {
        nap(200);
        Ok(())
    }
    fn set_hostname(&self, _: &str) -> Res<()> {
        nap(200);
        Ok(())
    }
    fn wifi_scan(&self) -> Res<Vec<Network>> {
        nap(1200);
        let n = |ssid: &str, strength, secure| Network {
            ssid: ssid.into(),
            strength,
            secure,
            device: "/org/freedesktop/NetworkManager/Devices/2".into(),
            ap: format!("/org/freedesktop/NetworkManager/AccessPoint/{}", ssid.len()),
        };
        Ok(vec![
            n("Home Network", 88, true),
            n("Cafe Guest", 61, false),
            n("Neighbor-5G", 47, true),
            n("Library", 22, true),
        ])
    }
    fn wifi_connect(
        &self,
        _: &str,
        _: &str,
        _: &str,
        password: Zeroizing<String>,
        _: bool,
    ) -> Res<()> {
        nap(1500);
        if password.as_str() == "wrong" {
            Err(Fail::new(
                "wifi-failed",
                "The network did not accept the connection.",
            ))
        } else {
            Ok(())
        }
    }
    fn create_account(&self, name: &str, _: &str, _: Zeroizing<Vec<u8>>, _: bool) -> Res<u32> {
        nap(3000);
        if name == "fail" {
            Err(Fail::new(
                "accounts-password-failed",
                "The password could not be set.",
            ))
        } else {
            Ok(1000)
        }
    }
    fn finish(&self, _: &BTreeMap<String, Choice>) -> Res<()> {
        nap(800);
        Ok(())
    }
    fn end_setup(&self) -> Res<()> {
        nap(300);
        Ok(())
    }
    fn screen_reader(&self, _: bool) -> Res<()> {
        Ok(())
    }
    fn high_contrast(&self, _: bool) -> Res<()> {
        Ok(())
    }
}

const DEMO_XKB: &str = "<layoutList>\
<layout><configItem><name>us</name><description>English (US)</description></configItem><variantList>\
<variant><configItem><name>dvorak</name><description>English (Dvorak)</description></configItem></variant>\
<variant><configItem><name>intl</name><description>English (US, intl., with dead keys)</description></configItem></variant></variantList></layout>\
<layout><configItem><name>gb</name><description>English (UK)</description></configItem></layout>\
<layout><configItem><name>de</name><description>German</description></configItem></layout>\
<layout><configItem><name>fr</name><description>French</description></configItem></layout>\
<layout><configItem><name>es</name><description>Spanish</description></configItem></layout>\
<layout><configItem><name>jp</name><description>Japanese</description></configItem></layout>\
</layoutList>";

const DEMO_ZONES: &str = "DE,AT\t+5230+01322\tEurope/Berlin\tGermany\nGB\t+513030-0000731\tEurope/London\t\n\
FR\t+4852+00220\tEurope/Paris\t\nUS\t+404251-0740023\tAmerica/New_York\tEastern\nUS\t+415100-0873900\tAmerica/Chicago\tCentral\n\
US\t+340308-1181434\tAmerica/Los_Angeles\tPacific\nJP\t+353916+1394441\tAsia/Tokyo\t\nIN\t+2232+08822\tAsia/Kolkata\t\n\
AU\t-3352+15113\tAustralia/Sydney\tNew South Wales\n";

#[cfg(test)]
mod tests {
    //! The checks the setters make before anything reaches the bus: a hostile
    //! string must be refused as `bad-argument`, never sent to localed,
    //! timedated, hostnamed or NetworkManager. (A valid value would try the
    //! system bus, so only refused ones are called.)
    use super::*;

    fn nasty() -> Vec<String> {
        let mut v: Vec<String> = [
            "",
            "\n",
            "a\nb",
            "a\0b",
            "..",
            "../etc",
            "/etc/passwd",
            "-x",
            "--root=/",
            "a b",
            "a;b",
            "a$(b)",
            "a`b`",
            "\u{202e}x",
            "é",
            "x=y",
            "a,b",
        ]
        .iter()
        .map(ToString::to_string)
        .collect();
        v.push("a".repeat(10 * 1024 * 1024));
        v
    }

    #[test]
    fn hostile_strings_never_reach_the_bus() {
        let real = Real::default();
        let bad = |r: Res<()>| assert_eq!(r.unwrap_err().code, "bad-argument");
        for s in nasty() {
            if !s.ends_with(".UTF-8") {
                bad(real.set_language(&s));
            }
            bad(real.set_hostname(&s));
            bad(real.set_timezone(&s));
            bad(real.set_keyboard(&s, ""));
            if !s.is_empty() {
                bad(real.set_keyboard("us", &s));
            }
        }
        // a language needs the UTF-8 suffix and plain characters
        for s in [
            "de_DE",
            "de_DE.UTF-8\n",
            "de DE.UTF-8",
            "-x.UTF-8;",
            "../.UTF-8 x",
        ] {
            bad(real.set_language(s));
        }
        bad(real.set_hostname("localhost"));
        bad(real.set_timezone("Europe//Berlin"));
        bad(real.set_timezone("../../etc/passwd"));
    }

    #[test]
    fn a_network_that_is_not_a_networkmanager_path_is_refused() {
        let real = Real::default();
        let pw = || Zeroizing::new("CANARY-wifi-pw".to_string());
        let nm = "/org/freedesktop/NetworkManager";
        for (dev, ap, ssid) in [
            ("/etc/passwd", "/", "net"),
            ("", "/", "net"),
            ("/org/freedesktop/NetworkManager/Devices/2\n", "/", "net"),
            ("/org/freedesktop/NetworkManager/../../x", "/", "net"),
            (&format!("{nm}/Devices/2"), "/etc/shadow", "net"),
            (&format!("{nm}/Devices/2"), "/", ""),
            (&format!("{nm}/Devices/2"), "/", &"s".repeat(33)),
        ] {
            let r = real.wifi_connect(dev, ap, ssid, pw(), false);
            let f = r.unwrap_err();
            assert_eq!(f.code, "bad-argument", "{dev} {ap} {ssid}");
            assert!(!f.text.contains("CANARY"));
        }
    }
}
