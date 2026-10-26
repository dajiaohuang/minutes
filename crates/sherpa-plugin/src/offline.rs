//! Offline transducer recognizer over the sherpa-onnx C API.
//!
//! macOS builds link sherpa-onnx 1.13.x directly, and sherpa-rs only ships
//! bindings for 1.12.x. The two layouts differ (the model config structs grew,
//! and they are nested by value), so this module covers the one offline call
//! path the plugin uses against bindings generated from the 1.13.x headers.

use crate::sys;
use std::ffi::{CStr, CString};

pub struct TransducerConfig {
    pub encoder: String,
    pub decoder: String,
    pub joiner: String,
    pub tokens: String,
    pub num_threads: i32,
    pub decoding_method: String,
    pub model_type: String,
    pub debug: bool,
}

impl Default for TransducerConfig {
    fn default() -> Self {
        Self {
            encoder: String::new(),
            decoder: String::new(),
            joiner: String::new(),
            tokens: String::new(),
            num_threads: 1,
            decoding_method: "greedy_search".into(),
            model_type: String::new(),
            debug: false,
        }
    }
}

pub struct TransducerRecognizer {
    recognizer: *const sys::SherpaOnnxOfflineRecognizer,
}

impl TransducerRecognizer {
    pub fn new(cfg: TransducerConfig) -> Result<Self, String> {
        let c = |value: &str, what: &str| {
            CString::new(value).map_err(|_| format!("{what} contained an interior null"))
        };
        let encoder = c(&cfg.encoder, "encoder path")?;
        let decoder = c(&cfg.decoder, "decoder path")?;
        let joiner = c(&cfg.joiner, "joiner path")?;
        let tokens = c(&cfg.tokens, "tokens path")?;
        let provider = c("cpu", "provider")?;
        let model_type = c(&cfg.model_type, "model type")?;
        let decoding_method = c(&cfg.decoding_method, "decoding method")?;

        // The C API treats a zeroed config as its baseline, so only the fields
        // this model family needs are set; every other model family stays null.
        let mut config: sys::SherpaOnnxOfflineRecognizerConfig = unsafe { std::mem::zeroed() };
        config.feat_config.sample_rate = 16_000;
        config.feat_config.feature_dim = 80;
        config.model_config.transducer.encoder = encoder.as_ptr();
        config.model_config.transducer.decoder = decoder.as_ptr();
        config.model_config.transducer.joiner = joiner.as_ptr();
        config.model_config.tokens = tokens.as_ptr();
        config.model_config.num_threads = cfg.num_threads;
        config.model_config.provider = provider.as_ptr();
        config.model_config.debug = i32::from(cfg.debug);
        config.model_config.model_type = model_type.as_ptr();
        config.decoding_method = decoding_method.as_ptr();
        config.max_active_paths = 4;

        let recognizer = unsafe { sys::SherpaOnnxCreateOfflineRecognizer(&config) };
        if recognizer.is_null() {
            return Err("sherpa-onnx rejected the transducer model".to_string());
        }
        Ok(Self { recognizer })
    }

    /// Decode one window with its own stream, so windows never share state.
    pub fn transcribe(&mut self, sample_rate: u32, samples: &[f32]) -> String {
        unsafe {
            let stream = sys::SherpaOnnxCreateOfflineStream(self.recognizer);
            if stream.is_null() {
                return String::new();
            }
            sys::SherpaOnnxAcceptWaveformOffline(
                stream,
                sample_rate as i32,
                samples.as_ptr(),
                samples.len() as i32,
            );
            sys::SherpaOnnxDecodeOfflineStream(self.recognizer, stream);
            let result = sys::SherpaOnnxGetOfflineStreamResult(stream);
            let text = if result.is_null() || (*result).text.is_null() {
                String::new()
            } else {
                CStr::from_ptr((*result).text)
                    .to_string_lossy()
                    .into_owned()
            };
            if !result.is_null() {
                sys::SherpaOnnxDestroyOfflineRecognizerResult(result);
            }
            sys::SherpaOnnxDestroyOfflineStream(stream);
            text
        }
    }
}

impl Drop for TransducerRecognizer {
    fn drop(&mut self) {
        unsafe { sys::SherpaOnnxDestroyOfflineRecognizer(self.recognizer) };
    }
}
