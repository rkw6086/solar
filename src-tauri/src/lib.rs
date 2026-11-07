mod attention;
pub mod engine;
pub mod model;
mod onboarding;
mod profile_config;
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
mod desktop_onboarding;
#[cfg(any(feature = "desktop", test))]
mod space_score;
#[cfg(feature = "desktop")]
pub use desktop::run;
