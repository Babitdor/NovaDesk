//! The model catalogue.
//!
//! One entry per model the Voice page can show. Three rules:
//!
//! - Copy, grades, speeds and licences are the published ones; the README names
//!   where they come from.
//! - "bytes" is what the download actually fetches, so the card, the progress bar
//!   and the post-download size check all read the same number. Models that
//!   cannot be downloaded yet carry the vendor's published size instead, because
//!   the number is only ever displayed.
//! - A model with no loader here is listed, not hidden: supported: false plus the
//!   reason the card shows. Only models whose weights and engine both exist are
//!   installable.

use serde::Serialize;

/// Every language any catalogued model names, with its display name. The list is
/// Whisper's own language table, which already covers the 25 European languages
/// Parakeet and Canary v2 use and the Chinese, Japanese, Korean and Cantonese
/// SenseVoice adds among them.
pub const LANGUAGES: &[(&str, &str)] = &[
    ("en", "English"),
    ("zh", "Chinese"),
    ("de", "German"),
    ("es", "Spanish"),
    ("ru", "Russian"),
    ("ko", "Korean"),
    ("fr", "French"),
    ("ja", "Japanese"),
    ("pt", "Portuguese"),
    ("tr", "Turkish"),
    ("pl", "Polish"),
    ("ca", "Catalan"),
    ("nl", "Dutch"),
    ("ar", "Arabic"),
    ("sv", "Swedish"),
    ("it", "Italian"),
    ("id", "Indonesian"),
    ("hi", "Hindi"),
    ("fi", "Finnish"),
    ("vi", "Vietnamese"),
    ("he", "Hebrew"),
    ("uk", "Ukrainian"),
    ("el", "Greek"),
    ("ms", "Malay"),
    ("cs", "Czech"),
    ("ro", "Romanian"),
    ("da", "Danish"),
    ("hu", "Hungarian"),
    ("ta", "Tamil"),
    ("no", "Norwegian"),
    ("th", "Thai"),
    ("ur", "Urdu"),
    ("hr", "Croatian"),
    ("bg", "Bulgarian"),
    ("lt", "Lithuanian"),
    ("la", "Latin"),
    ("mi", "Maori"),
    ("ml", "Malayalam"),
    ("cy", "Welsh"),
    ("sk", "Slovak"),
    ("te", "Telugu"),
    ("fa", "Persian"),
    ("lv", "Latvian"),
    ("bn", "Bengali"),
    ("sr", "Serbian"),
    ("az", "Azerbaijani"),
    ("sl", "Slovenian"),
    ("kn", "Kannada"),
    ("et", "Estonian"),
    ("mk", "Macedonian"),
    ("br", "Breton"),
    ("eu", "Basque"),
    ("is", "Icelandic"),
    ("hy", "Armenian"),
    ("ne", "Nepali"),
    ("mn", "Mongolian"),
    ("bs", "Bosnian"),
    ("kk", "Kazakh"),
    ("sq", "Albanian"),
    ("sw", "Swahili"),
    ("gl", "Galician"),
    ("mr", "Marathi"),
    ("pa", "Punjabi"),
    ("si", "Sinhala"),
    ("km", "Khmer"),
    ("sn", "Shona"),
    ("yo", "Yoruba"),
    ("so", "Somali"),
    ("af", "Afrikaans"),
    ("oc", "Occitan"),
    ("ka", "Georgian"),
    ("be", "Belarusian"),
    ("tg", "Tajik"),
    ("sd", "Sindhi"),
    ("gu", "Gujarati"),
    ("am", "Amharic"),
    ("yi", "Yiddish"),
    ("lo", "Lao"),
    ("uz", "Uzbek"),
    ("fo", "Faroese"),
    ("ht", "Haitian Creole"),
    ("ps", "Pashto"),
    ("tk", "Turkmen"),
    ("nn", "Nynorsk"),
    ("mt", "Maltese"),
    ("sa", "Sanskrit"),
    ("lb", "Luxembourgish"),
    ("my", "Myanmar"),
    ("bo", "Tibetan"),
    ("tl", "Tagalog"),
    ("mg", "Malagasy"),
    ("as", "Assamese"),
    ("tt", "Tatar"),
    ("haw", "Hawaiian"),
    ("ln", "Lingala"),
    ("ha", "Hausa"),
    ("ba", "Bashkir"),
    ("jw", "Javanese"),
    ("su", "Sundanese"),
    ("yue", "Cantonese"),
];

/// Which inference engine loads the weights. None means no engine exists yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Engine {
    Parakeet,
    Canary,
    Cohere,
    MoonshineBase,
    MoonshineTiny,
    SenseVoice,
    Whisper,
    None,
}

/// A card badge. Kept as a fixed vocabulary so the UI never renders a stray
/// string and the stylesheet can target each one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Badge {
    Recommended,
    Translate,
    LivePreview,
    Streaming,
}

/// Weight precision the download carries, which decides the file names the
/// engine looks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Quant {
    Fp32,
    Int8,
    Int4,
}

/// Where the weights come from.
///
/// Files are fetched one by one straight into the model directory; an archive is
/// streamed to a single .part file and unpacked afterwards. Both carry the exact
/// byte count so a truncated transfer is caught instead of installed.
#[derive(Debug, Clone, Copy)]
pub enum Source {
    /// (file name on disk, absolute URL, bytes)
    Files(&'static [(&'static str, &'static str, u64)]),
    /// (absolute URL, bytes)
    Archive(&'static str, u64),
}

#[derive(Debug, Clone, Copy)]
pub struct Model {
    pub id: &'static str,
    pub name: &'static str,
    pub engine: Engine,
    pub quant: Quant,
    /// Codes this catalogue can name for the model, used by the language filter.
    pub languages: &'static [&'static str],
    /// Languages the vendor publishes, which can be larger than the named ones.
    pub language_count: u16,
    pub bytes: u64,
    pub licence: &'static str,
    pub accuracy: Option<&'static str>,
    pub speed: Option<&'static str>,
    pub description: &'static str,
    pub badges: &'static [Badge],
    pub supported: bool,
    pub unsupported_reason: Option<&'static str>,
    pub source: Option<Source>,
    /// Files the engine needs on disk. Doubles as the integrity check after a
    /// download and the "is this model really installed" test before inference.
    pub required: &'static [&'static str],
}

const PARAKEET_V3_FILES: &[(&str, &str, u64)] = &[
    (
        "encoder-model.int8.onnx",
        "https://huggingface.co/istupakov/parakeet-tdt-0.6b-v3-onnx/resolve/main/encoder-model.int8.onnx",
        652183999,
    ),
    (
        "decoder_joint-model.int8.onnx",
        "https://huggingface.co/istupakov/parakeet-tdt-0.6b-v3-onnx/resolve/main/decoder_joint-model.int8.onnx",
        18202004,
    ),
    (
        "nemo128.onnx",
        "https://huggingface.co/istupakov/parakeet-tdt-0.6b-v3-onnx/resolve/main/nemo128.onnx",
        139764,
    ),
    (
        "vocab.txt",
        "https://huggingface.co/istupakov/parakeet-tdt-0.6b-v3-onnx/resolve/main/vocab.txt",
        93939,
    ),
];

const PARAKEET_V2_FILES: &[(&str, &str, u64)] = &[
    (
        "encoder-model.int8.onnx",
        "https://huggingface.co/istupakov/parakeet-tdt-0.6b-v2-onnx/resolve/main/encoder-model.int8.onnx",
        652184014,
    ),
    (
        "decoder_joint-model.int8.onnx",
        "https://huggingface.co/istupakov/parakeet-tdt-0.6b-v2-onnx/resolve/main/decoder_joint-model.int8.onnx",
        8998286,
    ),
    (
        "nemo128.onnx",
        "https://huggingface.co/istupakov/parakeet-tdt-0.6b-v3-onnx/resolve/main/nemo128.onnx",
        139764,
    ),
    (
        "vocab.txt",
        "https://huggingface.co/istupakov/parakeet-tdt-0.6b-v2-onnx/resolve/main/vocab.txt",
        9384,
    ),
];

const CANARY180_FILES: &[(&str, &str, u64)] = &[
    (
        "encoder-model.int8.onnx",
        "https://huggingface.co/istupakov/canary-180m-flash-onnx/resolve/main/encoder-model.int8.onnx",
        133710896,
    ),
    (
        "decoder-model.int8.onnx",
        "https://huggingface.co/istupakov/canary-180m-flash-onnx/resolve/main/decoder-model.int8.onnx",
        79520211,
    ),
    (
        "vocab.txt",
        "https://huggingface.co/istupakov/canary-180m-flash-onnx/resolve/main/vocab.txt",
        53555,
    ),
    (
        "nemo128.onnx",
        "https://huggingface.co/istupakov/parakeet-tdt-0.6b-v3-onnx/resolve/main/nemo128.onnx",
        139764,
    ),
];

const CANARY1B_FLASH_FILES: &[(&str, &str, u64)] = &[
    (
        "encoder-model.int8.onnx",
        "https://huggingface.co/istupakov/canary-1b-flash-onnx/resolve/main/encoder-model.int8.onnx",
        859379461,
    ),
    (
        "decoder-model.int8.onnx",
        "https://huggingface.co/istupakov/canary-1b-flash-onnx/resolve/main/decoder-model.int8.onnx",
        79520498,
    ),
    (
        "vocab.txt",
        "https://huggingface.co/istupakov/canary-1b-flash-onnx/resolve/main/vocab.txt",
        53566,
    ),
    (
        "nemo128.onnx",
        "https://huggingface.co/istupakov/parakeet-tdt-0.6b-v3-onnx/resolve/main/nemo128.onnx",
        139764,
    ),
];

const CANARY1B_V2_FILES: &[(&str, &str, u64)] = &[
    (
        "encoder-model.int8.onnx",
        "https://huggingface.co/istupakov/canary-1b-v2-onnx/resolve/main/encoder-model.int8.onnx",
        859078138,
    ),
    (
        "decoder-model.int8.onnx",
        "https://huggingface.co/istupakov/canary-1b-v2-onnx/resolve/main/decoder-model.int8.onnx",
        170040374,
    ),
    (
        "vocab.txt",
        "https://huggingface.co/istupakov/canary-1b-v2-onnx/resolve/main/vocab.txt",
        208022,
    ),
    (
        "nemo128.onnx",
        "https://huggingface.co/istupakov/parakeet-tdt-0.6b-v3-onnx/resolve/main/nemo128.onnx",
        139764,
    ),
];

const MOONSHINE_TINY_FILES: &[(&str, &str, u64)] = &[
    (
        "encoder_model.int8.onnx",
        "https://huggingface.co/onnx-community/moonshine-tiny-ONNX/resolve/main/onnx/encoder_model_int8.onnx",
        7919835,
    ),
    (
        "decoder_model_merged.int8.onnx",
        "https://huggingface.co/onnx-community/moonshine-tiny-ONNX/resolve/main/onnx/decoder_model_merged_int8.onnx",
        20189512,
    ),
    (
        "tokenizer.json",
        "https://huggingface.co/onnx-community/moonshine-tiny-ONNX/resolve/main/tokenizer.json",
        3761754,
    ),
];

const WHISPER_TINY_FILES: &[(&str, &str, u64)] = &[(
    "ggml-tiny.bin",
    "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny.bin",
    77691713,
)];

const WHISPER_TINY_EN_FILES: &[(&str, &str, u64)] = &[(
    "ggml-tiny.en.bin",
    "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny.en.bin",
    77704715,
)];

const WHISPER_BASE_FILES: &[(&str, &str, u64)] = &[(
    "ggml-base.bin",
    "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.bin",
    147951465,
)];

const WHISPER_BASE_EN_FILES: &[(&str, &str, u64)] = &[(
    "ggml-base.en.bin",
    "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.en.bin",
    147964211,
)];

const WHISPER_SMALL_FILES: &[(&str, &str, u64)] = &[(
    "ggml-small.bin",
    "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.bin",
    487601967,
)];

const WHISPER_SMALL_EN_FILES: &[(&str, &str, u64)] = &[(
    "ggml-small.en.bin",
    "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.en.bin",
    487614201,
)];

const WHISPER_MEDIUM_FILES: &[(&str, &str, u64)] = &[(
    "ggml-medium.bin",
    "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-medium.bin",
    1533763059,
)];

const WHISPER_MEDIUM_EN_FILES: &[(&str, &str, u64)] = &[(
    "ggml-medium.en.bin",
    "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-medium.en.bin",
    1533774781,
)];

const WHISPER_LARGE_V1_FILES: &[(&str, &str, u64)] = &[(
    "ggml-large-v1.bin",
    "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v1.bin",
    3094623691,
)];

const WHISPER_LARGE_V2_FILES: &[(&str, &str, u64)] = &[(
    "ggml-large-v2.bin",
    "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v2.bin",
    3094623691,
)];

const WHISPER_LARGE_V3_FILES: &[(&str, &str, u64)] = &[(
    "ggml-large-v3.bin",
    "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v3.bin",
    3095033483,
)];

const WHISPER_LARGE_V3_TURBO_FILES: &[(&str, &str, u64)] = &[(
    "ggml-large-v3-turbo.bin",
    "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v3-turbo.bin",
    1624555275,
)];

pub const MODELS: &[Model] = &[
    Model {
        id: "whisper-medium",
        name: "Whisper Medium",
        engine: Engine::Whisper,
        quant: Quant::Fp32,
        languages: &["en", "zh", "de", "es", "ru", "ko", "fr", "ja", "pt", "tr", "pl", "ca", "nl", "ar", "sv", "it", "id", "hi", "fi", "vi", "he", "uk", "el", "ms", "cs", "ro", "da", "hu", "ta", "no", "th", "ur", "hr", "bg", "lt", "la", "mi", "ml", "cy", "sk", "te", "fa", "lv", "bn", "sr", "az", "sl", "kn", "et", "mk", "br", "eu", "is", "hy", "ne", "mn", "bs", "kk", "sq", "sw", "gl", "mr", "pa", "si", "km", "sn", "yo", "so", "af", "oc", "ka", "be", "tg", "sd", "gu", "am", "yi", "lo", "uz", "fo", "ht", "ps", "tk", "nn", "mt", "sa", "lb", "my", "bo", "tl", "mg", "as", "tt", "haw", "ln", "ha", "ba", "jw", "su"],
        language_count: 99,
        bytes: 1533763059,
        licence: "Apache-2.0",
        accuracy: Some("A- (4.64% error)"),
        speed: Some("7.3 s / 30 s"),
        description: "Mid-size OpenAI Whisper for 99 languages, with language detection and translation; slower on CPU.",
        badges: &[Badge::Recommended, Badge::Translate],
        supported: true,
        unsupported_reason: None,
        source: Some(Source::Files(WHISPER_MEDIUM_FILES)),
        required: &["ggml-medium.bin"],
    },
    Model {
        id: "breeze-asr-25",
        name: "Breeze-ASR-25",
        engine: Engine::None,
        quant: Quant::Fp32,
        languages: &["zh", "en"],
        language_count: 2,
        bytes: 1700000000,
        licence: "Apache-2.0",
        accuracy: Some("A- (4.14% error)"),
        speed: Some("15 s / 30 s"),
        description: "Whisper fine-tune for Taiwanese Mandarin mixed with English; Traditional script, no punctuation.",
        badges: &[Badge::Translate],
        supported: false,
        unsupported_reason: Some("Whisper fine-tune with no published GGML weights; a conversion would have to ship here"),
        source: None,
        required: &[],
    },
    Model {
        id: "canary-1b-flash",
        name: "Canary 1B Flash",
        engine: Engine::Canary,
        quant: Quant::Int8,
        languages: &["en", "de", "es", "fr"],
        language_count: 4,
        bytes: 939093289,
        licence: "CC-BY-4.0",
        accuracy: Some("A- (4.75% error)"),
        speed: Some("1.9 s / 30 s"),
        description: "NVIDIA's 1B Canary Flash for English, German, Spanish and French, with translation; fast.",
        badges: &[Badge::Translate],
        supported: true,
        unsupported_reason: None,
        source: Some(Source::Files(CANARY1B_FLASH_FILES)),
        required: &["encoder-model.int8.onnx", "decoder-model.int8.onnx", "nemo128.onnx", "vocab.txt"],
    },
    Model {
        id: "canary-1b-v2",
        name: "Canary 1B v2",
        engine: Engine::Canary,
        quant: Quant::Int8,
        languages: &["en", "bg", "hr", "cs", "da", "nl", "et", "fi", "fr", "de", "el", "hu", "it", "lv", "lt", "mt", "pl", "pt", "ro", "sk", "sl", "es", "sv", "ru", "uk"],
        language_count: 25,
        bytes: 1029466298,
        licence: "CC-BY-4.0",
        accuracy: Some("A- (4.47% error)"),
        speed: Some("2.1 s / 30 s"),
        description: "NVIDIA's 1B Canary v2 for 25 European languages, with translation to and from English.",
        badges: &[Badge::Translate],
        supported: true,
        unsupported_reason: None,
        source: Some(Source::Files(CANARY1B_V2_FILES)),
        required: &["encoder-model.int8.onnx", "decoder-model.int8.onnx", "nemo128.onnx", "vocab.txt"],
    },
    Model {
        id: "canary-180m-flash",
        name: "Canary 180M Flash",
        engine: Engine::Canary,
        quant: Quant::Int8,
        languages: &["en", "de", "es", "fr"],
        language_count: 4,
        bytes: 213424426,
        licence: "CC-BY-4.0",
        accuracy: Some("B+ (5.98% error)"),
        speed: Some("0.9 s / 30 s"),
        description: "Small 180M Canary for English, German, Spanish and French; quick on any hardware and can translate.",
        badges: &[Badge::Recommended, Badge::Translate],
        supported: true,
        unsupported_reason: None,
        source: Some(Source::Files(CANARY180_FILES)),
        required: &["encoder-model.int8.onnx", "decoder-model.int8.onnx", "nemo128.onnx", "vocab.txt"],
    },
    Model {
        id: "canary-qwen-2.5b",
        name: "Canary-Qwen 2.5B",
        engine: Engine::None,
        quant: Quant::Fp32,
        languages: &["en"],
        language_count: 1,
        bytes: 2800000000,
        licence: "CC-BY-4.0",
        accuracy: Some("A (3.58% error)"),
        speed: Some("7.5 s / 30 s"),
        description: "English-only 2.5B Canary-Qwen from NVIDIA; very accurate but slow without a GPU.",
        badges: &[],
        supported: false,
        unsupported_reason: Some("No loader for this engine in this build"),
        source: None,
        required: &[],
    },
    Model {
        id: "cohere-int4",
        name: "Cohere Transcribe",
        engine: Engine::Cohere,
        quant: Quant::Int4,
        languages: &["en", "de", "fr", "it", "es", "pt", "el", "nl", "pl", "ar", "vi", "zh", "ja", "ko"],
        language_count: 14,
        bytes: 1323917499,
        licence: "Apache-2.0",
        accuracy: None,
        speed: None,
        description: "Multilingual transcriber covering 14 languages, including Chinese, Japanese and Korean. The heaviest download and the slowest to load.",
        badges: &[],
        supported: true,
        unsupported_reason: None,
        source: Some(Source::Archive(
            "https://blob.handy.computer/cohere-int4.tar.gz",
            1323917499,
        )),
        required: &["cohere-encoder.int4.onnx", "cohere-decoder.int4.onnx", "tokens.txt"],
    },
    Model {
        id: "cohere-transcribe-arabic",
        name: "Cohere Transcribe Arabic",
        engine: Engine::None,
        quant: Quant::Fp32,
        languages: &["ar", "en"],
        language_count: 2,
        bytes: 2400000000,
        licence: "Apache-2.0",
        accuracy: Some("A- (4.88% error)"),
        speed: Some("3.6 s / 30 s"),
        description: "2B Cohere model tuned for Arabic, which also transcribes English.",
        badges: &[],
        supported: false,
        unsupported_reason: Some("No loader for this engine in this build"),
        source: None,
        required: &[],
    },
    Model {
        id: "fun-asr-nano",
        name: "Fun-ASR Nano",
        engine: Engine::None,
        quant: Quant::Fp32,
        languages: &["zh", "en", "ja"],
        language_count: 3,
        bytes: 891000000,
        licence: "FunASR Model License 1.1",
        accuracy: Some("B+ (5.49% error)"),
        speed: Some("2.8 s / 30 s"),
        description: "0.8B Fun-ASR for Mandarin, English and Japanese.",
        badges: &[],
        supported: false,
        unsupported_reason: Some("No loader for this engine in this build"),
        source: None,
        required: &[],
    },
    Model {
        id: "fun-asr-nano-multilingual",
        name: "Fun-ASR Nano Multilingual",
        engine: Engine::None,
        quant: Quant::Fp32,
        languages: &["zh", "en", "ja", "ko", "yue", "vi", "id", "th", "ms", "tl", "ar", "hi"],
        language_count: 31,
        bytes: 891000000,
        licence: "FunASR Model License 1.1",
        accuracy: Some("A- (4.90% error)"),
        speed: Some("3.4 s / 30 s"),
        description: "0.8B Fun-ASR for 31 languages, mostly Asian and European; the lowest Japanese error rate here.",
        badges: &[],
        supported: false,
        unsupported_reason: Some("No loader for this engine in this build"),
        source: None,
        required: &[],
    },
    Model {
        id: "granite-speech-4.0-1b",
        name: "Granite Speech 4.0 1B",
        engine: Engine::None,
        quant: Quant::Fp32,
        languages: &["en", "fr", "de", "es", "pt", "ja"],
        language_count: 6,
        bytes: 2600000000,
        licence: "Apache-2.0",
        accuracy: Some("A- (4.66% error)"),
        speed: Some("6.5 s / 30 s"),
        description: "IBM's Granite Speech 4.0 for English, French, German, Spanish, Portuguese and Japanese, with translation.",
        badges: &[Badge::Translate],
        supported: false,
        unsupported_reason: Some("No loader for this engine in this build"),
        source: None,
        required: &[],
    },
    Model {
        id: "granite-speech-4.1-2b",
        name: "Granite Speech 4.1 2B",
        engine: Engine::None,
        quant: Quant::Fp32,
        languages: &["en", "fr", "de", "es", "pt", "ja"],
        language_count: 6,
        bytes: 2600000000,
        licence: "Apache-2.0",
        accuracy: Some("A- (4.14% error)"),
        speed: Some("6.5 s / 30 s"),
        description: "IBM's 2B Granite Speech 4.1 for English, French, German, Spanish, Portuguese and Japanese, with translation.",
        badges: &[Badge::Translate],
        supported: false,
        unsupported_reason: Some("No loader for this engine in this build"),
        source: None,
        required: &[],
    },
    Model {
        id: "granite-speech-4.1-2b-nar",
        name: "Granite Speech 4.1 2B NAR",
        engine: Engine::None,
        quant: Quant::Fp32,
        languages: &["en", "fr", "de", "es", "pt"],
        language_count: 5,
        bytes: 2500000000,
        licence: "Apache-2.0",
        accuracy: Some("B+ (5.33% error)"),
        speed: Some("7.5 s / 30 s"),
        description: "Non-autoregressive Granite Speech 4.1 for English, French, German, Spanish and Portuguese.",
        badges: &[],
        supported: false,
        unsupported_reason: Some("No loader for this engine in this build"),
        source: None,
        required: &[],
    },
    Model {
        id: "granite-speech-4.1-2b-plus",
        name: "Granite Speech 4.1 2B Plus",
        engine: Engine::None,
        quant: Quant::Fp32,
        languages: &["en", "fr", "de", "es", "pt"],
        language_count: 5,
        bytes: 2300000000,
        licence: "Apache-2.0",
        accuracy: Some("A- (4.46% error)"),
        speed: Some("6.5 s / 30 s"),
        description: "Granite Speech 4.1 Plus for English, French, German, Spanish and Portuguese, with word timestamps.",
        badges: &[],
        supported: false,
        unsupported_reason: Some("No loader for this engine in this build"),
        source: None,
        required: &[],
    },
    Model {
        id: "granite-speech-5.0-470m-turbOctc",
        name: "Granite Speech 5.0 470M TurboCTC",
        engine: Engine::None,
        quant: Quant::Fp32,
        languages: &["en"],
        language_count: 1,
        bytes: 506000000,
        licence: "Apache-2.0",
        accuracy: Some("A- (4.61% error)"),
        speed: Some("1.3 s / 30 s"),
        description: "English-only 470M IBM Granite Speech 5.0 with a CTC decoder; fast, especially on a GPU.",
        badges: &[],
        supported: false,
        unsupported_reason: Some("No loader for this engine in this build"),
        source: None,
        required: &[],
    },
    Model {
        id: "moonshine-base",
        name: "Moonshine Base",
        engine: Engine::MoonshineBase,
        quant: Quant::Fp32,
        languages: &["en"],
        language_count: 1,
        bytes: 57901034,
        licence: "MIT",
        accuracy: Some("C+ (12.25% error)"),
        speed: Some("0.8 s / 30 s"),
        description: "English-only 62M Moonshine; a small, fast download with lower accuracy than larger models.",
        badges: &[],
        supported: true,
        unsupported_reason: None,
        source: Some(Source::Archive(
            "https://blob.handy.computer/moonshine-base.tar.gz",
            57901034,
        )),
        required: &["encoder_model.onnx", "decoder_model_merged.onnx", "tokenizer.json"],
    },
    Model {
        id: "moonshine-tiny",
        name: "Moonshine Tiny",
        engine: Engine::MoonshineTiny,
        quant: Quant::Int8,
        languages: &["en"],
        language_count: 1,
        bytes: 31871101,
        licence: "MIT",
        accuracy: Some("C (14.13% error)"),
        speed: Some("0.5 s / 30 s"),
        description: "English-only 27M Moonshine, the smallest download here; very fast, with low accuracy.",
        badges: &[],
        supported: true,
        unsupported_reason: None,
        source: Some(Source::Files(MOONSHINE_TINY_FILES)),
        required: &["encoder_model.int8.onnx", "decoder_model_merged.int8.onnx", "tokenizer.json"],
    },
    Model {
        id: "moonshine-streaming-tiny",
        name: "Moonshine Streaming Tiny",
        engine: Engine::None,
        quant: Quant::Fp32,
        languages: &["en"],
        language_count: 1,
        bytes: 50000000,
        licence: "MIT",
        accuracy: Some("C- (18.18% error)"),
        speed: Some("0.5 s / 30 s"),
        description: "English-only 44M streaming Moonshine; very small and fast, with low accuracy.",
        badges: &[Badge::LivePreview],
        supported: false,
        unsupported_reason: Some("Streaming-only export: Nova transcribes each recording after you stop"),
        source: None,
        required: &[],
    },
    Model {
        id: "moonshine-streaming-small",
        name: "Moonshine Streaming Small",
        engine: Engine::None,
        quant: Quant::Fp32,
        languages: &["en"],
        language_count: 1,
        bytes: 199000000,
        licence: "MIT",
        accuracy: Some("B- (8.55% error)"),
        speed: Some("1.4 s / 30 s"),
        description: "English-only streaming Moonshine; between the tiny and medium sizes in speed and accuracy.",
        badges: &[Badge::LivePreview],
        supported: false,
        unsupported_reason: Some("Streaming-only export: Nova transcribes each recording after you stop"),
        source: None,
        required: &[],
    },
    Model {
        id: "moonshine-streaming-medium",
        name: "Moonshine Streaming Medium",
        engine: Engine::None,
        quant: Quant::Fp32,
        languages: &["en"],
        language_count: 1,
        bytes: 296000000,
        licence: "MIT",
        accuracy: Some("B (7.87% error)"),
        speed: Some("2.1 s / 30 s"),
        description: "English-only streaming Moonshine; the most accurate of the three streaming sizes.",
        badges: &[Badge::LivePreview],
        supported: false,
        unsupported_reason: Some("Streaming-only export: Nova transcribes each recording after you stop"),
        source: None,
        required: &[],
    },
    Model {
        id: "moss-transcribe-diarize",
        name: "MOSS Transcribe Diarize",
        engine: Engine::None,
        quant: Quant::Fp32,
        languages: &["en", "zh"],
        language_count: 2,
        bytes: 987000000,
        licence: "Apache-2.0",
        accuracy: Some("B+ (5.13% error)"),
        speed: Some("9.8 s / 30 s"),
        description: "0.9B MOSS model for English and Mandarin, built for long recordings with several speakers.",
        badges: &[],
        supported: false,
        unsupported_reason: Some("No loader for this engine in this build"),
        source: None,
        required: &[],
    },
    Model {
        id: "nemotron-speech-streaming-en-0.6b",
        name: "Nemotron Speech Streaming English 0.6B",
        engine: Engine::None,
        quant: Quant::Fp32,
        languages: &["en"],
        language_count: 1,
        bytes: 730000000,
        licence: "NVIDIA Open Model License",
        accuracy: Some("B (6.43% error)"),
        speed: Some("1.1 s / 30 s"),
        description: "English-only streaming 0.6B Nemotron from NVIDIA, for low-latency live transcription.",
        badges: &[Badge::LivePreview],
        supported: false,
        unsupported_reason: Some("Streaming-only export: Nova transcribes each recording after you stop"),
        source: None,
        required: &[],
    },
    Model {
        id: "orukeet",
        name: "Orukeet",
        engine: Engine::None,
        quant: Quant::Fp32,
        languages: &["en", "bg", "hr", "cs", "da", "nl", "et", "fi", "fr", "de", "el", "hu", "it", "lv", "lt", "mt", "pl", "pt", "ro", "sk", "sl", "es", "sv", "ru", "uk"],
        language_count: 25,
        bytes: 740000000,
        licence: "CC-BY-SA-4.0",
        accuracy: Some("A- (4.43% error)"),
        speed: Some("1.2 s / 30 s"),
        description: "Oruk AI's retrained Parakeet TDT v3 for 25 European languages, with lower error rates than the original.",
        badges: &[],
        supported: false,
        unsupported_reason: Some("No loader for this engine in this build"),
        source: None,
        required: &[],
    },
    Model {
        id: "parakeet-ctc-0.6b",
        name: "Parakeet CTC 0.6B",
        engine: Engine::None,
        quant: Quant::Fp32,
        languages: &["en"],
        language_count: 1,
        bytes: 722000000,
        licence: "CC-BY-4.0",
        accuracy: Some("B+ (5.53% error)"),
        speed: Some("1.1 s / 30 s"),
        description: "English-only 0.6B Parakeet with a CTC decoder; fast on any hardware.",
        badges: &[],
        supported: false,
        unsupported_reason: Some("No loader for this engine in this build"),
        source: None,
        required: &[],
    },
    Model {
        id: "parakeet-ctc-1.1b",
        name: "Parakeet CTC 1.1B",
        engine: Engine::None,
        quant: Quant::Fp32,
        languages: &["en"],
        language_count: 1,
        bytes: 1300000000,
        licence: "CC-BY-4.0",
        accuracy: Some("B+ (5.61% error)"),
        speed: Some("1.8 s / 30 s"),
        description: "English-only 1.1B Parakeet with a CTC decoder; larger and slower than the 0.6B.",
        badges: &[],
        supported: false,
        unsupported_reason: Some("No loader for this engine in this build"),
        source: None,
        required: &[],
    },
    Model {
        id: "parakeet-multitalker-en",
        name: "Parakeet Multitalker English",
        engine: Engine::None,
        quant: Quant::Fp32,
        languages: &["en"],
        language_count: 1,
        bytes: 873000000,
        licence: "NVIDIA Open Model License",
        accuracy: Some("B (6.52% error)"),
        speed: Some("1.1 s / 30 s"),
        description: "English-only streaming Parakeet for overlapping speakers in meetings, not solo dictation.",
        badges: &[Badge::LivePreview],
        supported: false,
        unsupported_reason: Some("Streaming-only export: Nova transcribes each recording after you stop"),
        source: None,
        required: &[],
    },
    Model {
        id: "parakeet-primeline",
        name: "Parakeet Primeline",
        engine: Engine::None,
        quant: Quant::Fp32,
        languages: &["en", "bg", "hr", "cs", "da", "nl", "et", "fi", "fr", "de", "el", "hu", "it", "lv", "lt", "mt", "pl", "pt", "ro", "sk", "sl", "es", "sv", "ru", "uk"],
        language_count: 25,
        bytes: 740000000,
        licence: "CC-BY-4.0",
        accuracy: Some("A- (4.82% error)"),
        speed: Some("1.2 s / 30 s"),
        description: "Primeline's Parakeet TDT v3 fine-tuned for German, still covering 25 European languages.",
        badges: &[],
        supported: false,
        unsupported_reason: Some("No loader for this engine in this build"),
        source: None,
        required: &[],
    },
    Model {
        id: "parakeet-rnnt-0.6b",
        name: "Parakeet RNN-T 0.6B",
        engine: Engine::None,
        quant: Quant::Fp32,
        languages: &["en"],
        language_count: 1,
        bytes: 730000000,
        licence: "CC-BY-4.0",
        accuracy: Some("A- (4.57% error)"),
        speed: Some("1.1 s / 30 s"),
        description: "English-only 0.6B Parakeet with an RNN-T decoder; fast on any hardware.",
        badges: &[],
        supported: false,
        unsupported_reason: Some("No loader for this engine in this build"),
        source: None,
        required: &[],
    },
    Model {
        id: "parakeet-rnnt-1.1b",
        name: "Parakeet RNN-T 1.1B",
        engine: Engine::None,
        quant: Quant::Fp32,
        languages: &["en"],
        language_count: 1,
        bytes: 1300000000,
        licence: "CC-BY-4.0",
        accuracy: Some("A- (4.45% error)"),
        speed: Some("1.9 s / 30 s"),
        description: "English-only 1.1B Parakeet with an RNN-T decoder; larger and slower than the 0.6B.",
        badges: &[],
        supported: false,
        unsupported_reason: Some("No loader for this engine in this build"),
        source: None,
        required: &[],
    },
    Model {
        id: "parakeet-tdt-0.6b-v2",
        name: "Parakeet TDT 0.6B v2",
        engine: Engine::Parakeet,
        quant: Quant::Int8,
        languages: &["en"],
        language_count: 1,
        bytes: 661331448,
        licence: "CC-BY-4.0",
        accuracy: Some("A- (4.11% error)"),
        speed: Some("1.1 s / 30 s"),
        description: "English-only 0.6B Parakeet TDT v2 from NVIDIA; fast and accurate on any hardware.",
        badges: &[],
        supported: true,
        unsupported_reason: None,
        source: Some(Source::Files(PARAKEET_V2_FILES)),
        required: &["encoder-model.int8.onnx", "decoder_joint-model.int8.onnx", "nemo128.onnx", "vocab.txt"],
    },
    Model {
        id: "parakeet-tdt-0.6b-v3",
        name: "Parakeet TDT 0.6B v3",
        engine: Engine::Parakeet,
        quant: Quant::Int8,
        languages: &["en", "bg", "hr", "cs", "da", "nl", "et", "fi", "fr", "de", "el", "hu", "it", "lv", "lt", "mt", "pl", "pt", "ro", "sk", "sl", "es", "sv", "ru", "uk"],
        language_count: 25,
        bytes: 670619706,
        licence: "CC-BY-4.0",
        accuracy: Some("A- (4.83% error)"),
        speed: Some("1.2 s / 30 s"),
        description: "NVIDIA's 0.6B Parakeet TDT v3 for 25 European languages, with language detection; fast on any hardware.",
        badges: &[],
        supported: true,
        unsupported_reason: None,
        source: Some(Source::Files(PARAKEET_V3_FILES)),
        required: &["encoder-model.int8.onnx", "decoder_joint-model.int8.onnx", "nemo128.onnx", "vocab.txt"],
    },
    Model {
        id: "parakeet-tdt-1.1b",
        name: "Parakeet TDT 1.1B",
        engine: Engine::None,
        quant: Quant::Fp32,
        languages: &["en"],
        language_count: 1,
        bytes: 1300000000,
        licence: "CC-BY-4.0",
        accuracy: Some("A- (4.24% error)"),
        speed: Some("1.8 s / 30 s"),
        description: "English-only 1.1B Parakeet TDT; larger and slower than the 0.6B v2.",
        badges: &[],
        supported: false,
        unsupported_reason: Some("No loader for this engine in this build"),
        source: None,
        required: &[],
    },
    Model {
        id: "parakeet-tdt-ctc-1.1b",
        name: "Parakeet TDT-CTC 1.1B",
        engine: Engine::None,
        quant: Quant::Fp32,
        languages: &["en"],
        language_count: 1,
        bytes: 1300000000,
        licence: "CC-BY-4.0",
        accuracy: Some("A- (4.68% error)"),
        speed: Some("1.8 s / 30 s"),
        description: "English-only 1.1B Parakeet with a hybrid TDT-CTC decoder.",
        badges: &[],
        supported: false,
        unsupported_reason: Some("No loader for this engine in this build"),
        source: None,
        required: &[],
    },
    Model {
        id: "parakeet-tdt-ctc-110m",
        name: "Parakeet TDT-CTC 110M",
        engine: Engine::None,
        quant: Quant::Fp32,
        languages: &["en"],
        language_count: 1,
        bytes: 135000000,
        licence: "CC-BY-4.0",
        accuracy: Some("B+ (6.11% error)"),
        speed: Some("0.4 s / 30 s"),
        description: "English-only 110M Parakeet; a small download that is near-instant even on CPU.",
        badges: &[],
        supported: false,
        unsupported_reason: Some("No loader for this engine in this build"),
        source: None,
        required: &[],
    },
    Model {
        id: "parakeet-ultra",
        name: "Parakeet Ultra",
        engine: Engine::None,
        quant: Quant::Fp32,
        languages: &["en", "bg", "hr", "cs", "da", "nl", "et", "fi", "fr", "de", "el", "hu", "it", "lv", "lt", "mt", "pl", "pt", "ro", "sk", "sl", "es", "sv", "ru", "uk"],
        language_count: 25,
        bytes: 740000000,
        licence: "CC-BY-4.0",
        accuracy: Some("A- (4.26% error)"),
        speed: Some("1.3 s / 30 s"),
        description: "Moondream's retrained Parakeet TDT v3 for 25 European languages; more accurate in noise and on long audio.",
        badges: &[],
        supported: false,
        unsupported_reason: Some("No loader for this engine in this build"),
        source: None,
        required: &[],
    },
    Model {
        id: "qwen3-asr-0.6b",
        name: "Qwen3-ASR 0.6B",
        engine: Engine::None,
        quant: Quant::Fp32,
        languages: &["zh", "yue", "en"],
        language_count: 30,
        bytes: 850000000,
        licence: "Apache-2.0",
        accuracy: Some("A- (4.23% error)"),
        speed: Some("3.1 s / 30 s"),
        description: "Alibaba's 0.6B Qwen3-ASR for 30 languages including Mandarin and Cantonese, with language detection.",
        badges: &[],
        supported: false,
        unsupported_reason: Some("No loader for this engine in this build"),
        source: None,
        required: &[],
    },
    Model {
        id: "qwen3-asr-1.7b",
        name: "Qwen3-ASR 1.7B",
        engine: Engine::None,
        quant: Quant::Fp32,
        languages: &["zh", "yue", "en"],
        language_count: 30,
        bytes: 2200000000,
        licence: "Apache-2.0",
        accuracy: Some("A (3.23% error)"),
        speed: Some("7.3 s / 30 s"),
        description: "Alibaba's 1.7B Qwen3-ASR for 30 languages including Mandarin and Cantonese; best on Apple Silicon or a GPU.",
        badges: &[],
        supported: false,
        unsupported_reason: Some("No loader for this engine in this build"),
        source: None,
        required: &[],
    },
    Model {
        id: "sense-voice-small",
        name: "SenseVoice Small",
        engine: Engine::SenseVoice,
        quant: Quant::Int8,
        languages: &["zh", "en", "ja", "ko", "yue"],
        language_count: 5,
        bytes: 160307845,
        licence: "FunASR Model License 1.1",
        accuracy: Some("B (7.14% error)"),
        speed: Some("1.1 s / 30 s"),
        description: "Small, fast model for Mandarin, English, Japanese and Korean; lists Cantonese but is weak at it.",
        badges: &[],
        supported: true,
        unsupported_reason: None,
        source: Some(Source::Archive(
            "https://blob.handy.computer/sense-voice-int8.tar.gz",
            160307845,
        )),
        required: &["model.int8.onnx", "tokens.txt"],
    },
    Model {
        id: "voxtral-mini-3b",
        name: "Voxtral Mini 3B",
        engine: Engine::None,
        quant: Quant::Fp32,
        languages: &["en", "fr", "de", "es", "it", "pt", "nl", "hi"],
        language_count: 8,
        bytes: 5000000000,
        licence: "Apache-2.0",
        accuracy: Some("A (3.89% error)"),
        speed: Some("28 s / 30 s"),
        description: "Mistral's Voxtral Mini 3B for 8 languages, with translation; best on Apple Silicon or a GPU.",
        badges: &[Badge::Translate],
        supported: false,
        unsupported_reason: Some("No loader for this engine in this build"),
        source: None,
        required: &[],
    },
    Model {
        id: "voxtral-mini-4b-realtime",
        name: "Voxtral Mini 4B Realtime",
        engine: Engine::None,
        quant: Quant::Fp32,
        languages: &["en", "fr", "de", "es", "it", "pt", "nl", "hi"],
        language_count: 13,
        bytes: 4700000000,
        licence: "Apache-2.0",
        accuracy: Some("B+ (6.27% error)"),
        speed: Some("39 s / 30 s"),
        description: "4B Mistral Voxtral with streaming for 13 languages; best on Apple Silicon or a discrete GPU.",
        badges: &[Badge::LivePreview],
        supported: false,
        unsupported_reason: Some("Streaming-only export: Nova transcribes each recording after you stop"),
        source: None,
        required: &[],
    },
    Model {
        id: "voxtral-small-24b",
        name: "Voxtral Small 24B",
        engine: Engine::None,
        quant: Quant::Fp32,
        languages: &["en", "fr", "de", "es", "it", "pt", "nl", "hi"],
        language_count: 8,
        bytes: 25800000000,
        licence: "Apache-2.0",
        accuracy: Some("A (3.55% error)"),
        speed: None,
        description: "Mistral's 24B Voxtral Small for 8 languages, with translation; a 26 GB download that needs lots of memory.",
        badges: &[Badge::Translate],
        supported: false,
        unsupported_reason: Some("No loader for this engine in this build"),
        source: None,
        required: &[],
    },
    Model {
        id: "whisper-base",
        name: "Whisper Base",
        engine: Engine::Whisper,
        quant: Quant::Fp32,
        languages: &["en", "zh", "de", "es", "ru", "ko", "fr", "ja", "pt", "tr", "pl", "ca", "nl", "ar", "sv", "it", "id", "hi", "fi", "vi", "he", "uk", "el", "ms", "cs", "ro", "da", "hu", "ta", "no", "th", "ur", "hr", "bg", "lt", "la", "mi", "ml", "cy", "sk", "te", "fa", "lv", "bn", "sr", "az", "sl", "kn", "et", "mk", "br", "eu", "is", "hy", "ne", "mn", "bs", "kk", "sq", "sw", "gl", "mr", "pa", "si", "km", "sn", "yo", "so", "af", "oc", "ka", "be", "tg", "sd", "gu", "am", "yi", "lo", "uz", "fo", "ht", "ps", "tk", "nn", "mt", "sa", "lb", "my", "bo", "tl", "mg", "as", "tt", "haw", "ln", "ha", "ba", "jw", "su"],
        language_count: 99,
        bytes: 147951465,
        licence: "Apache-2.0",
        accuracy: Some("B- (9.88% error)"),
        speed: Some("1.0 s / 30 s"),
        description: "Small Whisper Base for 99 languages, with language detection and translation; fast but less accurate.",
        badges: &[Badge::Translate],
        supported: true,
        unsupported_reason: None,
        source: Some(Source::Files(WHISPER_BASE_FILES)),
        required: &["ggml-base.bin"],
    },
    Model {
        id: "whisper-base-en",
        name: "Whisper Base English",
        engine: Engine::Whisper,
        quant: Quant::Fp32,
        languages: &["en"],
        language_count: 1,
        bytes: 147964211,
        licence: "Apache-2.0",
        accuracy: Some("B (7.60% error)"),
        speed: Some("0.9 s / 30 s"),
        description: "English-only Whisper Base; small and fast, with a lower English error rate than the 99-language Base.",
        badges: &[],
        supported: true,
        unsupported_reason: None,
        source: Some(Source::Files(WHISPER_BASE_EN_FILES)),
        required: &["ggml-base.en.bin"],
    },
    Model {
        id: "whisper-large-v1",
        name: "Whisper Large v1",
        engine: Engine::Whisper,
        quant: Quant::Fp32,
        languages: &["en", "zh", "de", "es", "ru", "ko", "fr", "ja", "pt", "tr", "pl", "ca", "nl", "ar", "sv", "it", "id", "hi", "fi", "vi", "he", "uk", "el", "ms", "cs", "ro", "da", "hu", "ta", "no", "th", "ur", "hr", "bg", "lt", "la", "mi", "ml", "cy", "sk", "te", "fa", "lv", "bn", "sr", "az", "sl", "kn", "et", "mk", "br", "eu", "is", "hy", "ne", "mn", "bs", "kk", "sq", "sw", "gl", "mr", "pa", "si", "km", "sn", "yo", "so", "af", "oc", "ka", "be", "tg", "sd", "gu", "am", "yi", "lo", "uz", "fo", "ht", "ps", "tk", "nn", "mt", "sa", "lb", "my", "bo", "tl", "mg", "as", "tt", "haw", "ln", "ha", "ba", "jw", "su"],
        language_count: 99,
        bytes: 3094623691,
        licence: "Apache-2.0",
        accuracy: Some("A- (4.46% error)"),
        speed: Some("15 s / 30 s"),
        description: "Original Whisper Large (v1) for 99 languages, with language detection and translation; slow without a GPU.",
        badges: &[Badge::Translate],
        supported: true,
        unsupported_reason: None,
        source: Some(Source::Files(WHISPER_LARGE_V1_FILES)),
        required: &["ggml-large-v1.bin"],
    },
    Model {
        id: "whisper-large-v2",
        name: "Whisper Large v2",
        engine: Engine::Whisper,
        quant: Quant::Fp32,
        languages: &["en", "zh", "de", "es", "ru", "ko", "fr", "ja", "pt", "tr", "pl", "ca", "nl", "ar", "sv", "it", "id", "hi", "fi", "vi", "he", "uk", "el", "ms", "cs", "ro", "da", "hu", "ta", "no", "th", "ur", "hr", "bg", "lt", "la", "mi", "ml", "cy", "sk", "te", "fa", "lv", "bn", "sr", "az", "sl", "kn", "et", "mk", "br", "eu", "is", "hy", "ne", "mn", "bs", "kk", "sq", "sw", "gl", "mr", "pa", "si", "km", "sn", "yo", "so", "af", "oc", "ka", "be", "tg", "sd", "gu", "am", "yi", "lo", "uz", "fo", "ht", "ps", "tk", "nn", "mt", "sa", "lb", "my", "bo", "tl", "mg", "as", "tt", "haw", "ln", "ha", "ba", "jw", "su"],
        language_count: 99,
        bytes: 3094623691,
        licence: "Apache-2.0",
        accuracy: Some("A- (4.21% error)"),
        speed: Some("15 s / 30 s"),
        description: "Whisper Large v2 for 99 languages, with language detection and translation; slow without a GPU.",
        badges: &[Badge::Translate],
        supported: true,
        unsupported_reason: None,
        source: Some(Source::Files(WHISPER_LARGE_V2_FILES)),
        required: &["ggml-large-v2.bin"],
    },
    Model {
        id: "whisper-large-v3",
        name: "Whisper Large v3",
        engine: Engine::Whisper,
        quant: Quant::Fp32,
        languages: &["en", "zh", "de", "es", "ru", "ko", "fr", "ja", "pt", "tr", "pl", "ca", "nl", "ar", "sv", "it", "id", "hi", "fi", "vi", "he", "uk", "el", "ms", "cs", "ro", "da", "hu", "ta", "no", "th", "ur", "hr", "bg", "lt", "la", "mi", "ml", "cy", "sk", "te", "fa", "lv", "bn", "sr", "az", "sl", "kn", "et", "mk", "br", "eu", "is", "hy", "ne", "mn", "bs", "kk", "sq", "sw", "gl", "mr", "pa", "si", "km", "sn", "yo", "so", "af", "oc", "ka", "be", "tg", "sd", "gu", "am", "yi", "lo", "uz", "fo", "ht", "ps", "tk", "nn", "mt", "sa", "lb", "my", "bo", "tl", "mg", "as", "tt", "haw", "ln", "ha", "ba", "jw", "su", "yue"],
        language_count: 100,
        bytes: 3095033483,
        licence: "Apache-2.0",
        accuracy: Some("A- (4.03% error)"),
        speed: Some("15 s / 30 s"),
        description: "OpenAI's Whisper Large v3 for 100 languages, with language detection and translation; best with a GPU.",
        badges: &[Badge::Translate],
        supported: true,
        unsupported_reason: None,
        source: Some(Source::Files(WHISPER_LARGE_V3_FILES)),
        required: &["ggml-large-v3.bin"],
    },
    Model {
        id: "whisper-large-v3-turbo",
        name: "Whisper Large v3 Turbo",
        engine: Engine::Whisper,
        quant: Quant::Fp32,
        languages: &["en", "zh", "de", "es", "ru", "ko", "fr", "ja", "pt", "tr", "pl", "ca", "nl", "ar", "sv", "it", "id", "hi", "fi", "vi", "he", "uk", "el", "ms", "cs", "ro", "da", "hu", "ta", "no", "th", "ur", "hr", "bg", "lt", "la", "mi", "ml", "cy", "sk", "te", "fa", "lv", "bn", "sr", "az", "sl", "kn", "et", "mk", "br", "eu", "is", "hy", "ne", "mn", "bs", "kk", "sq", "sw", "gl", "mr", "pa", "si", "km", "sn", "yo", "so", "af", "oc", "ka", "be", "tg", "sd", "gu", "am", "yi", "lo", "uz", "fo", "ht", "ps", "tk", "nn", "mt", "sa", "lb", "my", "bo", "tl", "mg", "as", "tt", "haw", "ln", "ha", "ba", "jw", "su", "yue"],
        language_count: 100,
        bytes: 1624555275,
        licence: "Apache-2.0",
        accuracy: Some("A- (4.38% error)"),
        speed: Some("9.7 s / 30 s"),
        description: "Pruned, faster Whisper Large v3 for 100 languages, with language detection; slow without a GPU.",
        badges: &[],
        supported: true,
        unsupported_reason: None,
        source: Some(Source::Files(WHISPER_LARGE_V3_TURBO_FILES)),
        required: &["ggml-large-v3-turbo.bin"],
    },
    Model {
        id: "whisper-medium-en",
        name: "Whisper Medium English",
        engine: Engine::Whisper,
        quant: Quant::Fp32,
        languages: &["en"],
        language_count: 1,
        bytes: 1533774781,
        licence: "Apache-2.0",
        accuracy: Some("A- (4.88% error)"),
        speed: Some("6.8 s / 30 s"),
        description: "English-only Whisper Medium; accurate, but slower than Parakeet models of similar size.",
        badges: &[],
        supported: true,
        unsupported_reason: None,
        source: Some(Source::Files(WHISPER_MEDIUM_EN_FILES)),
        required: &["ggml-medium.en.bin"],
    },
    Model {
        id: "whisper-small",
        name: "Whisper Small",
        engine: Engine::Whisper,
        quant: Quant::Fp32,
        languages: &["en", "zh", "de", "es", "ru", "ko", "fr", "ja", "pt", "tr", "pl", "ca", "nl", "ar", "sv", "it", "id", "hi", "fi", "vi", "he", "uk", "el", "ms", "cs", "ro", "da", "hu", "ta", "no", "th", "ur", "hr", "bg", "lt", "la", "mi", "ml", "cy", "sk", "te", "fa", "lv", "bn", "sr", "az", "sl", "kn", "et", "mk", "br", "eu", "is", "hy", "ne", "mn", "bs", "kk", "sq", "sw", "gl", "mr", "pa", "si", "km", "sn", "yo", "so", "af", "oc", "ka", "be", "tg", "sd", "gu", "am", "yi", "lo", "uz", "fo", "ht", "ps", "tk", "nn", "mt", "sa", "lb", "my", "bo", "tl", "mg", "as", "tt", "haw", "ln", "ha", "ba", "jw", "su"],
        language_count: 99,
        bytes: 487601967,
        licence: "Apache-2.0",
        accuracy: Some("B (6.51% error)"),
        speed: Some("2.6 s / 30 s"),
        description: "Small OpenAI Whisper for 99 languages, with language detection and translation.",
        badges: &[Badge::Translate],
        supported: true,
        unsupported_reason: None,
        source: Some(Source::Files(WHISPER_SMALL_FILES)),
        required: &["ggml-small.bin"],
    },
    Model {
        id: "whisper-small-en",
        name: "Whisper Small English",
        engine: Engine::Whisper,
        quant: Quant::Fp32,
        languages: &["en"],
        language_count: 1,
        bytes: 487614201,
        licence: "Apache-2.0",
        accuracy: Some("B+ (6.14% error)"),
        speed: Some("2.4 s / 30 s"),
        description: "English-only Whisper Small; compact, with moderate speed and accuracy.",
        badges: &[],
        supported: true,
        unsupported_reason: None,
        source: Some(Source::Files(WHISPER_SMALL_EN_FILES)),
        required: &["ggml-small.en.bin"],
    },
    Model {
        id: "whisper-tiny",
        name: "Whisper Tiny",
        engine: Engine::Whisper,
        quant: Quant::Fp32,
        languages: &["en", "zh", "de", "es", "ru", "ko", "fr", "ja", "pt", "tr", "pl", "ca", "nl", "ar", "sv", "it", "id", "hi", "fi", "vi", "he", "uk", "el", "ms", "cs", "ro", "da", "hu", "ta", "no", "th", "ur", "hr", "bg", "lt", "la", "mi", "ml", "cy", "sk", "te", "fa", "lv", "bn", "sr", "az", "sl", "kn", "et", "mk", "br", "eu", "is", "hy", "ne", "mn", "bs", "kk", "sq", "sw", "gl", "mr", "pa", "si", "km", "sn", "yo", "so", "af", "oc", "ka", "be", "tg", "sd", "gu", "am", "yi", "lo", "uz", "fo", "ht", "ps", "tk", "nn", "mt", "sa", "lb", "my", "bo", "tl", "mg", "as", "tt", "haw", "ln", "ha", "ba", "jw", "su"],
        language_count: 99,
        bytes: 77691713,
        licence: "Apache-2.0",
        accuracy: Some("C (13.84% error)"),
        speed: Some("0.7 s / 30 s"),
        description: "Smallest Whisper, for 99 languages with language detection and translation; very fast, low accuracy.",
        badges: &[Badge::Translate],
        supported: true,
        unsupported_reason: None,
        source: Some(Source::Files(WHISPER_TINY_FILES)),
        required: &["ggml-tiny.bin"],
    },
    Model {
        id: "whisper-tiny-en",
        name: "Whisper Tiny English",
        engine: Engine::Whisper,
        quant: Quant::Fp32,
        languages: &["en"],
        language_count: 1,
        bytes: 77704715,
        licence: "Apache-2.0",
        accuracy: Some("C+ (10.72% error)"),
        speed: Some("0.6 s / 30 s"),
        description: "English-only Whisper Tiny; very small and fast, with low accuracy.",
        badges: &[],
        supported: true,
        unsupported_reason: None,
        source: Some(Source::Files(WHISPER_TINY_EN_FILES)),
        required: &["ggml-tiny.en.bin"],
    },
];

/// The catalogue as the UI receives it: borrowed fields become owned strings and
/// languages gain their display names, so the frontend needs no copy of the table.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelView {
    pub id: String,
    pub name: String,
    pub engine: Engine,
    pub languages: Vec<LanguageView>,
    pub language_count: u16,
    pub bytes: u64,
    pub licence: String,
    pub accuracy: Option<String>,
    pub speed: Option<String>,
    pub description: String,
    pub badges: Vec<Badge>,
    pub supported: bool,
    pub unsupported_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LanguageView {
    pub id: &'static str,
    pub name: &'static str,
}

impl Engine {
    /// The stable string the manifest records. Informational only: every decision
    /// about whether a model can run reads the files on disk and this build's own
    /// support, never this label.
    pub fn label(self) -> &'static str {
        match self {
            Engine::Parakeet => "parakeet",
            Engine::Canary => "canary",
            Engine::Cohere => "cohere",
            Engine::MoonshineBase => "moonshineBase",
            Engine::MoonshineTiny => "moonshineTiny",
            Engine::SenseVoice => "senseVoice",
            Engine::Whisper => "whisper",
            Engine::None => "none",
        }
    }
}

impl Model {
    /// Display names for the languages this model names, in LANGUAGES order.
    pub fn language_views(&self) -> Vec<LanguageView> {
        LANGUAGES
            .iter()
            .filter(|(code, _)| self.languages.contains(code))
            .map(|(id, name)| LanguageView { id, name })
            .collect()
    }

    pub fn view(&self) -> ModelView {
        ModelView {
            id: self.id.to_string(),
            name: self.name.to_string(),
            engine: self.engine,
            languages: self.language_views(),
            language_count: self.language_count,
            bytes: self.bytes,
            licence: self.licence.to_string(),
            accuracy: self.accuracy.map(str::to_string),
            speed: self.speed.map(str::to_string),
            description: self.description.to_string(),
            badges: self.badges.to_vec(),
            supported: self.supported,
            unsupported_reason: self.unsupported_reason.map(str::to_string),
        }
    }
}

pub fn find(id: &str) -> Option<&'static Model> {
    MODELS.iter().find(|model| model.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_match_their_download_spec() {
        for model in MODELS.iter().filter(|model| model.source.is_some()) {
            let listed: u64 = match model.source {
                Some(Source::Files(files)) => files.iter().map(|(_, _, bytes)| *bytes).sum(),
                Some(Source::Archive(_, bytes)) => bytes,
                None => 0,
            };
            assert_eq!(model.bytes, listed, "{} card size drifted", model.id);
        }
    }

    #[test]
    fn every_model_shows_a_size() {
        for model in MODELS {
            assert!(model.bytes > 0, "{} has no size to show", model.id);
        }
    }

    #[test]
    fn languages_come_from_the_shared_table() {
        for model in MODELS {
            for code in model.languages {
                assert!(
                    LANGUAGES.iter().any(|(id, _)| id == code),
                    "{} lists {code}, which is not in LANGUAGES",
                    model.id
                );
            }
        }
    }

    #[test]
    fn named_languages_never_exceed_the_published_count() {
        for model in MODELS {
            assert!(
                model.languages.len() <= model.language_count as usize,
                "{} names more languages than it publishes",
                model.id
            );
            assert!(
                model.language_count > 0,
                "{} publishes no languages",
                model.id
            );
        }
    }

    #[test]
    fn unsupported_models_cannot_be_downloaded() {
        for model in MODELS.iter().filter(|model| !model.supported) {
            assert!(model.source.is_none(), "{} offers a download", model.id);
            assert!(
                model.unsupported_reason.is_some(),
                "{} gives no reason",
                model.id
            );
        }
    }

    #[test]
    fn supported_models_come_with_weights_and_an_engine() {
        for model in MODELS.iter().filter(|model| model.supported) {
            assert!(model.source.is_some(), "{} cannot be installed", model.id);
            assert!(!model.required.is_empty(), "{} needs no files", model.id);
            assert!(
                model.unsupported_reason.is_none(),
                "{} is both runnable and refused",
                model.id
            );
            assert_ne!(model.engine, Engine::None, "{} has no engine", model.id);
        }
    }

    #[test]
    fn every_required_file_is_downloaded() {
        for model in MODELS {
            let Some(Source::Files(files)) = model.source else {
                continue;
            };
            for required in model.required {
                assert!(
                    files.iter().any(|(name, _, _)| name == required),
                    "{} needs {required} but never fetches it",
                    model.id
                );
            }
        }
    }

    #[test]
    fn whisper_models_point_at_one_ggml_file() {
        let mut seen = 0;
        for model in MODELS
            .iter()
            .filter(|model| model.engine == Engine::Whisper)
        {
            seen += 1;
            assert!(model.supported, "{} cannot run on this build", model.id);
            let Some(Source::Files(files)) = model.source else {
                panic!("{} has no file list", model.id);
            };
            assert_eq!(files.len(), 1, "{} fetches more than one weight", model.id);
            let (name, url, bytes) = files[0];
            assert_eq!(
                model.required,
                &[name],
                "{} requires another name",
                model.id
            );
            assert!(
                name.starts_with("ggml-") && name.ends_with(".bin"),
                "{} is not a GGML file: {name}",
                model.id
            );
            assert_eq!(model.bytes, bytes, "{} card size drifted", model.id);
            assert!(url.starts_with("https://"), "{}: {url}", model.id);
        }
        assert_eq!(seen, 12, "the Whisper family lost an entry");
    }

    #[test]
    fn models_without_an_engine_stay_unsupported() {
        for model in MODELS.iter().filter(|model| model.engine == Engine::None) {
            assert!(
                !model.supported,
                "{} claims to run without an engine",
                model.id
            );
            assert!(
                model.unsupported_reason.is_some(),
                "{} gives no reason",
                model.id
            );
        }
    }

    #[test]
    fn ids_are_unique() {
        let mut seen = std::collections::HashSet::new();
        for model in MODELS {
            assert!(seen.insert(model.id), "{} appears twice", model.id);
        }
    }
}
