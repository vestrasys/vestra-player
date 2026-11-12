//! The playback side: the backend thread and everything it owns (libmpv,
//! sources and their leases, processing, subtitles, producers, offline jobs,
//! render targets and frame pacing).

pub(crate) mod audio_hold;
pub(crate) mod backend;
pub(crate) mod cover_listing;
pub(crate) mod effects;
pub(crate) mod gpu_warm;
pub(crate) mod media_copy;
pub(crate) mod media_jobs;
pub(crate) mod media_join;
pub(crate) mod media_sources;
pub(crate) mod media_tracks;
pub(crate) mod media_worker;
pub(crate) mod native_subtitle;
pub(crate) mod offline_audio;
pub(crate) mod offline_descriptor;
pub(crate) mod plugin_files;
pub(crate) mod processing;
pub(crate) mod processing_live;
pub mod processing_operations;
#[cfg(feature = "producer-audio")]
pub(crate) mod producer;
#[cfg(feature = "producer-audio")]
pub(crate) mod producer_audio;
#[cfg(feature = "producer-audio")]
pub mod producer_binary;
#[cfg(feature = "producer-audio")]
pub(crate) mod producer_controls;
#[cfg(feature = "producer-audio")]
pub mod producer_input;
#[cfg(feature = "producer-audio")]
pub(crate) mod producer_input_delivery;
pub(crate) mod session;
pub(crate) mod source_backend;
pub(crate) mod source_geometry;
pub mod source_lease;
pub(crate) mod source_resolvers;
pub(crate) mod subtitle_backend;
pub mod subtitle_format;
pub mod subtitle_state;
pub mod subtitle_style;
pub mod subtitle_upload;
// Windows: libmpv reads leased local files through stream_cb (Unix: fd://).
pub(crate) mod frame_pacing;
#[cfg(any(windows, test))]
pub(crate) mod lease_stream;
pub(crate) mod motion;
pub(crate) mod render_denylist;
pub(crate) mod render_mode;
pub(crate) mod render_state;
pub(crate) mod render_target;
pub(crate) mod renderer;
pub(crate) mod upscaling;
pub(crate) mod video_click;
pub(crate) mod video_render;
pub(crate) mod video_shaders;
pub(crate) mod video_view;
