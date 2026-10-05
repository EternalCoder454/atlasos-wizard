//! The D-Bus face: `net.eterneon.atlas.WizardHelper1`, the authorization of
//! every call (polkit, then the caller's uid), the idle exit and shutdown.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use atlas_framework_system::polkit::{self, Denied};
use wizard_core::accounts::parse_passwd;
use wizard_core::choices::Value as Choice;
use zbus::message::Header;
use zbus::zvariant::{OwnedValue, Value};
use zeroize::Zeroizing;

use crate::core::{ChoiceMap, Core};
use crate::error::{DbusError, HelperError};
use crate::paths::Paths;

/// The well-known bus name.
pub const BUS_NAME: &str = "net.eterneon.atlas.WizardHelper";
/// The object path.
pub const OBJECT_PATH: &str = "/net/eterneon/atlas/WizardHelper";

/// polkit action of `CreateAccount`.
pub const ACTION_CREATE: &str = "net.eterneon.atlas.wizard.create-account";
/// polkit action of `Finish` and `EndSetup`.
pub const ACTION_FINISH: &str = "net.eterneon.atlas.wizard.finish";
/// polkit action of `GiveUp`.
pub const ACTION_FALLBACK: &str = "net.eterneon.atlas.wizard.fallback";

/// Most keys a `Finish` choices map may have (the real one has seven).
const MAX_CHOICE_KEYS: usize = 16;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

#[derive(Default)]
struct Counters {
    active: usize,
    last: Option<Instant>,
    closing: bool,
}

/// Calls in flight and the time of the last one, under one lock, so a call
/// cannot slip in between the idle check and the decision to exit.
#[derive(Default)]
pub struct Activity(Mutex<Counters>);

/// Held for as long as a call is in flight.
pub struct ActivityGuard {
    activity: Arc<Activity>,
}

impl Activity {
    /// Registers a call; `None` once the helper is shutting down.
    pub fn enter(self: &Arc<Self>) -> Option<ActivityGuard> {
        let mut c = lock(&self.0);
        if c.closing {
            return None;
        }
        c.active += 1;
        Some(ActivityGuard {
            activity: self.clone(),
        })
    }

    /// When nothing is in flight and the last call ended `timeout` ago (or the
    /// helper started that long ago), marks the helper closing and returns
    /// true; from then on [`enter`](Self::enter) refuses.
    pub fn close_if_idle(&self, timeout: Duration, started: Instant) -> bool {
        let mut c = lock(&self.0);
        if c.active == 0 && c.last.unwrap_or(started).elapsed() >= timeout {
            c.closing = true;
        }
        c.closing
    }

    /// Refuses new calls from now on (SIGTERM).
    pub fn begin_close(&self) {
        lock(&self.0).closing = true;
    }

    /// Waits until no call is in flight; false when `limit` ran out first.
    pub async fn wait_drained(&self, limit: Duration) -> bool {
        let end = Instant::now() + limit;
        while lock(&self.0).active > 0 {
            if Instant::now() >= end {
                return false;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        true
    }
}

impl Drop for ActivityGuard {
    fn drop(&mut self) {
        let mut c = lock(&self.activity.0);
        c.active = c.active.saturating_sub(1);
        c.last = Some(Instant::now());
    }
}

/// The D-Bus object.
pub struct Service {
    core: Arc<Core>,
    activity: Arc<Activity>,
}

impl Service {
    /// A service over `core`.
    pub fn new(core: Arc<Core>) -> Service {
        Service {
            core,
            activity: Arc::new(Activity::default()),
        }
    }

    /// The activity tracker (shared with [`serve`]).
    pub fn activity(&self) -> Arc<Activity> {
        self.activity.clone()
    }

    /// Counts the call, asks polkit (non-interactive), and checks that the
    /// caller is the setup user. Everything else follows only after this.
    async fn authorize(
        &self,
        method: &str,
        header: &Header<'_>,
        conn: &zbus::Connection,
        action: &str,
    ) -> Result<ActivityGuard, HelperError> {
        let guard = self.activity.enter().ok_or_else(|| {
            HelperError::failed("shutting-down", "The helper is shutting down; try again.")
        })?;
        let sender = header
            .sender()
            .map(ToString::to_string)
            .unwrap_or_else(|| "(none)".into());
        log::info!("{method}: call from {sender}");
        let r = check_caller(self.core.paths(), header, conn, action).await;
        if let Err(e) = &r {
            log::warn!("{method}: {e}");
        }
        r.map(|()| guard)
    }
}

/// polkit, then the uid of the caller against `atlas-setup`'s.
pub async fn check_caller(
    paths: &Paths,
    header: &Header<'_>,
    conn: &zbus::Connection,
    action: &str,
) -> Result<(), HelperError> {
    polkit::check(conn, header, action, false)
        .await
        .map_err(|d| match d {
            Denied::NoSender => HelperError::not_authorized("no-sender", "The call has no sender."),
            Denied::Unavailable(why) => {
                log::warn!("polkit unavailable: {why}");
                HelperError::not_authorized(
                    "polkit-unavailable",
                    "The authorization service did not answer.",
                )
            }
            Denied::NotAuthorized { .. } => {
                HelperError::not_authorized("polkit-denied", "This action is not allowed here.")
            }
        })?;

    let sender = header
        .sender()
        .ok_or_else(|| HelperError::not_authorized("no-sender", "The call has no sender."))?;
    let caller_uid = zbus::fdo::DBusProxy::new(conn)
        .await
        .map_err(|e| {
            log::warn!("cannot reach the bus daemon: {e}");
            HelperError::not_authorized("caller-unknown", "The caller could not be identified.")
        })?
        .get_connection_unix_user(sender.clone().into())
        .await
        .map_err(|e| {
            log::warn!("GetConnectionUnixUser failed: {e}");
            HelperError::not_authorized("caller-unknown", "The caller could not be identified.")
        })?;
    let setup_uid = setup_uid(paths).ok_or_else(|| {
        log::warn!("the setup user {} is not in passwd", paths.setup_user());
        HelperError::not_authorized("not-setup-user", "Only the setup user may call this.")
    })?;
    if caller_uid != setup_uid {
        return Err(HelperError::not_authorized(
            "not-setup-user",
            "Only the setup user may call this.",
        ));
    }
    Ok(())
}

/// The setup user's uid from passwd.
pub fn setup_uid(paths: &Paths) -> Option<u32> {
    let text = std::fs::read_to_string(paths.passwd()).ok()?;
    parse_passwd(&text)
        .into_iter()
        .find(|e| e.name == paths.setup_user())
        .map(|e| e.uid)
}

/// Converts a D-Bus value into a choices value (strings, booleans, numbers and
/// one level of `a{sv}`); anything else is refused.
fn to_choice(v: &Value<'_>, depth: u8) -> Option<Choice> {
    match v {
        Value::Str(s) => Some(Choice::Str(s.as_str().to_string())),
        Value::Bool(b) => Some(Choice::Bool(*b)),
        Value::F64(f) => Some(Choice::F64(*f)),
        Value::Value(inner) => to_choice(inner, depth),
        Value::Dict(d) if depth < 2 => {
            let mut out = BTreeMap::new();
            for (k, v) in d.iter() {
                let Value::Str(k) = k else { return None };
                if out.len() >= MAX_CHOICE_KEYS {
                    return None;
                }
                out.insert(k.as_str().to_string(), to_choice(v, depth + 1)?);
            }
            Some(Choice::Map(out))
        }
        _ => None,
    }
}

/// The `Finish` argument as a choices map.
///
/// # Errors
/// `Invalid` for a value of a type no choice has, or too many keys.
pub fn choices_from_dbus(m: &HashMap<String, OwnedValue>) -> Result<ChoiceMap, HelperError> {
    let bad = || HelperError::invalid("choices-bad-value", "Those choices are not allowed.");
    if m.len() > MAX_CHOICE_KEYS {
        return Err(bad());
    }
    let mut out = BTreeMap::new();
    for (k, v) in m {
        out.insert(k.clone(), to_choice(v, 0).ok_or_else(bad)?);
    }
    Ok(out)
}

#[zbus::interface(name = "net.eterneon.atlas.WizardHelper1")]
impl Service {
    /// Creates the first account; returns its uid.
    async fn create_account(
        &self,
        name: String,
        full_name: String,
        password: Vec<u8>,
        autologin: bool,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] conn: &zbus::Connection,
    ) -> Result<u32, DbusError> {
        // zeroed on every path from here on, including the early returns
        let password = Zeroizing::new(password);
        let _guard = self
            .authorize("CreateAccount", &header, conn, ACTION_CREATE)
            .await?;
        Ok(self
            .core
            .create_account(name, full_name, password, autologin)
            .await?)
    }

    /// Writes the account's settings and finishes setup.
    async fn finish(
        &self,
        choices: HashMap<String, OwnedValue>,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] conn: &zbus::Connection,
    ) -> Result<(), DbusError> {
        let _guard = self
            .authorize("Finish", &header, conn, ACTION_FINISH)
            .await?;
        let map = choices_from_dbus(&choices)?;
        Ok(self.core.finish(map).await?)
    }

    /// Restarts the display manager.
    async fn end_setup(
        &self,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] conn: &zbus::Connection,
    ) -> Result<(), DbusError> {
        let _guard = self
            .authorize("EndSetup", &header, conn, ACTION_FINISH)
            .await?;
        Ok(self.core.end_setup().await?)
    }

    /// Hands over to the text-mode fallback.
    async fn give_up(
        &self,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] conn: &zbus::Connection,
    ) -> Result<(), DbusError> {
        let _guard = self
            .authorize("GiveUp", &header, conn, ACTION_FALLBACK)
            .await?;
        Ok(self.core.give_up().await?)
    }
}

/// How long a call in flight may finish after SIGTERM.
const TERM_GRACE: Duration = Duration::from_secs(25);

/// Serves on `conn` until idle for the configured time, or SIGTERM. Either
/// way: refuse new calls, release the bus name (so the next call starts a
/// fresh helper), wait for calls in flight, return.
pub async fn serve(conn: zbus::Connection, service: Service) -> zbus::Result<()> {
    let activity = service.activity();
    let idle = service.core.paths().idle_timeout();
    let started = Instant::now();
    conn.object_server().at(OBJECT_PATH, service).await?;
    conn.request_name(BUS_NAME).await?;
    log::info!("serving {BUS_NAME}");

    let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    let tick = (idle / 4).max(Duration::from_millis(50));
    let terminated = loop {
        tokio::select! {
            _ = term.recv() => break true,
            _ = tokio::time::sleep(tick) => {
                if activity.close_if_idle(idle, started) {
                    break false;
                }
            }
        }
    };
    log::info!(
        "{}",
        if terminated {
            "SIGTERM: shutting down"
        } else {
            "idle: exiting"
        }
    );
    activity.begin_close();
    let _ = conn.release_name(BUS_NAME).await;
    let limit = if terminated {
        TERM_GRACE
    } else {
        Duration::from_secs(300)
    };
    if !activity.wait_drained(limit).await {
        log::error!("a call was still running after {limit:?}; exiting anyway");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closing_only_when_idle_and_then_refuses_calls() {
        let a = Arc::new(Activity::default());
        let start = Instant::now();
        assert!(!a.close_if_idle(Duration::from_secs(60), start));
        let g = a.enter().unwrap();
        assert!(!a.close_if_idle(Duration::ZERO, start), "a call in flight");
        drop(g);
        assert!(a.close_if_idle(Duration::ZERO, start));
        assert!(a.enter().is_none(), "refused once closing");
    }

    #[test]
    fn a_finished_call_postpones_the_exit() {
        let a = Arc::new(Activity::default());
        let start = Instant::now();
        std::thread::sleep(Duration::from_millis(60));
        drop(a.enter().unwrap());
        assert!(!a.close_if_idle(Duration::from_millis(50), start));
    }

    #[tokio::test]
    async fn wait_drained_waits_for_the_guard() {
        let a = Arc::new(Activity::default());
        let g = a.enter().unwrap();
        assert!(!a.wait_drained(Duration::from_millis(100)).await);
        drop(g);
        assert!(a.wait_drained(Duration::from_secs(1)).await);
    }

    #[test]
    fn dbus_values_convert_and_odd_ones_are_refused() {
        let ok = |v: Value<'_>| OwnedValue::try_from(v).unwrap();
        let mut kb: HashMap<String, Value<'_>> = HashMap::new();
        kb.insert("layout".into(), Value::from("de"));
        let kb = zbus::zvariant::Dict::from(kb);
        let mut m = HashMap::new();
        m.insert("look".to_string(), ok(Value::from("dark")));
        m.insert("high_contrast".to_string(), ok(Value::from(true)));
        m.insert("text_scale".to_string(), ok(Value::from(1.25f64)));
        m.insert("keyboard".to_string(), ok(Value::Dict(kb)));
        let c = choices_from_dbus(&m).unwrap();
        assert_eq!(c["look"], Choice::Str("dark".into()));
        assert_eq!(c["high_contrast"], Choice::Bool(true));
        assert_eq!(c["text_scale"], Choice::F64(1.25));
        assert!(matches!(c["keyboard"], Choice::Map(_)));

        let mut bad = HashMap::new();
        bad.insert("look".to_string(), ok(Value::from(7u32)));
        assert!(choices_from_dbus(&bad).is_err());
        let many: HashMap<String, OwnedValue> = (0..40)
            .map(|i| (format!("k{i}"), ok(Value::from(true))))
            .collect();
        assert!(choices_from_dbus(&many).is_err());
    }
}
