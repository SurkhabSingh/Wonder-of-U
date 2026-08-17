mod asset;
mod envelope;
mod queue;
mod dictionary;
mod control;
mod alass;
mod ffmpeg;
mod model;
mod runtime;
mod transfer;
mod ytdlp;

pub(crate) use asset::AssetKind;
pub(crate) use control::{
    cancel_whisper_model_download_inner, toggle_whisper_model_download_pause_inner,
};
/// The queue is the only way in now. Each asset module contributes a *plan*; nothing outside
/// this module can start a download directly, which is what keeps "one at a time" true.
pub(crate) use queue::{enqueue_download, DownloadQueue, QueuedDownload};
