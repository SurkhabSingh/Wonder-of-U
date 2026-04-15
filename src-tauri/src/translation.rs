use serde::Deserialize;
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};

const RUNTIME_PROBE_SCRIPT: &str = r#"
import importlib.util
import json
import sys

modules = ["ctranslate2", "transformers", "sentencepiece", "google.protobuf"]
missing = []
for name in modules:
    try:
        if importlib.util.find_spec(name) is None:
            missing.append(name)
    except (ImportError, ModuleNotFoundError, ValueError):
        missing.append(name)
print(json.dumps({"python": sys.executable, "missing": missing}))
"#;

#[derive(Debug, Clone, Copy)]
pub struct TranslationLanguageSpec {
    pub id: &'static str,
    pub label: &'static str,
    pub whisper_code: &'static str,
    pub nllb_code: &'static str,
}

#[derive(Debug, Clone, Copy)]
pub struct ManagedTranslationFileSpec {
    pub relative_path: &'static str,
    pub download_url: &'static str,
}

#[derive(Debug, Clone, Copy)]
pub struct TranslationModelSpec {
    pub id: &'static str,
    pub label: &'static str,
    pub files: &'static [ManagedTranslationFileSpec],
}

pub const TRANSLATION_LANGUAGE_SPECS: [TranslationLanguageSpec; 10] = [
    TranslationLanguageSpec {
        id: "en",
        label: "English",
        whisper_code: "en",
        nllb_code: "eng_Latn",
    },
    TranslationLanguageSpec {
        id: "ja",
        label: "Japanese",
        whisper_code: "ja",
        nllb_code: "jpn_Jpan",
    },
    TranslationLanguageSpec {
        id: "hi",
        label: "Hindi",
        whisper_code: "hi",
        nllb_code: "hin_Deva",
    },
    TranslationLanguageSpec {
        id: "es",
        label: "Spanish",
        whisper_code: "es",
        nllb_code: "spa_Latn",
    },
    TranslationLanguageSpec {
        id: "fr",
        label: "French",
        whisper_code: "fr",
        nllb_code: "fra_Latn",
    },
    TranslationLanguageSpec {
        id: "de",
        label: "German",
        whisper_code: "de",
        nllb_code: "deu_Latn",
    },
    TranslationLanguageSpec {
        id: "ko",
        label: "Korean",
        whisper_code: "ko",
        nllb_code: "kor_Hang",
    },
    TranslationLanguageSpec {
        id: "zh",
        label: "Chinese (Simplified)",
        whisper_code: "zh",
        nllb_code: "zho_Hans",
    },
    TranslationLanguageSpec {
        id: "pt",
        label: "Portuguese",
        whisper_code: "pt",
        nllb_code: "por_Latn",
    },
    TranslationLanguageSpec {
        id: "ru",
        label: "Russian",
        whisper_code: "ru",
        nllb_code: "rus_Cyrl",
    },
];

const DISTILLED_600M_INT8_FILES: [ManagedTranslationFileSpec; 7] = [
    ManagedTranslationFileSpec {
        relative_path: "config.json",
        download_url:
            "https://huggingface.co/skywood/nllb-200-distilled-600M-ct2-int8/resolve/main/config.json",
    },
    ManagedTranslationFileSpec {
        relative_path: "model.bin",
        download_url:
            "https://huggingface.co/skywood/nllb-200-distilled-600M-ct2-int8/resolve/main/model.bin",
    },
    ManagedTranslationFileSpec {
        relative_path: "sentencepiece.bpe.model",
        download_url:
            "https://huggingface.co/skywood/nllb-200-distilled-600M-ct2-int8/resolve/main/sentencepiece.bpe.model",
    },
    ManagedTranslationFileSpec {
        relative_path: "shared_vocabulary.json",
        download_url:
            "https://huggingface.co/skywood/nllb-200-distilled-600M-ct2-int8/resolve/main/shared_vocabulary.json",
    },
    ManagedTranslationFileSpec {
        relative_path: "special_tokens_map.json",
        download_url:
            "https://huggingface.co/skywood/nllb-200-distilled-600M-ct2-int8/resolve/main/special_tokens_map.json",
    },
    ManagedTranslationFileSpec {
        relative_path: "tokenizer.json",
        download_url:
            "https://huggingface.co/skywood/nllb-200-distilled-600M-ct2-int8/resolve/main/tokenizer.json",
    },
    ManagedTranslationFileSpec {
        relative_path: "tokenizer_config.json",
        download_url:
            "https://huggingface.co/skywood/nllb-200-distilled-600M-ct2-int8/resolve/main/tokenizer_config.json",
    },
];

const DISTILLED_1_3B_INT8_FILES: [ManagedTranslationFileSpec; 7] = [
    ManagedTranslationFileSpec {
        relative_path: "config.json",
        download_url:
            "https://huggingface.co/OpenNMT/nllb-200-distilled-1.3B-ct2-int8/resolve/main/config.json",
    },
    ManagedTranslationFileSpec {
        relative_path: "generation_config.json",
        download_url:
            "https://huggingface.co/OpenNMT/nllb-200-distilled-1.3B-ct2-int8/resolve/main/generation_config.json",
    },
    ManagedTranslationFileSpec {
        relative_path: "model.bin",
        download_url:
            "https://huggingface.co/OpenNMT/nllb-200-distilled-1.3B-ct2-int8/resolve/main/model.bin",
    },
    ManagedTranslationFileSpec {
        relative_path: "shared_vocabulary.json",
        download_url:
            "https://huggingface.co/OpenNMT/nllb-200-distilled-1.3B-ct2-int8/resolve/main/shared_vocabulary.json",
    },
    ManagedTranslationFileSpec {
        relative_path: "special_tokens_map.json",
        download_url:
            "https://huggingface.co/OpenNMT/nllb-200-distilled-1.3B-ct2-int8/resolve/main/special_tokens_map.json",
    },
    ManagedTranslationFileSpec {
        relative_path: "tokenizer.json",
        download_url:
            "https://huggingface.co/OpenNMT/nllb-200-distilled-1.3B-ct2-int8/resolve/main/tokenizer.json",
    },
    ManagedTranslationFileSpec {
        relative_path: "tokenizer_config.json",
        download_url:
            "https://huggingface.co/OpenNMT/nllb-200-distilled-1.3B-ct2-int8/resolve/main/tokenizer_config.json",
    },
];

const NLLB_3_3B_INT8_FILES: [ManagedTranslationFileSpec; 7] = [
    ManagedTranslationFileSpec {
        relative_path: "config.json",
        download_url:
            "https://huggingface.co/OpenNMT/nllb-200-3.3B-ct2-int8/resolve/main/config.json",
    },
    ManagedTranslationFileSpec {
        relative_path: "generation_config.json",
        download_url:
            "https://huggingface.co/OpenNMT/nllb-200-3.3B-ct2-int8/resolve/main/generation_config.json",
    },
    ManagedTranslationFileSpec {
        relative_path: "model.bin",
        download_url:
            "https://huggingface.co/OpenNMT/nllb-200-3.3B-ct2-int8/resolve/main/model.bin",
    },
    ManagedTranslationFileSpec {
        relative_path: "shared_vocabulary.json",
        download_url:
            "https://huggingface.co/OpenNMT/nllb-200-3.3B-ct2-int8/resolve/main/shared_vocabulary.json",
    },
    ManagedTranslationFileSpec {
        relative_path: "special_tokens_map.json",
        download_url:
            "https://huggingface.co/OpenNMT/nllb-200-3.3B-ct2-int8/resolve/main/special_tokens_map.json",
    },
    ManagedTranslationFileSpec {
        relative_path: "tokenizer.json",
        download_url:
            "https://huggingface.co/OpenNMT/nllb-200-3.3B-ct2-int8/resolve/main/tokenizer.json",
    },
    ManagedTranslationFileSpec {
        relative_path: "tokenizer_config.json",
        download_url:
            "https://huggingface.co/OpenNMT/nllb-200-3.3B-ct2-int8/resolve/main/tokenizer_config.json",
    },
];

pub const TRANSLATION_MODEL_SPECS: [TranslationModelSpec; 3] = [
    TranslationModelSpec {
        id: "distilled-600m-int8",
        label: "Distilled 600M",
        files: &DISTILLED_600M_INT8_FILES,
    },
    TranslationModelSpec {
        id: "distilled-1.3b-int8",
        label: "Distilled 1.3B",
        files: &DISTILLED_1_3B_INT8_FILES,
    },
    TranslationModelSpec {
        id: "3.3b-int8",
        label: "3.3B",
        files: &NLLB_3_3B_INT8_FILES,
    },
];

#[derive(Debug, Clone)]
pub struct PythonRuntimeProbe {
    pub executable_path: PathBuf,
    pub missing_modules: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct TranslationRequest {
    pub python_path: PathBuf,
    pub bridge_script_path: PathBuf,
    pub model_dir: PathBuf,
    pub transcript_path: PathBuf,
    pub output_path: PathBuf,
    pub source_language: String,
    pub target_language: String,
}

#[derive(Debug, Clone)]
pub struct TranslationResult {
    pub translation_path: PathBuf,
}

#[derive(Debug, Deserialize)]
struct RuntimeProbeResponse {
    python: String,
    missing: Vec<String>,
}

fn parse_runtime_probe_output(output: Output, command_label: &str) -> Result<PythonRuntimeProbe, String> {
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let details = if !stderr.is_empty() { stderr } else { stdout };
        return Err(if details.is_empty() {
            format!("{command_label} could not report the local Python runtime.")
        } else {
            details
        });
    }

    let response: RuntimeProbeResponse = serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("Could not parse the local Python runtime probe: {error}"))?;

    Ok(PythonRuntimeProbe {
        executable_path: PathBuf::from(response.python),
        missing_modules: response.missing,
    })
}

pub fn default_translation_target_id() -> &'static str {
    "en"
}

pub fn default_translation_model_id() -> &'static str {
    "distilled-600m-int8"
}

pub fn default_translation_model_choice() -> String {
    default_translation_model_id().to_string()
}

pub fn default_translation_target_choice() -> String {
    default_translation_target_id().to_string()
}

pub fn translation_model_spec(model_id: &str) -> &'static TranslationModelSpec {
    TRANSLATION_MODEL_SPECS
        .iter()
        .find(|spec| spec.id.eq_ignore_ascii_case(model_id.trim()))
        .unwrap_or(&TRANSLATION_MODEL_SPECS[0])
}

pub fn translation_language_spec(language_id: &str) -> Option<&'static TranslationLanguageSpec> {
    TRANSLATION_LANGUAGE_SPECS
        .iter()
        .find(|spec| spec.id.eq_ignore_ascii_case(language_id.trim()))
}

pub fn translation_language_from_whisper_code(
    whisper_code: &str,
) -> Option<&'static TranslationLanguageSpec> {
    TRANSLATION_LANGUAGE_SPECS
        .iter()
        .find(|spec| spec.whisper_code.eq_ignore_ascii_case(whisper_code.trim()))
}

pub fn probe_python_runtime(
    command: &str,
    extra_args: &[&str],
) -> Result<PythonRuntimeProbe, String> {
    let output = Command::new(command)
        .args(extra_args)
        .arg("-c")
        .arg(RUNTIME_PROBE_SCRIPT)
        .output()
        .map_err(|error| format!("{}: {}", command, error))?;

    parse_runtime_probe_output(output, command)
}

pub fn probe_python_executable(executable_path: &Path) -> Result<PythonRuntimeProbe, String> {
    let output = Command::new(executable_path)
        .arg("-c")
        .arg(RUNTIME_PROBE_SCRIPT)
        .output()
        .map_err(|error| format!("{}: {}", executable_path.display(), error))?;

    parse_runtime_probe_output(output, &executable_path.display().to_string())
}

pub fn verify_translation_model_dir(model_dir: &Path) -> Result<(), String> {
    let metadata = fs::metadata(model_dir).map_err(|error| error.to_string())?;
    if !metadata.is_dir() {
        return Err("The selected translation model path is not a directory.".into());
    }

    if !model_dir.join("model.bin").is_file() {
        return Err("The translation model directory is missing model.bin.".into());
    }

    let tokenizer_markers = [
        "tokenizer.json",
        "sentencepiece.bpe.model",
        "source.spm",
        "shared_vocabulary.json",
    ];
    if !tokenizer_markers
        .iter()
        .any(|marker| model_dir.join(marker).is_file())
    {
        return Err(
            "The translation model directory is missing tokenizer files such as tokenizer.json, sentencepiece.bpe.model, or shared_vocabulary.json."
                .into(),
        );
    }

    Ok(())
}

pub fn translation_output_path(transcript_path: &Path, target_language: &str) -> PathBuf {
    let parent = transcript_path.parent().unwrap_or_else(|| Path::new("."));
    let file_name = transcript_path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("recording.transcript.txt");
    let base_name = file_name
        .strip_suffix(".transcript.txt")
        .unwrap_or(file_name);
    parent.join(format!("{base_name}.translation.{target_language}.txt"))
}

pub fn run_translation(request: &TranslationRequest) -> Result<TranslationResult, String> {
    verify_translation_model_dir(&request.model_dir)?;

    let transcript_metadata =
        fs::metadata(&request.transcript_path).map_err(|error| error.to_string())?;
    if !transcript_metadata.is_file() {
        return Err("The transcript input path is not a file.".into());
    }

    if let Some(parent) = request.output_path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }

    let output = Command::new(&request.python_path)
        .arg(&request.bridge_script_path)
        .arg("--model-dir")
        .arg(&request.model_dir)
        .arg("--transcript-path")
        .arg(&request.transcript_path)
        .arg("--output-path")
        .arg(&request.output_path)
        .arg("--source-language")
        .arg(&request.source_language)
        .arg("--target-language")
        .arg(&request.target_language)
        .output()
        .map_err(|error| error.to_string())?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let details = if !stderr.is_empty() { stderr } else { stdout };
        return Err(if details.is_empty() {
            "The local CTranslate2 translation failed.".into()
        } else {
            details
        });
    }

    if !request.output_path.exists() {
        return Err("The translation finished without writing the output file.".into());
    }

    Ok(TranslationResult {
        translation_path: request.output_path.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::{
        translation_language_from_whisper_code, translation_output_path,
        verify_translation_model_dir,
    };

    #[test]
    fn translation_output_path_reuses_transcript_stem() {
        let transcript_path = std::path::Path::new("C:\\Temp\\lesson.transcript.txt");
        let translation_path = translation_output_path(transcript_path, "en");
        assert_eq!(
            translation_path.to_string_lossy(),
            "C:\\Temp\\lesson.translation.en.txt"
        );
    }

    #[test]
    fn translation_model_validation_requires_model_bin() {
        let temp_dir = tempfile::tempdir().unwrap();
        std::fs::write(
            temp_dir.path().join("sentencepiece.bpe.model"),
            b"tokenizer",
        )
        .unwrap();

        let error = verify_translation_model_dir(temp_dir.path()).unwrap_err();
        assert!(error.contains("model.bin"));
    }

    #[test]
    fn whisper_language_lookup_maps_supported_codes() {
        let language = translation_language_from_whisper_code("ja").unwrap();
        assert_eq!(language.id, "ja");
        assert_eq!(language.nllb_code, "jpn_Jpan");
    }
}
