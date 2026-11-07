mod attention;
pub mod engine;
#[cfg_attr(not(feature = "desktop"), allow(dead_code))]
mod logging_credentials;
pub mod model;
mod onboarding;
mod profile_config;
#[cfg_attr(not(feature = "desktop"), allow(dead_code))]
mod remote_logging;
#[cfg(any(feature = "desktop", test))]
mod reset;
mod routines;
mod runs;
#[cfg(any(feature = "desktop", test))]
mod storage_root;
pub mod store;

#[cfg(feature = "desktop")]
mod attention_geometry;
#[cfg(any(feature = "desktop", test))]
mod attention_sound;
#[cfg(feature = "desktop")]
mod desktop;
#[cfg(feature = "desktop")]
mod desktop_attention;
#[cfg(feature = "desktop")]
mod desktop_intro;
#[cfg(feature = "desktop")]
mod desktop_logging;
#[cfg(feature = "desktop")]
mod desktop_onboarding;
#[cfg(any(feature = "desktop", test))]
mod sound_effects;
#[cfg(feature = "desktop")]
pub use desktop::run;
