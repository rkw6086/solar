use crate::{engine::Engine, model::Settings};

pub(crate) const STEPS: [&str; 8] = [
    "birth",
    "welcome",
    "team",
    "desktop",
    "connection",
    "orbit",
    "workspace",
    "ready",
];

impl Engine {
    pub fn save_onboarding_progress(&self, step: &str) -> Result<Settings, String> {
        self.ensure_running()?;
        if !STEPS.contains(&step) {
            return Err("Unknown welcome step".into());
        }
        let _write = self
            .settings_write
            .lock()
            .map_err(|_| "Settings are unavailable")?;
        let mut settings = self.settings()?;
        settings.onboarding_step = step.into();
        self.store.put("settings", "app", &settings)?;
        self.changed();
        Ok(settings)
    }

    pub fn complete_onboarding(&self) -> Result<Settings, String> {
        self.ensure_running()?;
        let _write = self
            .settings_write
            .lock()
            .map_err(|_| "Settings are unavailable")?;
        let mut settings = self.settings()?;
        settings.onboarding_complete = true;
        settings.onboarding_step = "ready".into();
        self.store.put("settings", "app", &settings)?;
        self.changed();
        Ok(settings)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_run_is_resumable_and_completion_survives_restart() {
        let temp = tempfile::tempdir().unwrap();
        let engine = Engine::open(temp.path().into()).unwrap();
        assert!(!engine.settings().unwrap().onboarding_complete);
        engine.save_onboarding_progress("connection").unwrap();
        assert_eq!(
            Engine::open(temp.path().into())
                .unwrap()
                .settings()
                .unwrap()
                .onboarding_step,
            "connection"
        );
        engine.complete_onboarding().unwrap();
        let reopened = Engine::open(temp.path().into()).unwrap();
        assert!(reopened.settings().unwrap().onboarding_complete);
        assert!(reopened.bootstrap().unwrap().orbits.is_empty());
        assert!(reopened.bootstrap().unwrap().conversations.is_empty());
        assert!(reopened.bootstrap().unwrap().providers.is_empty());
    }

    #[test]
    fn bad_steps_and_stale_settings_cannot_rewind_progress() {
        let temp = tempfile::tempdir().unwrap();
        let engine = Engine::open(temp.path().into()).unwrap();
        let stale = engine.settings().unwrap();
        assert!(engine.save_onboarding_progress("invented").is_err());
        engine.complete_onboarding().unwrap();
        let saved = engine.save_settings(stale).unwrap();
        assert!(saved.onboarding_complete);
        assert_eq!(saved.onboarding_step, "ready");
        engine.save_onboarding_progress("birth").unwrap();
        assert!(
            engine.settings().unwrap().onboarding_complete,
            "Replaying the introduction should not reset setup"
        );
    }

    #[test]
    fn old_settings_receive_accessible_intro_defaults() {
        let settings: Settings = serde_json::from_str(r#"{"reducedMotion":true}"#).unwrap();
        assert!(settings.reduced_motion);
        assert_eq!(settings.onboarding_step, "birth");
        assert!(!settings.onboarding_complete);
        assert!(settings.sound_effects);
        assert_eq!(settings.sound_effects_volume, 0.25);
    }

    #[test]
    fn existing_audio_preferences_apply_to_sound_effects() {
        let settings: Settings =
            serde_json::from_str(r#"{"onboardingMusic":false,"onboardingVolume":0.09}"#).unwrap();
        assert!(!settings.sound_effects);
        assert_eq!(settings.sound_effects_volume, 0.09);
        let saved = serde_json::to_value(settings).unwrap();
        assert_eq!(saved["soundEffects"], false);
        assert!(saved.get("onboardingMusic").is_none());
    }
}
