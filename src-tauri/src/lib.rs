mod attention;
pub mod engine;
pub mod model;
mod routines;
mod runs;
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
pub use desktop::run;
