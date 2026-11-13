//! Narrow physical dock actions. Embedded storage and arbitrary patches are not
//! part of this API. The UC worker serializes mutations; scheduler admission is
//! atomic and refuses busy work rather than replaying it later.
use crate::core::sync::MutexExt;
use super::{
    app::App,
    error::{CoreError, CoreResult},
    scheduler::{Priority, Res, Resources, TaskSpec},
};
use serde::Deserialize;
use serde_json::{json, Value};

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum DockAction {
    ThemeSelect { theme_id: String },
    ThemeNext {},
    ThemePrevious {},
    Live { mode: LiveMode },
    BrightnessSet { value: u8 },
    BrightnessStep { delta: i16 },
}
#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LiveMode {
    On,
    Off,
    Toggle,
}
#[derive(Clone, Copy, Debug)]
pub enum BrightnessChange {
    Set(u8),
    Step(i16),
}
impl BrightnessChange {
    pub fn target(self, reported: Option<u8>) -> CoreResult<u8> {
        match self {
            Self::Set(value) if value <= 100 => Ok(value),
            Self::Step(delta) if delta != 0 && (-100..=100).contains(&delta) => {
                let current = reported.ok_or_else(|| {
                    CoreError::coded(
                        "brightness_unknown",
                        "Current screen brightness is unknown. Choose a fixed brightness first.",
                    )
                    .status(409)
                })?;
                Ok((current as i16 + delta).clamp(0, 100) as u8)
            }
            _ => Err(CoreError::coded("invalid_action", "Invalid brightness action.").status(400)),
        }
    }
}
fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
}
pub fn reported_brightness(info: &Value) -> Option<u8> {
    let data = info.get("data").filter(|d| d.is_object()).unwrap_or(info);
    data.get("brightness")?
        .as_u64()
        .filter(|v| *v <= 100)
        .map(|v| v as u8)
}
impl DockAction {
    pub fn validate(&self) -> CoreResult<()> {
        match self {
            Self::ThemeSelect { theme_id } if !valid_id(theme_id) => {
                Err(CoreError::coded("invalid_action", "Invalid cooler theme ID.").status(400))
            }
            Self::BrightnessSet { value } => BrightnessChange::Set(*value).target(None).map(|_| ()),
            Self::BrightnessStep { delta } if *delta == 0 || !(-100..=100).contains(delta) => {
                Err(CoreError::coded("invalid_action", "Invalid brightness step.").status(400))
            }
            _ => Ok(()),
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{
        api::{Backends, NoSources},
        app::NoEvents,
        device_service::DeviceService,
        fake_media::FakeMedia,
    };
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };
    use std::time::Duration;
    fn app(mode: &str) -> (tempfile::TempDir, Arc<App>) {
        let dir = tempfile::tempdir().unwrap();
        let app = App::with_device(
            dir.path().into(),
            Backends {
                media: Arc::new(FakeMedia::default()),
                sources: Arc::new(NoSources),
            },
            Arc::new(NoEvents),
            "test",
            DeviceService::fake(mode, 0.01, false),
        );
        app.live.set_no_device(true);
        (dir, app)
    }
    #[test]
    fn strict_actions_and_brightness_baselines() {
        for value in [
            json!({"op":"theme_next","extra":1}),
            json!({"op":"brightness_set","value":-1}),
            json!({"op":"brightness_set","value":1.5}),
            json!({"op":"live","mode":"maybe"}),
            json!({"op":"raw","key":3}),
        ] {
            assert!(serde_json::from_value::<DockAction>(value).is_err());
        }
        for value in [
            json!({"op":"theme_select","theme_id":"../x"}),
            json!({"op":"brightness_set","value":101}),
            json!({"op":"brightness_step","delta":0}),
            json!({"op":"brightness_step","delta":101}),
        ] {
            assert!(serde_json::from_value::<DockAction>(value)
                .unwrap()
                .validate()
                .is_err());
        }
        assert!(BrightnessChange::Step(1)
            .target(None)
            .unwrap_err()
            .code_is("brightness_unknown"));
        assert_eq!(BrightnessChange::Step(10).target(Some(95)).unwrap(), 100);
        assert_eq!(BrightnessChange::Step(-10).target(Some(5)).unwrap(), 0);
        assert_eq!(
            reported_brightness(&json!({"cmd":"info","data":{"brightness":73}})),
            Some(73)
        );
        for v in [
            json!({}),
            json!({"brightness":101}),
            json!({"brightness":-1}),
            json!({"brightness":1.5}),
        ] {
            assert_eq!(reported_brightness(&v), None);
        }
        assert_eq!(
            crate::device::brightness_frame(73).unwrap(),
            crate::device::frame::frame(0x03, &[73])
        );
        assert!(crate::device::brightness_frame(101).is_err());
    }
    #[test]
    fn catalog_cycle_and_actions_leave_embedded_untouched() {
        let (_dir, app) = app("winusb");
        for (id, name) in [
            ("themeaaa", "Zulu"),
            ("themebbb", "alpha"),
            ("themeccc", "Alpha"),
        ] {
            app.lib
                .write_original(&json!({"id":id,"name":name}))
                .unwrap();
        }
        let rows = theme_rows(&app).unwrap();
        assert_eq!(
            rows.iter().map(|r| r.0.as_str()).collect::<Vec<_>>(),
            vec!["themebbb", "themeccc", "themeaaa"]
        );
        assert_eq!(
            next_theme(&rows, Some("themeaaa"), true).unwrap(),
            "themebbb"
        );
        assert_eq!(
            next_theme(&rows, Some("themebbb"), false).unwrap(),
            "themeaaa"
        );
        let before = app.settings.load().slot("standby").cloned();
        let state = app
            .dock_action(DockAction::ThemeSelect {
                theme_id: "themeaaa".into(),
            })
            .unwrap();
        assert_eq!(state["currentThemeId"], "themeaaa");
        assert!(!state["liveEnabled"].as_bool().unwrap());
        assert_eq!(app.settings.load().slot("standby").cloned(), before);
        assert_eq!(
            app.dock_action(DockAction::ThemeNext {}).unwrap()["currentThemeId"],
            "themebbb"
        );
        assert_eq!(
            app.dock_action(DockAction::ThemePrevious {}).unwrap()["currentThemeId"],
            "themeaaa"
        );
        assert_eq!(app.settings.load().slot("standby").cloned(), before);
        assert_eq!(
            app.dock_control_state().unwrap().as_object().unwrap().len(),
            10
        );
    }
    #[test]
    fn brightness_reports_acknowledged_request_and_refuses_unknown_step() {
        let (_dir, app) = app("brightness-unknown");
        assert!(app
            .dock_action(DockAction::BrightnessStep { delta: 10 })
            .unwrap_err()
            .code_is("brightness_unknown"));
        assert_eq!(
            app.brightness_state(),
            json!({"reported":null,"requested":null})
        );
        assert_eq!(
            app.dock_action(DockAction::BrightnessSet { value: 95 })
                .unwrap()["brightness"],
            json!({"reported":95,"requested":95})
        );
        assert_eq!(
            app.dock_action(DockAction::BrightnessStep { delta: 10 })
                .unwrap()["brightness"],
            json!({"reported":100,"requested":100})
        );
        assert_eq!(
            app.app_state()["brightness"],
            json!({"reported":100,"requested":100})
        );
    }
    #[test]
    fn unconfirmed_brightness_keeps_request_distinct_from_actual_report() {
        for (mode, reported) in [
            ("brightness-no-readback", Value::Null),
            ("brightness-mismatch", json!(69)),
        ] {
            let (_dir, app) = app(mode);
            *app.last_info.lock_or_recover() = Some(json!({"data":{"brightness":50}}));
            assert!(app
                .dock_action(DockAction::BrightnessSet { value: 70 })
                .unwrap_err()
                .code_is("brightness_unconfirmed"));
            assert_eq!(
                app.brightness_state(),
                json!({"requested":70,"reported":reported})
            );
        }
    }
    #[test]
    fn busy_actions_are_refused_without_queued_replay() {
        let (_dir, app) = app("winusb");
        let release = Arc::new(AtomicBool::new(false));
        let held = release.clone();
        let handle = app.sched.submit(
            TaskSpec::new("test", "held", Resources::one(Res::Device)).visible(),
            move |ctx| {
                while !held.load(Ordering::SeqCst) {
                    ctx.sleep(Duration::from_millis(2))?;
                }
                Ok(Value::Null)
            },
        );
        assert!(app
            .dock_action(DockAction::BrightnessSet { value: 70 })
            .unwrap_err()
            .code_is("busy"));
        assert_eq!(app.brightness_state()["requested"], Value::Null);
        release.store(true, Ordering::SeqCst);
        handle.wait().unwrap();
        assert_eq!(app.brightness_state()["requested"], Value::Null);
    }
}
fn theme_rows(app: &App) -> CoreResult<Vec<(String, String)>> {
    let originals = app.lib.list_originals();
    if originals.len() > 4096 {
        return Err(
            CoreError::coded("catalog_limit", "Too many cooler themes for dock controls.")
                .status(413),
        );
    }
    let mut themes: Vec<_> = originals
        .iter()
        .filter_map(|o| {
            let id = o["id"].as_str()?;
            if !valid_id(id) {
                return None;
            }
            Some((
                id.to_string(),
                o["name"]
                    .as_str()
                    .unwrap_or("Theme")
                    .chars()
                    .filter(|c| !c.is_control())
                    .take(256)
                    .collect::<String>(),
            ))
        })
        .collect();
    themes.sort_by(|a, b| {
        a.1.to_lowercase()
            .cmp(&b.1.to_lowercase())
            .then_with(|| a.0.cmp(&b.0))
    });
    Ok(themes)
}
fn next_theme(
    rows: &[(String, String)],
    current: Option<&str>,
    forward: bool,
) -> CoreResult<String> {
    if rows.is_empty() {
        return Err(CoreError::coded("no_themes", "No cooler themes are available.").status(409));
    }
    let index = current.and_then(|id| rows.iter().position(|row| row.0 == id));
    let next = match (index, forward) {
        (Some(i), true) => (i + 1) % rows.len(),
        (Some(i), false) => (i + rows.len() - 1) % rows.len(),
        (None, true) => 0,
        (None, false) => rows.len() - 1,
    };
    Ok(rows[next].0.clone())
}
impl App {
    pub fn brightness_state(&self) -> Value {
        let reported = self
            .last_info
            .lock_or_recover()
            .as_ref()
            .and_then(reported_brightness);
        json!({"reported":reported,"requested":*self.brightness_requested.lock_or_recover()})
    }
    pub fn dock_control_state(&self) -> CoreResult<Value> {
        let themes = theme_rows(self)?;
        let live = self.live.status();
        Ok(
            json!({"themes":themes.into_iter().map(|(id,name)|json!({"id":id,"name":name})).collect::<Vec<_>>(),"currentThemeId":self.live_theme_oid(),"embeddedThemeId":self.embedded_theme_oid(),"liveEnabled":self.settings.load().live().in_app,"running":live["running"],"connected":live["connected"],"paused":live["paused"],"deviceHold":live["deviceHold"],"systemHold":live["systemHold"],"brightness":self.brightness_state()}),
        )
    }
    pub fn dock_action(&self, action: DockAction) -> CoreResult<Value> {
        action.validate()?;
        let status = self.live.status();
        if status["systemHold"] == true
            || status["previewing"] == true
            || status["calibrating"] == true
            || status["tuning"] == true
        {
            return Err(CoreError::coded(
                "busy",
                "The cooler is in use. Finish the current preview or adjustment first.",
            )
            .status(409));
        }
        let brightness = matches!(
            action,
            DockAction::BrightnessSet { .. } | DockAction::BrightnessStep { .. }
        );
        let resources = if brightness {
            Resources::one(Res::Device)
        } else {
            Resources::none()
        };
        let app = self.arc();
        let spec = TaskSpec::new("dock_action", "Cooler dock action", resources)
            .priority(Priority::User)
            .visible();
        let result = self
            .sched
            .try_submit_idle(spec, move |_| {
                match &action {
                    DockAction::ThemeSelect { theme_id } => {
                        app.theme_use(theme_id)?;
                    }
                    DockAction::ThemeNext {} | DockAction::ThemePrevious {} => {
                        let rows = theme_rows(&app)?;
                        let next = next_theme(
                            &rows,
                            app.live_theme_oid().as_deref(),
                            matches!(action, DockAction::ThemeNext {}),
                        )?;
                        app.theme_use(&next)?;
                    }
                    DockAction::Live { mode } => {
                        let current = app.settings.load().live();
                        let enabled = match mode {
                            LiveMode::On => true,
                            LiveMode::Off => false,
                            LiveMode::Toggle => !current.in_app,
                        };
                        app.settings.update(|s| {
                            s["live"]["inApp"] = json!(enabled);
                        })?;
                        if enabled {
                            app.prepare_live();
                            let cfg = app.live_config();
                            app.live.start_with(cfg.clone());
                            app.live.reload(&cfg);
                        } else if current.in_app || app.live.running() {
                            app.live.stop();
                        }
                        app.settings_changed();
                    }
                    DockAction::BrightnessSet { value } => {
                        app.apply_dock_brightness(BrightnessChange::Set(*value))?;
                    }
                    DockAction::BrightnessStep { delta } => {
                        app.apply_dock_brightness(BrightnessChange::Step(*delta))?;
                    }
                }
                Ok(Value::Null)
            })?
            .wait();
        self.events.app_changed();
        result?;
        self.dock_control_state()
    }
    fn apply_dock_brightness(&self, change: BrightnessChange) -> CoreResult<()> {
        match self.device.brightness_change(change) {
            Ok(result) => {
                let reported = result["reported"].as_u64().filter(|v| *v <= 100);
                let mut last = self.last_info.lock_or_recover();
                let info = last.get_or_insert_with(|| json!({}));
                let data = if info.get("data").is_some_and(Value::is_object) {
                    &mut info["data"]
                } else {
                    info
                };
                data["brightness"] = json!(reported);
                *self.brightness_requested.lock_or_recover() =
                    result["requested"].as_u64().map(|v| v as u8);
                if reported == result["requested"].as_u64() {
                    Ok(())
                } else {
                    Err(CoreError::coded("brightness_unconfirmed","The brightness write was sent, but the screen did not confirm the requested level. Check its reported brightness before trying again.").status(409))
                }
            }
            Err(error) => {
                if let Some(info) = self.last_info.lock_or_recover().as_mut() {
                    let data = if info.get("data").is_some_and(Value::is_object) {
                        &mut info["data"]
                    } else {
                        info
                    };
                    data["brightness"] = Value::Null;
                }
                Err(error)
            }
        }
    }
}
