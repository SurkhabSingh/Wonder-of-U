mod dictionary;
mod ffmpeg;
mod path_probe;
mod updates;
mod whisper;
mod ytdlp;

pub(crate) use dictionary::{
    detect_local_dictionary, find_managed_dictionary_root, managed_dictionary_install_directory,
};
pub(crate) use ffmpeg::{
    collect_managed_ffmpeg_candidates, detect_local_ffmpeg, managed_ffmpeg_install_directory,
    verify_ffmpeg_binary,
};
pub(crate) use updates::{
    check_dictionary_update_inner, check_whisper_model_update_inner,
    check_whisper_runtime_update_inner, check_ytdlp_update_inner,
};
pub(crate) use ytdlp::{
    detect_local_ytdlp, managed_ytdlp_install_directory, verify_ytdlp_binary,
};
pub(crate) use whisper::{
    all_managed_model_paths, app_managed_runtime_directory, collect_managed_whisper_cli_candidates,
    refresh_whisper_detection_state, whisper_detection_inputs_changed,
};
