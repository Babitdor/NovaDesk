//! The on-device voice catalogue: what Nova can read aloud, and where each
//! download comes from.
//!
//! Three rules, the same ones the dictation catalogue follows:
//!
//! - Every number is published, not estimated. The byte counts are the sums of
//!   the two files each voice needs, read from `voices.json` in
//!   rhasspy/piper-voices (and the one voice that was actually downloaded
//!   matched its entry to the byte). Sample rates and speaker counts are read
//!   from each voice's own MODEL_CARD. The runtime bundle size is the
//!   `Content-Length` of Piper's 2023.11.14-2 release asset.
//! - `gender` is only set where a model card states it. Not one of these
//!   nineteen cards does, so all of them are None rather than guessed from a
//!   name or a sample rate.
//! - A voice with no runtime on this host is listed, not hidden: supported:
//!   false plus the reason the card shows. That is a compile-time property
//!   here (Piper publishes a Windows binary, which is the one Nova installs),
//!   so the UI is honest on Linux and macOS instead of failing at speak time.

use serde::Serialize;

/// Where Piper comes from. One asset, unpacked beside the voices.
///
/// The binary is Piper's own release zip, downloaded at runtime. That is a
/// deliberate licensing choice as much as an engineering one: espeak-ng is
/// GPLv3, and shipping it inside `novacode-desktop.exe` would make this
/// executable GPLv3. Downloaded beside it, it stays a separate program that
/// Nova runs, which is what Piper's release ships and what its licence expects.
const RUNTIME_URL: &str =
    "https://github.com/rhasspy/piper/releases/download/2023.11.14-2/piper_windows_amd64.zip";
/// The asset's Content-Length, checked after the transfer like every voice.
const RUNTIME_BYTES: u64 = 22_477_236;

/// Which engine speaks, and therefore where the CPU work happens.
///
/// Piper is a prebuilt binary, so there is no crate, no bindgen and no CMake on
/// the build machine, and espeak-ng stays outside this executable. Kale is
/// plain ONNX weights, run here on ort, which this build already links for
/// dictation: no second download, but the phonemes have to be produced in
/// process (frontend.rs) because the graph does not read letters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TtsEngine {
    Piper,
    Kale,
    Supertonic,
    Kokoro,
}

impl TtsEngine {
    /// Whether this engine's graph takes phonemes, and therefore needs the
    /// shared front end in `speak/frontend/`. Kale ships its own copy, so it
    /// answers no; a second phoneme voice would answer yes and share it.
    pub fn needs_frontend(self) -> bool {
        matches!(self, TtsEngine::Kokoro)
    }

    /// The engine's name in the install manifest, so a voice installed by one
    /// build is recognisable by the next.
    pub fn name(self) -> &'static str {
        match self {
            TtsEngine::Piper => "piper",
            TtsEngine::Kale => "kale",
            TtsEngine::Supertonic => "supertonic",
            TtsEngine::Kokoro => "kokoro",
        }
    }
}

/// A card badge. A fixed vocabulary, so the UI never renders a stray string
/// and the stylesheet can target each one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum TtsBadge {
    Recommended,
    LowMemory,
    MultiSpeaker,
    HighestQuality,
}

/// Where the files come from. Files are fetched one at a time straight into
/// the voice folder; a Zip is streamed to one archive and unpacked, which is
/// how the Piper runtime arrives. Both carry the exact byte count so a
/// truncated transfer is caught instead of installed.
#[derive(Debug, Clone, Copy)]
pub enum Source {
    /// (file name on disk, absolute URL, bytes)
    Files(&'static [(&'static str, &'static str, u64)]),
    /// (absolute URL, bytes)
    Zip(&'static str, u64),
}

#[derive(Debug, Clone, Copy)]
pub struct Voice {
    pub id: &'static str,
    pub name: &'static str,
    pub engine: TtsEngine,
    /// ISO code the voice is trained for, exactly as Piper writes it.
    pub language: &'static str,
    pub language_name: &'static str,
    /// Only where the model card states it; see the module header.
    pub gender: Option<&'static str>,
    pub bytes: u64,
    pub sample_rate: u32,
    /// Speakers the weights carry. 1 means `-s` is never passed.
    pub speakers: u32,
    pub licence: &'static str,
    pub description: &'static str,
    pub badges: &'static [TtsBadge],
    pub supported: bool,
    pub unsupported_reason: Option<&'static str>,
    pub source: Option<Source>,
    /// Files the engine needs on disk. Doubles as the integrity check after a
    /// download and the "is this voice really installed" test before speaking.
    pub required: &'static [&'static str],
}

/// Piper publishes binaries for Windows only, in the one asset Nova downloads.
/// `supported` is therefore a property of the build, not of the voice.
const RUNTIME_HOST: Option<&str> = if cfg!(windows) {
    None
} else {
    Some("Piper ships a prebuilt Windows binary; this build runs on another platform")
};

/// en_US-amy-low: 63_104_526 + 4_164 bytes, from voices.json.
const EN_US_AMY_LOW_FILES: &[(&str, &str, u64)] = &[
    (
        "en_US-amy-low.onnx",
        "https://huggingface.co/rhasspy/piper-voices/resolve/main/en/en_US/amy/low/en_US-amy-low.onnx",
        63_104_526,
    ),
    (
        "en_US-amy-low.onnx.json",
        "https://huggingface.co/rhasspy/piper-voices/resolve/main/en/en_US/amy/low/en_US-amy-low.onnx.json",
        4_164,
    ),
];
const EN_US_AMY_LOW_REQUIRED: &[&str] = &["en_US-amy-low.onnx", "en_US-amy-low.onnx.json"];

/// en_US-lessac-medium: 63_201_294 + 4_885 bytes, from voices.json.
const EN_US_LESSAC_MEDIUM_FILES: &[(&str, &str, u64)] = &[
    (
        "en_US-lessac-medium.onnx",
        "https://huggingface.co/rhasspy/piper-voices/resolve/main/en/en_US/lessac/medium/en_US-lessac-medium.onnx",
        63_201_294,
    ),
    (
        "en_US-lessac-medium.onnx.json",
        "https://huggingface.co/rhasspy/piper-voices/resolve/main/en/en_US/lessac/medium/en_US-lessac-medium.onnx.json",
        4_885,
    ),
];
const EN_US_LESSAC_MEDIUM_REQUIRED: &[&str] =
    &["en_US-lessac-medium.onnx", "en_US-lessac-medium.onnx.json"];

/// en_US-lessac-high: 113_895_201 + 4_883 bytes, from voices.json.
const EN_US_LESSAC_HIGH_FILES: &[(&str, &str, u64)] = &[
    (
        "en_US-lessac-high.onnx",
        "https://huggingface.co/rhasspy/piper-voices/resolve/main/en/en_US/lessac/high/en_US-lessac-high.onnx",
        113_895_201,
    ),
    (
        "en_US-lessac-high.onnx.json",
        "https://huggingface.co/rhasspy/piper-voices/resolve/main/en/en_US/lessac/high/en_US-lessac-high.onnx.json",
        4_883,
    ),
];
const EN_US_LESSAC_HIGH_REQUIRED: &[&str] =
    &["en_US-lessac-high.onnx", "en_US-lessac-high.onnx.json"];

/// en_US-ryan-high: 120_786_792 + 4_166 bytes, from voices.json.
const EN_US_RYAN_HIGH_FILES: &[(&str, &str, u64)] = &[
    (
        "en_US-ryan-high.onnx",
        "https://huggingface.co/rhasspy/piper-voices/resolve/main/en/en_US/ryan/high/en_US-ryan-high.onnx",
        120_786_792,
    ),
    (
        "en_US-ryan-high.onnx.json",
        "https://huggingface.co/rhasspy/piper-voices/resolve/main/en/en_US/ryan/high/en_US-ryan-high.onnx.json",
        4_166,
    ),
];
const EN_US_RYAN_HIGH_REQUIRED: &[&str] = &["en_US-ryan-high.onnx", "en_US-ryan-high.onnx.json"];

/// en_GB-alba-medium: 63_221_984 + 6_295 bytes, from voices.json.
const EN_GB_ALBA_MEDIUM_FILES: &[(&str, &str, u64)] = &[
    (
        "en_GB-alba-medium.onnx",
        "https://huggingface.co/rhasspy/piper-voices/resolve/main/en/en_GB/alba/medium/en_GB-alba-medium.onnx",
        63_221_984,
    ),
    (
        "en_GB-alba-medium.onnx.json",
        "https://huggingface.co/rhasspy/piper-voices/resolve/main/en/en_GB/alba/medium/en_GB-alba-medium.onnx.json",
        6_295,
    ),
];
const EN_GB_ALBA_MEDIUM_REQUIRED: &[&str] =
    &["en_GB-alba-medium.onnx", "en_GB-alba-medium.onnx.json"];

/// en_GB-northern_english_male-medium: 63_221_984 + 6_267 bytes, from voices.json.
const EN_GB_NORTHERN_ENGLISH_MALE_MEDIUM_FILES: &[(&str, &str, u64)] = &[
    (
        "en_GB-northern_english_male-medium.onnx",
        "https://huggingface.co/rhasspy/piper-voices/resolve/main/en/en_GB/northern_english_male/medium/en_GB-northern_english_male-medium.onnx",
        63_221_984,
    ),
    (
        "en_GB-northern_english_male-medium.onnx.json",
        "https://huggingface.co/rhasspy/piper-voices/resolve/main/en/en_GB/northern_english_male/medium/en_GB-northern_english_male-medium.onnx.json",
        6_267,
    ),
];
const EN_GB_NORTHERN_ENGLISH_MALE_MEDIUM_REQUIRED: &[&str] = &[
    "en_GB-northern_english_male-medium.onnx",
    "en_GB-northern_english_male-medium.onnx.json",
];

/// en_US-libritts_r-medium: 78_580_914 + 20_123 bytes, from voices.json.
const EN_US_LIBRITTS_R_MEDIUM_FILES: &[(&str, &str, u64)] = &[
    (
        "en_US-libritts_r-medium.onnx",
        "https://huggingface.co/rhasspy/piper-voices/resolve/main/en/en_US/libritts_r/medium/en_US-libritts_r-medium.onnx",
        78_580_914,
    ),
    (
        "en_US-libritts_r-medium.onnx.json",
        "https://huggingface.co/rhasspy/piper-voices/resolve/main/en/en_US/libritts_r/medium/en_US-libritts_r-medium.onnx.json",
        20_123,
    ),
];
const EN_US_LIBRITTS_R_MEDIUM_REQUIRED: &[&str] = &[
    "en_US-libritts_r-medium.onnx",
    "en_US-libritts_r-medium.onnx.json",
];

/// de_DE-thorsten-high: 113_895_201 + 4_875 bytes, from voices.json.
const DE_DE_THORSTEN_HIGH_FILES: &[(&str, &str, u64)] = &[
    (
        "de_DE-thorsten-high.onnx",
        "https://huggingface.co/rhasspy/piper-voices/resolve/main/de/de_DE/thorsten/high/de_DE-thorsten-high.onnx",
        113_895_201,
    ),
    (
        "de_DE-thorsten-high.onnx.json",
        "https://huggingface.co/rhasspy/piper-voices/resolve/main/de/de_DE/thorsten/high/de_DE-thorsten-high.onnx.json",
        4_875,
    ),
];
const DE_DE_THORSTEN_HIGH_REQUIRED: &[&str] =
    &["de_DE-thorsten-high.onnx", "de_DE-thorsten-high.onnx.json"];

/// fr_FR-siwis-medium: 63_221_984 + 6_369 bytes, from voices.json.
const FR_FR_SIWIS_MEDIUM_FILES: &[(&str, &str, u64)] = &[
    (
        "fr_FR-siwis-medium.onnx",
        "https://huggingface.co/rhasspy/piper-voices/resolve/main/fr/fr_FR/siwis/medium/fr_FR-siwis-medium.onnx",
        63_221_984,
    ),
    (
        "fr_FR-siwis-medium.onnx.json",
        "https://huggingface.co/rhasspy/piper-voices/resolve/main/fr/fr_FR/siwis/medium/fr_FR-siwis-medium.onnx.json",
        6_369,
    ),
];
const FR_FR_SIWIS_MEDIUM_REQUIRED: &[&str] =
    &["fr_FR-siwis-medium.onnx", "fr_FR-siwis-medium.onnx.json"];

/// es_ES-davefx-medium: 63_221_984 + 6_175 bytes, from voices.json.
const ES_ES_DAVEFX_MEDIUM_FILES: &[(&str, &str, u64)] = &[
    (
        "es_ES-davefx-medium.onnx",
        "https://huggingface.co/rhasspy/piper-voices/resolve/main/es/es_ES/davefx/medium/es_ES-davefx-medium.onnx",
        63_221_984,
    ),
    (
        "es_ES-davefx-medium.onnx.json",
        "https://huggingface.co/rhasspy/piper-voices/resolve/main/es/es_ES/davefx/medium/es_ES-davefx-medium.onnx.json",
        6_175,
    ),
];
const ES_ES_DAVEFX_MEDIUM_REQUIRED: &[&str] =
    &["es_ES-davefx-medium.onnx", "es_ES-davefx-medium.onnx.json"];

/// it_IT-paola-medium: 63_221_984 + 6_114 bytes, from voices.json.
const IT_IT_PAOLA_MEDIUM_FILES: &[(&str, &str, u64)] = &[
    (
        "it_IT-paola-medium.onnx",
        "https://huggingface.co/rhasspy/piper-voices/resolve/main/it/it_IT/paola/medium/it_IT-paola-medium.onnx",
        63_221_984,
    ),
    (
        "it_IT-paola-medium.onnx.json",
        "https://huggingface.co/rhasspy/piper-voices/resolve/main/it/it_IT/paola/medium/it_IT-paola-medium.onnx.json",
        6_114,
    ),
];
const IT_IT_PAOLA_MEDIUM_REQUIRED: &[&str] =
    &["it_IT-paola-medium.onnx", "it_IT-paola-medium.onnx.json"];

/// pt_BR-faber-medium: 63_221_984 + 6_257 bytes, from voices.json.
const PT_BR_FABER_MEDIUM_FILES: &[(&str, &str, u64)] = &[
    (
        "pt_BR-faber-medium.onnx",
        "https://huggingface.co/rhasspy/piper-voices/resolve/main/pt/pt_BR/faber/medium/pt_BR-faber-medium.onnx",
        63_221_984,
    ),
    (
        "pt_BR-faber-medium.onnx.json",
        "https://huggingface.co/rhasspy/piper-voices/resolve/main/pt/pt_BR/faber/medium/pt_BR-faber-medium.onnx.json",
        6_257,
    ),
];
const PT_BR_FABER_MEDIUM_REQUIRED: &[&str] =
    &["pt_BR-faber-medium.onnx", "pt_BR-faber-medium.onnx.json"];

/// ru_RU-irina-medium: 63_201_294 + 4_765 bytes, from voices.json.
const RU_RU_IRINA_MEDIUM_FILES: &[(&str, &str, u64)] = &[
    (
        "ru_RU-irina-medium.onnx",
        "https://huggingface.co/rhasspy/piper-voices/resolve/main/ru/ru_RU/irina/medium/ru_RU-irina-medium.onnx",
        63_201_294,
    ),
    (
        "ru_RU-irina-medium.onnx.json",
        "https://huggingface.co/rhasspy/piper-voices/resolve/main/ru/ru_RU/irina/medium/ru_RU-irina-medium.onnx.json",
        4_765,
    ),
];
const RU_RU_IRINA_MEDIUM_REQUIRED: &[&str] =
    &["ru_RU-irina-medium.onnx", "ru_RU-irina-medium.onnx.json"];

/// pl_PL-darkman-medium: 63_221_984 + 6_133 bytes, from voices.json.
const PL_PL_DARKMAN_MEDIUM_FILES: &[(&str, &str, u64)] = &[
    (
        "pl_PL-darkman-medium.onnx",
        "https://huggingface.co/rhasspy/piper-voices/resolve/main/pl/pl_PL/darkman/medium/pl_PL-darkman-medium.onnx",
        63_221_984,
    ),
    (
        "pl_PL-darkman-medium.onnx.json",
        "https://huggingface.co/rhasspy/piper-voices/resolve/main/pl/pl_PL/darkman/medium/pl_PL-darkman-medium.onnx.json",
        6_133,
    ),
];
const PL_PL_DARKMAN_MEDIUM_REQUIRED: &[&str] = &[
    "pl_PL-darkman-medium.onnx",
    "pl_PL-darkman-medium.onnx.json",
];

/// nl_NL-mls-medium: 76_584_246 + 5_856 bytes, from voices.json.
const NL_NL_MLS_MEDIUM_FILES: &[(&str, &str, u64)] = &[
    (
        "nl_NL-mls-medium.onnx",
        "https://huggingface.co/rhasspy/piper-voices/resolve/main/nl/nl_NL/mls/medium/nl_NL-mls-medium.onnx",
        76_584_246,
    ),
    (
        "nl_NL-mls-medium.onnx.json",
        "https://huggingface.co/rhasspy/piper-voices/resolve/main/nl/nl_NL/mls/medium/nl_NL-mls-medium.onnx.json",
        5_856,
    ),
];
const NL_NL_MLS_MEDIUM_REQUIRED: &[&str] = &["nl_NL-mls-medium.onnx", "nl_NL-mls-medium.onnx.json"];

/// tr_TR-dfki-medium: 63_201_294 + 4_960 bytes, from voices.json.
const TR_TR_DFKI_MEDIUM_FILES: &[(&str, &str, u64)] = &[
    (
        "tr_TR-dfki-medium.onnx",
        "https://huggingface.co/rhasspy/piper-voices/resolve/main/tr/tr_TR/dfki/medium/tr_TR-dfki-medium.onnx",
        63_201_294,
    ),
    (
        "tr_TR-dfki-medium.onnx.json",
        "https://huggingface.co/rhasspy/piper-voices/resolve/main/tr/tr_TR/dfki/medium/tr_TR-dfki-medium.onnx.json",
        4_960,
    ),
];
const TR_TR_DFKI_MEDIUM_REQUIRED: &[&str] =
    &["tr_TR-dfki-medium.onnx", "tr_TR-dfki-medium.onnx.json"];

/// zh_CN-huayan-medium: 63_201_294 + 4_822 bytes, from voices.json.
const ZH_CN_HUAYAN_MEDIUM_FILES: &[(&str, &str, u64)] = &[
    (
        "zh_CN-huayan-medium.onnx",
        "https://huggingface.co/rhasspy/piper-voices/resolve/main/zh/zh_CN/huayan/medium/zh_CN-huayan-medium.onnx",
        63_201_294,
    ),
    (
        "zh_CN-huayan-medium.onnx.json",
        "https://huggingface.co/rhasspy/piper-voices/resolve/main/zh/zh_CN/huayan/medium/zh_CN-huayan-medium.onnx.json",
        4_822,
    ),
];
const ZH_CN_HUAYAN_MEDIUM_REQUIRED: &[&str] =
    &["zh_CN-huayan-medium.onnx", "zh_CN-huayan-medium.onnx.json"];

/// ar_JO-kareem-low: 63_201_294 + 5_022 bytes, from voices.json.
const AR_JO_KAREEM_LOW_FILES: &[(&str, &str, u64)] = &[
    (
        "ar_JO-kareem-low.onnx",
        "https://huggingface.co/rhasspy/piper-voices/resolve/main/ar/ar_JO/kareem/low/ar_JO-kareem-low.onnx",
        63_201_294,
    ),
    (
        "ar_JO-kareem-low.onnx.json",
        "https://huggingface.co/rhasspy/piper-voices/resolve/main/ar/ar_JO/kareem/low/ar_JO-kareem-low.onnx.json",
        5_022,
    ),
];
const AR_JO_KAREEM_LOW_REQUIRED: &[&str] = &["ar_JO-kareem-low.onnx", "ar_JO-kareem-low.onnx.json"];

/// hi_IN-pratham-medium: 63_516_050 + 4_970 bytes, from voices.json.
const HI_IN_PRATHAM_MEDIUM_FILES: &[(&str, &str, u64)] = &[
    (
        "hi_IN-pratham-medium.onnx",
        "https://huggingface.co/rhasspy/piper-voices/resolve/main/hi/hi_IN/pratham/medium/hi_IN-pratham-medium.onnx",
        63_516_050,
    ),
    (
        "hi_IN-pratham-medium.onnx.json",
        "https://huggingface.co/rhasspy/piper-voices/resolve/main/hi/hi_IN/pratham/medium/hi_IN-pratham-medium.onnx.json",
        4_970,
    ),
];
const HI_IN_PRATHAM_MEDIUM_REQUIRED: &[&str] = &[
    "hi_IN-pratham-medium.onnx",
    "hi_IN-pratham-medium.onnx.json",
];

/// kale-tts-1-nano, from the repository's own tree: 14_767_138 + 1_789 + 113 +
/// 4_069 + 100 bytes of model files, then the front end at 2_786_691 + 76_579
/// (the word list and its overrides), 1_745_840 (the GRU) and 522_240 + 336
/// (one voice's style table and its per-voice inputs).
///
/// The front end is part of the engine, not of the model: the graph takes
/// Kokoro phoneme ids, so without a word list and a phonemiser there is nothing
/// to send it. Those files are downloaded flat, because the engine addresses
/// them by name and a nested folder would only be a second thing to get wrong.
const KALE_FILES: &[(&str, &str, u64)] = &[
    (
        "kale-tts-1-nano.onnx",
        "https://huggingface.co/KaleComputer/kale-tts-1-nano/resolve/main/kale-tts-1-nano.onnx",
        14_767_138,
    ),
    (
        "tokenizer.json",
        "https://huggingface.co/KaleComputer/kale-tts-1-nano/resolve/main/tokenizer.json",
        1_789,
    ),
    (
        "tokenizer_config.json",
        "https://huggingface.co/KaleComputer/kale-tts-1-nano/resolve/main/tokenizer_config.json",
        113,
    ),
    (
        "voices.json",
        "https://huggingface.co/KaleComputer/kale-tts-1-nano/resolve/main/voices.json",
        4_069,
    ),
    (
        "config.json",
        "https://huggingface.co/KaleComputer/kale-tts-1-nano/resolve/main/config.json",
        100,
    ),
    (
        "lexicon.tsv",
        "https://huggingface.co/KaleComputer/kale-tts-1-nano/resolve/main/frontend/words/lexicon.tsv",
        2_786_691,
    ),
    (
        "override.public.tsv",
        "https://huggingface.co/KaleComputer/kale-tts-1-nano/resolve/main/frontend/words/override.public.tsv",
        76_579,
    ),
    (
        "g2p.bin",
        "https://huggingface.co/KaleComputer/kale-tts-1-nano/resolve/main/frontend/words/g2p.bin",
        1_745_840,
    ),
    (
        "s057b.bin",
        "https://huggingface.co/KaleComputer/kale-tts-1-nano/resolve/main/voices/s057b.bin",
        522_240,
    ),
    (
        "s057b.extra.bin",
        "https://huggingface.co/KaleComputer/kale-tts-1-nano/resolve/main/voices/s057b.extra.bin",
        336,
    ),
];
const KALE_REQUIRED: &[&str] = &[
    "kale-tts-1-nano.onnx",
    "tokenizer.json",
    "tokenizer_config.json",
    "voices.json",
    "config.json",
    "lexicon.tsv",
    "override.public.tsv",
    "g2p.bin",
    "s057b.bin",
    "s057b.extra.bin",
];

/// Kokoro-82M: 92_361_116 for the int8 graph and 524_288 for one style table.
///
/// This is the smallest graph of the several the repository publishes (the fp32
/// one is 325_532_232, the fp16 163_234_740), which is why the card is this one:
/// a quantised graph that reads the same phoneme ids beats a bigger graph that
/// would cost the user 230 MB more for a voice they can already hear.
const KOKORO_FILES: &[(&str, &str, u64)] = &[
    (
        "model_quantized.onnx",
        "https://huggingface.co/onnx-community/Kokoro-82M-v1.0-ONNX/resolve/main/onnx/model_quantized.onnx",
        92_361_116,
    ),
    (
        "af.bin",
        "https://huggingface.co/onnx-community/Kokoro-82M-v1.0-ONNX/resolve/main/voices/af.bin",
        524_288,
    ),
];
const KOKORO_REQUIRED: &[&str] = &["model_quantized.onnx", "af.bin"];

/// The shared phoneme front end, installed once into `speak/frontend/` for every
/// voice whose graph takes Kokoro phoneme ids rather than letters: the word
/// list, its overrides, the GRU for words the list has not got, and the id
/// table. Kale ships its own copy inside its folder (a 20 MB voice that arrives
/// complete); Kokoro reuses this one, so two voices needing the same 4.5 MB is
/// one download rather than two.
pub struct FrontEnd {
    pub bytes: u64,
    pub files: &'static [(&'static str, &'static str, u64)],
    /// The same four names, for the installer's integrity check.
    pub required: &'static [&'static str],
}

pub const FRONTEND: FrontEnd = FrontEnd {
    bytes: 4_610_899,
    required: &["lexicon.tsv", "override.public.tsv", "g2p.bin", "tokenizer.json"],
    files: &[
        (
            "lexicon.tsv",
            "https://huggingface.co/KaleComputer/kale-tts-1-nano/resolve/main/frontend/words/lexicon.tsv",
            2_786_691,
        ),
        (
            "override.public.tsv",
            "https://huggingface.co/KaleComputer/kale-tts-1-nano/resolve/main/frontend/words/override.public.tsv",
            76_579,
        ),
        (
            "g2p.bin",
            "https://huggingface.co/KaleComputer/kale-tts-1-nano/resolve/main/frontend/words/g2p.bin",
            1_745_840,
        ),
        (
            "tokenizer.json",
            "https://huggingface.co/KaleComputer/kale-tts-1-nano/resolve/main/tokenizer.json",
            1_789,
        ),
    ],
};

/// supertonic-3: 3_700_147 (duration) + 36_416_150 (text encoder) +
/// 256_534_781 (vector estimator) + 101_424_195 (vocoder) + 8_253 (tts.json) +
/// 277_676 (unicode indexer) + 291_748 (one voice style), from the repository's
/// own file tree.
///
/// 398 MB is the reason this voice is not the default: it is a flow-matching
/// model, so the vector estimator alone is bigger than every Piper voice here
/// put together. The style is one file of ten, because each is a separate
/// 290 KB download and the card must not promise voices it does not fetch.
const SUPER_FILES: &[(&str, &str, u64)] = &[
    (
        "duration_predictor.onnx",
        "https://huggingface.co/Supertone/supertonic-3/resolve/main/onnx/duration_predictor.onnx",
        3_700_147,
    ),
    (
        "text_encoder.onnx",
        "https://huggingface.co/Supertone/supertonic-3/resolve/main/onnx/text_encoder.onnx",
        36_416_150,
    ),
    (
        "vector_estimator.onnx",
        "https://huggingface.co/Supertone/supertonic-3/resolve/main/onnx/vector_estimator.onnx",
        256_534_781,
    ),
    (
        "vocoder.onnx",
        "https://huggingface.co/Supertone/supertonic-3/resolve/main/onnx/vocoder.onnx",
        101_424_195,
    ),
    (
        "tts.json",
        "https://huggingface.co/Supertone/supertonic-3/resolve/main/onnx/tts.json",
        8_253,
    ),
    (
        "unicode_indexer.json",
        "https://huggingface.co/Supertone/supertonic-3/resolve/main/onnx/unicode_indexer.json",
        277_676,
    ),
    (
        "M1.json",
        "https://huggingface.co/Supertone/supertonic-3/resolve/main/voice_styles/M1.json",
        291_748,
    ),
];
const SUPER_REQUIRED: &[&str] = &[
    "duration_predictor.onnx",
    "text_encoder.onnx",
    "vector_estimator.onnx",
    "vocoder.onnx",
    "tts.json",
    "unicode_indexer.json",
    "M1.json",
];

/// Piper's own binary and the libraries it loads, unpacked under `piper/`.
/// The file list is the integrity check: a half-unpacked archive fails it.
pub struct Runtime {
    pub bytes: u64,
    pub source: Source,
    pub files: &'static [&'static str],
    /// Directories, checked as directories because they hold hundreds of files.
    pub dirs: &'static [&'static str],
}

pub const RUNTIME: Runtime = Runtime {
    bytes: RUNTIME_BYTES,
    source: Source::Zip(RUNTIME_URL, RUNTIME_BYTES),
    files: &[
        "piper.exe",
        "espeak-ng.dll",
        "piper_phonemize.dll",
        "onnxruntime.dll",
        "onnxruntime_providers_shared.dll",
        "libtashkeel_model.ort",
    ],
    dirs: &["espeak-ng-data"],
};

/// True when this build can run what it downloads.
const RUNTIME_READY: bool = cfg!(windows);

pub const VOICES: &[Voice] = &[
    Voice {
        id: "en_US-amy-low",
        name: "Amy",
        engine: TtsEngine::Piper,
        language: "en_US",
        language_name: "English (United States)",
        gender: None,
        bytes: 63_108_690,
        sample_rate: 16_000,
        speakers: 1,
        licence: "See model card",
        description: "A calm American voice and the smallest English download here; clear on short replies, thinner on long ones.",
        badges: &[TtsBadge::LowMemory],
        supported: RUNTIME_READY,
        unsupported_reason: RUNTIME_HOST,
        source: Some(Source::Files(EN_US_AMY_LOW_FILES)),
        required: EN_US_AMY_LOW_REQUIRED,
    },
    Voice {
        id: "en_US-lessac-medium",
        name: "Lessac",
        engine: TtsEngine::Piper,
        language: "en_US",
        language_name: "English (United States)",
        gender: None,
        bytes: 63_206_179,
        sample_rate: 22_050,
        speakers: 1,
        licence: "Blizzard 2013",
        description: "The reference American voice Piper is tuned around: steady, neutral, and the one most guides point at first.",
        badges: &[TtsBadge::Recommended],
        supported: RUNTIME_READY,
        unsupported_reason: RUNTIME_HOST,
        source: Some(Source::Files(EN_US_LESSAC_MEDIUM_FILES)),
        required: EN_US_LESSAC_MEDIUM_REQUIRED,
    },
    Voice {
        id: "en_US-lessac-high",
        name: "Lessac High",
        engine: TtsEngine::Piper,
        language: "en_US",
        language_name: "English (United States)",
        gender: None,
        bytes: 113_900_084,
        sample_rate: 22_050,
        speakers: 1,
        licence: "Blizzard 2013",
        description: "The same speaker trained longer at high quality; near the top of what this engine produces.",
        badges: &[TtsBadge::HighestQuality],
        supported: RUNTIME_READY,
        unsupported_reason: RUNTIME_HOST,
        source: Some(Source::Files(EN_US_LESSAC_HIGH_FILES)),
        required: EN_US_LESSAC_HIGH_REQUIRED,
    },
    Voice {
        id: "en_US-ryan-high",
        name: "Ryan",
        engine: TtsEngine::Piper,
        language: "en_US",
        language_name: "English (United States)",
        gender: None,
        bytes: 120_790_958,
        sample_rate: 22_050,
        speakers: 1,
        licence: "CC BY-NC-SA 4.0",
        description: "A brighter, faster American read at high quality. The dataset licence forbids commercial use.",
        badges: &[TtsBadge::HighestQuality],
        supported: RUNTIME_READY,
        unsupported_reason: RUNTIME_HOST,
        source: Some(Source::Files(EN_US_RYAN_HIGH_FILES)),
        required: EN_US_RYAN_HIGH_REQUIRED,
    },
    Voice {
        id: "en_GB-alba-medium",
        name: "Alba",
        engine: TtsEngine::Piper,
        language: "en_GB",
        language_name: "English (United Kingdom)",
        gender: None,
        bytes: 63_228_279,
        sample_rate: 22_050,
        speakers: 1,
        licence: "CC BY 4.0",
        description: "A Scottish English voice read from the Edinburgh speech corpus.",
        badges: &[],
        supported: RUNTIME_READY,
        unsupported_reason: RUNTIME_HOST,
        source: Some(Source::Files(EN_GB_ALBA_MEDIUM_FILES)),
        required: EN_GB_ALBA_MEDIUM_REQUIRED,
    },
    Voice {
        id: "en_GB-northern_english_male-medium",
        name: "Northern English Male",
        engine: TtsEngine::Piper,
        language: "en_GB",
        language_name: "English (United Kingdom)",
        gender: None,
        bytes: 63_228_251,
        sample_rate: 22_050,
        speakers: 1,
        licence: "CC BY-SA 4.0",
        description: "A male voice from the northern English open speech collection.",
        badges: &[],
        supported: RUNTIME_READY,
        unsupported_reason: RUNTIME_HOST,
        source: Some(Source::Files(EN_GB_NORTHERN_ENGLISH_MALE_MEDIUM_FILES)),
        required: EN_GB_NORTHERN_ENGLISH_MALE_MEDIUM_REQUIRED,
    },
    Voice {
        id: "en_US-libritts_r-medium",
        name: "LibriTTS-R",
        engine: TtsEngine::Piper,
        language: "en_US",
        language_name: "English (United States)",
        gender: None,
        bytes: 78_601_037,
        sample_rate: 22_050,
        speakers: 904,
        licence: "CC BY 4.0",
        description: "904 American readers in one download; pick a speaker number and keep it, or every reply changes voice.",
        badges: &[TtsBadge::MultiSpeaker],
        supported: RUNTIME_READY,
        unsupported_reason: RUNTIME_HOST,
        source: Some(Source::Files(EN_US_LIBRITTS_R_MEDIUM_FILES)),
        required: EN_US_LIBRITTS_R_MEDIUM_REQUIRED,
    },
    Voice {
        id: "de_DE-thorsten-high",
        name: "Thorsten",
        engine: TtsEngine::Piper,
        language: "de_DE",
        language_name: "German (Germany)",
        gender: None,
        bytes: 113_900_076,
        sample_rate: 22_050,
        speakers: 1,
        licence: "CC0",
        description: "The Thorsten dataset at high quality: the cleanest German voice in this list.",
        badges: &[TtsBadge::HighestQuality],
        supported: RUNTIME_READY,
        unsupported_reason: RUNTIME_HOST,
        source: Some(Source::Files(DE_DE_THORSTEN_HIGH_FILES)),
        required: DE_DE_THORSTEN_HIGH_REQUIRED,
    },
    Voice {
        id: "fr_FR-siwis-medium",
        name: "Siwis",
        engine: TtsEngine::Piper,
        language: "fr_FR",
        language_name: "French (France)",
        gender: None,
        bytes: 63_228_353,
        sample_rate: 22_050,
        speakers: 1,
        licence: "CC BY 4.0",
        description: "A French voice built from a read speech corpus, level and unhurried.",
        badges: &[],
        supported: RUNTIME_READY,
        unsupported_reason: RUNTIME_HOST,
        source: Some(Source::Files(FR_FR_SIWIS_MEDIUM_FILES)),
        required: FR_FR_SIWIS_MEDIUM_REQUIRED,
    },
    Voice {
        id: "es_ES-davefx-medium",
        name: "DaveFX",
        engine: TtsEngine::Piper,
        language: "es_ES",
        language_name: "Spanish (Spain)",
        gender: None,
        bytes: 63_228_159,
        sample_rate: 22_050,
        speakers: 1,
        licence: "CC0",
        description: "Castilian Spanish, medium quality, trained on the DaveFX open corpus.",
        badges: &[],
        supported: RUNTIME_READY,
        unsupported_reason: RUNTIME_HOST,
        source: Some(Source::Files(ES_ES_DAVEFX_MEDIUM_FILES)),
        required: ES_ES_DAVEFX_MEDIUM_REQUIRED,
    },
    Voice {
        id: "it_IT-paola-medium",
        name: "Paola",
        engine: TtsEngine::Piper,
        language: "it_IT",
        language_name: "Italian (Italy)",
        gender: None,
        bytes: 63_228_098,
        sample_rate: 22_050,
        speakers: 1,
        licence: "CC0",
        description: "An Italian voice fine-tuned from a public read corpus.",
        badges: &[],
        supported: RUNTIME_READY,
        unsupported_reason: RUNTIME_HOST,
        source: Some(Source::Files(IT_IT_PAOLA_MEDIUM_FILES)),
        required: IT_IT_PAOLA_MEDIUM_REQUIRED,
    },
    Voice {
        id: "pt_BR-faber-medium",
        name: "Faber",
        engine: TtsEngine::Piper,
        language: "pt_BR",
        language_name: "Portuguese (Brazil)",
        gender: None,
        bytes: 63_228_241,
        sample_rate: 22_050,
        speakers: 1,
        licence: "CC0",
        description: "Brazilian Portuguese; the Portuguese voice to start with.",
        badges: &[],
        supported: RUNTIME_READY,
        unsupported_reason: RUNTIME_HOST,
        source: Some(Source::Files(PT_BR_FABER_MEDIUM_FILES)),
        required: PT_BR_FABER_MEDIUM_REQUIRED,
    },
    Voice {
        id: "ru_RU-irina-medium",
        name: "Irina",
        engine: TtsEngine::Piper,
        language: "ru_RU",
        language_name: "Russian (Russia)",
        gender: None,
        bytes: 63_206_059,
        sample_rate: 22_050,
        speakers: 1,
        licence: "Unknown",
        description: "A Russian voice built from the RHVoice recordings; the dataset publishes no licence.",
        badges: &[],
        supported: RUNTIME_READY,
        unsupported_reason: RUNTIME_HOST,
        source: Some(Source::Files(RU_RU_IRINA_MEDIUM_FILES)),
        required: RU_RU_IRINA_MEDIUM_REQUIRED,
    },
    Voice {
        id: "pl_PL-darkman-medium",
        name: "Darkman",
        engine: TtsEngine::Piper,
        language: "pl_PL",
        language_name: "Polish (Poland)",
        gender: None,
        bytes: 63_228_117,
        sample_rate: 22_050,
        speakers: 1,
        licence: "CC0",
        description: "A Polish male voice, medium quality, from public reads.",
        badges: &[],
        supported: RUNTIME_READY,
        unsupported_reason: RUNTIME_HOST,
        source: Some(Source::Files(PL_PL_DARKMAN_MEDIUM_FILES)),
        required: PL_PL_DARKMAN_MEDIUM_REQUIRED,
    },
    Voice {
        id: "nl_NL-mls-medium",
        name: "MLS",
        engine: TtsEngine::Piper,
        language: "nl_NL",
        language_name: "Dutch (Netherlands)",
        gender: None,
        bytes: 76_590_102,
        sample_rate: 22_050,
        speakers: 52,
        licence: "CC BY 4.0",
        description: "52 Dutch readers from the Multilingual LibriSpeech recordings.",
        badges: &[TtsBadge::MultiSpeaker],
        supported: RUNTIME_READY,
        unsupported_reason: RUNTIME_HOST,
        source: Some(Source::Files(NL_NL_MLS_MEDIUM_FILES)),
        required: NL_NL_MLS_MEDIUM_REQUIRED,
    },
    Voice {
        id: "tr_TR-dfki-medium",
        name: "DFKI",
        engine: TtsEngine::Piper,
        language: "tr_TR",
        language_name: "Turkish (Turkey)",
        gender: None,
        bytes: 63_206_254,
        sample_rate: 22_050,
        speakers: 1,
        licence: "CC BY-NC-SA 4.0",
        description: "Turkish from the DFKI open speech data; the licence forbids commercial use.",
        badges: &[],
        supported: RUNTIME_READY,
        unsupported_reason: RUNTIME_HOST,
        source: Some(Source::Files(TR_TR_DFKI_MEDIUM_FILES)),
        required: TR_TR_DFKI_MEDIUM_REQUIRED,
    },
    Voice {
        id: "zh_CN-huayan-medium",
        name: "HuaYan",
        engine: TtsEngine::Piper,
        language: "zh_CN",
        language_name: "Chinese (China)",
        gender: None,
        bytes: 63_206_116,
        sample_rate: 22_050,
        speakers: 1,
        licence: "Unknown",
        description: "Mandarin from the HuaYan corpus; the dataset publishes no licence.",
        badges: &[],
        supported: RUNTIME_READY,
        unsupported_reason: RUNTIME_HOST,
        source: Some(Source::Files(ZH_CN_HUAYAN_MEDIUM_FILES)),
        required: ZH_CN_HUAYAN_MEDIUM_REQUIRED,
    },
    Voice {
        id: "ar_JO-kareem-low",
        name: "Kareem",
        engine: TtsEngine::Piper,
        language: "ar_JO",
        language_name: "Arabic (Jordan)",
        gender: None,
        bytes: 63_206_316,
        sample_rate: 16_000,
        speakers: 1,
        licence: "See model card",
        description: "Jordanian Arabic, low quality: intelligible, and the only Arabic voice Piper ships.",
        badges: &[],
        supported: RUNTIME_READY,
        unsupported_reason: RUNTIME_HOST,
        source: Some(Source::Files(AR_JO_KAREEM_LOW_FILES)),
        required: AR_JO_KAREEM_LOW_REQUIRED,
    },
    Voice {
        id: "hi_IN-pratham-medium",
        name: "Pratham",
        engine: TtsEngine::Piper,
        language: "hi_IN",
        language_name: "Hindi (India)",
        gender: None,
        bytes: 63_521_020,
        sample_rate: 22_050,
        speakers: 1,
        licence: "CC BY-NC-SA 4.0",
        description: "Hindi from the Indic NLP corpus; the licence forbids commercial use.",
        badges: &[],
        supported: RUNTIME_READY,
        unsupported_reason: RUNTIME_HOST,
        source: Some(Source::Files(HI_IN_PRATHAM_MEDIUM_FILES)),
        required: HI_IN_PRATHAM_MEDIUM_REQUIRED,
    },
    Voice {
        id: "kale-tts-1-nano",
        name: "Kale",
        engine: TtsEngine::Kale,
        language: "en_US",
        language_name: "English (United States)",
        gender: None,
        bytes: 19_904_895,
        sample_rate: 24_000,
        // voices.json lists ten presets, but a preset is 522 KB of style table
        // plus its per-voice inputs, and the download fetches the one this
        // voice is called for. Advertising ten and installing one would fail at
        // speak time for nine of them, so the card says one.
        speakers: 1,
        licence: "Apache-2.0",
        description: "The smallest download here, and the only one that runs without Piper: 15 MB of weights plus the 4.6 MB text front end that turns words into the phonemes they are spoken as. Ten American presets. Numbers and names are read plainly rather than cleverly, and the speed control does not reach this engine.",
        badges: &[TtsBadge::LowMemory],
        // Plain ONNX on ort, which every build already links for dictation, so
        // there is no runtime to install and no platform where this is refused.
        supported: true,
        unsupported_reason: None,
        source: Some(Source::Files(KALE_FILES)),
        required: KALE_REQUIRED,
    },
    Voice {
        id: "supertonic-3",
        name: "Supertonic",
        engine: TtsEngine::Supertonic,
        language: "en_US",
        language_name: "English (United States)",
        gender: None,
        bytes: 398_652_950,
        sample_rate: 44_100,
        // One style file is downloaded; the repository lists ten.
        speakers: 1,
        licence: "OpenRAIL-M",
        description: "A flow-matching voice that runs in this app: character input, no phoneme front end, 44.1 kHz, and the best of what is here on long replies. 398 MB is the whole cost, and the weights are OpenRAIL-M rather than permissive, so the use restrictions in that licence apply to what it says.",
        badges: &[TtsBadge::HighestQuality],
        supported: true,
        unsupported_reason: None,
        source: Some(Source::Files(SUPER_FILES)),
        required: SUPER_REQUIRED,
    },
    Voice {
        id: "kokoro-82m",
        name: "Kokoro",
        engine: TtsEngine::Kokoro,
        language: "en_US",
        language_name: "English (United States)",
        gender: None,
        bytes: 92_885_404,
        sample_rate: 24_000,
        // voices/af.bin is one style table; the repository lists dozens.
        speakers: 1,
        licence: "Apache-2.0",
        description: "The reference voice Kale is a finetune of, on its quantised graph: 92 MB and the shared 4.5 MB phoneme front end arrive once, and any other Kokoro voice reuses it. Steady and neutral, with the same plain reading of numbers and names.",
        badges: &[TtsBadge::Recommended],
        supported: true,
        unsupported_reason: None,
        source: Some(Source::Files(KOKORO_FILES)),
        required: KOKORO_REQUIRED,
    },
];

/// The catalogue as the UI receives it: borrowed fields become owned strings
/// so the frontend needs no copy of the table.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceView {
    pub id: String,
    pub name: String,
    pub engine: TtsEngine,
    pub language: LanguageView,
    pub gender: Option<String>,
    pub bytes: u64,
    pub sample_rate: u32,
    pub speakers: u32,
    pub licence: String,
    pub description: String,
    pub badges: Vec<TtsBadge>,
    pub supported: bool,
    pub unsupported_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LanguageView {
    pub id: &'static str,
    pub name: &'static str,
}

impl Voice {
    pub fn language_view(&self) -> LanguageView {
        LanguageView {
            id: self.language,
            name: self.language_name,
        }
    }

    /// The two files this voice needs, or nothing when it has no weights here.
    pub fn files(&self) -> &'static [(&'static str, &'static str, u64)] {
        match self.source {
            Some(Source::Files(files)) => files,
            _ => &[],
        }
    }

    pub fn view(&self) -> VoiceView {
        VoiceView {
            id: self.id.to_string(),
            name: self.name.to_string(),
            engine: self.engine,
            language: self.language_view(),
            gender: self.gender.map(str::to_string),
            bytes: self.bytes,
            sample_rate: self.sample_rate,
            speakers: self.speakers,
            licence: self.licence.to_string(),
            description: self.description.to_string(),
            badges: self.badges.to_vec(),
            supported: self.supported,
            unsupported_reason: self.unsupported_reason.map(str::to_string),
        }
    }
}

pub fn find(id: &str) -> Option<&'static Voice> {
    VOICES.iter().find(|voice| voice.id == id)
}

/// Every language the catalogue covers, in catalogue order, for the filter.
pub fn languages() -> Vec<LanguageView> {
    let mut seen: Vec<LanguageView> = Vec::new();
    for voice in VOICES {
        if !seen.iter().any(|entry| entry.id == voice.language) {
            seen.push(voice.language_view());
        }
    }
    seen
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_unique_and_sizes_are_real() {
        for (index, voice) in VOICES.iter().enumerate() {
            assert!(voice.bytes > 0, "{} shows no size", voice.id);
            assert!(
                !VOICES[..index].iter().any(|other| other.id == voice.id),
                "{} is listed twice",
                voice.id
            );
        }
    }

    #[test]
    fn the_card_size_is_the_sum_of_its_files() {
        for voice in VOICES {
            let listed: u64 = voice.files().iter().map(|(_, _, bytes)| *bytes).sum();
            assert_eq!(voice.bytes, listed, "{} card size drifted", voice.id);
        }
    }

    #[test]
    fn every_required_file_is_downloaded() {
        for voice in VOICES {
            let names: Vec<&str> = voice.files().iter().map(|(name, _, _)| *name).collect();
            for name in voice.required {
                assert!(
                    names.contains(name),
                    "{} requires {} but never downloads it",
                    voice.id,
                    name
                );
            }
            assert_eq!(
                voice.required.len(),
                names.len(),
                "{} downloads files nothing checks",
                voice.id
            );
        }
    }

    #[test]
    fn every_download_is_https() {
        for voice in VOICES {
            for (_, url, _) in voice.files() {
                assert!(
                    url.starts_with("https://"),
                    "{} downloads over {}",
                    voice.id,
                    url
                );
            }
        }
        match RUNTIME.source {
            Source::Zip(url, _) => {
                assert!(
                    url.starts_with("https://"),
                    "the runtime downloads over {}",
                    url
                )
            }
            Source::Files(_) => panic!("the runtime arrives as one archive"),
        }
    }

    #[test]
    fn supported_voices_come_with_weights_and_an_engine() {
        for voice in VOICES.iter().filter(|voice| voice.supported) {
            assert!(voice.source.is_some(), "{} cannot be installed", voice.id);
            assert!(!voice.required.is_empty(), "{} needs no files", voice.id);
            assert!(
                voice.unsupported_reason.is_none(),
                "{} is both runnable and refused",
                voice.id
            );
            // An engine that needs something installed is only runnable once
            // that runtime is; an in-process engine has nothing to install.
            match voice.engine {
                TtsEngine::Piper => assert_eq!(
                    voice.supported, RUNTIME_READY,
                    "{} should follow the Piper binary",
                    voice.id
                ),
                TtsEngine::Kale | TtsEngine::Supertonic | TtsEngine::Kokoro => assert!(
                    voice.supported,
                    "{} runs in process and cannot be refused",
                    voice.id
                ),
            }
        }
        for voice in VOICES.iter().filter(|voice| !voice.supported) {
            assert!(
                voice.unsupported_reason.is_some(),
                "{} gives no reason",
                voice.id
            );
        }
    }

    #[test]
    fn onnx_voices_ship_a_graph_a_front_end_and_a_style_table() {
        for voice in VOICES
            .iter()
            .filter(|voice| voice.engine != TtsEngine::Piper)
        {
            let names: Vec<&str> = voice.files().iter().map(|(name, _, _)| *name).collect();
            assert!(
                voice.required.iter().any(|name| name.ends_with(".onnx")),
                "{} is an ONNX voice with no graph",
                voice.id
            );
            // Every in-process engine needs a table that turns characters or
            // phonemes into rows: Kale's tokenizer.json, Supertonic's indexer,
            // or the shared front end's id table for a voice that reuses it.
            let shared_ids = FRONTEND
                .files
                .iter()
                .any(|(name, _, _)| *name == "tokenizer.json");
            assert!(
                voice.required.contains(&"tokenizer.json")
                    || voice.required.contains(&"unicode_indexer.json")
                    || shared_ids,
                "{} has no id table, so no text could be sent",
                voice.id
            );
            // Kale brings front-end weights and a style table; Supertonic
            // brings its style as one JSON.
            assert!(
                voice.required.iter().any(|name| name.ends_with(".bin"))
                    || voice.required.iter().any(|name| name.ends_with(".json")),
                "{} downloads no weights or style",
                voice.id
            );
            for name in voice.required {
                assert!(
                    names.contains(name),
                    "{} requires {} but never downloads it",
                    voice.id,
                    name
                );
            }
        }
        for voice in VOICES
            .iter()
            .filter(|voice| voice.engine == TtsEngine::Piper)
        {
            let names: Vec<&str> = voice.files().iter().map(|(name, _, _)| *name).collect();
            assert!(
                names.iter().any(|name| name.ends_with(".onnx")),
                "{} has no weights",
                voice.id
            );
            assert!(
                names.iter().any(|name| name.ends_with(".onnx.json")),
                "{} has no voice description",
                voice.id
            );
        }
    }

    #[test]
    fn sample_rates_and_speakers_are_the_ones_piper_writes() {
        for voice in VOICES
            .iter()
            .filter(|voice| voice.engine == TtsEngine::Piper)
        {
            assert!(
                matches!(voice.sample_rate, 16_000 | 22_050),
                "{} claims {} Hz",
                voice.id,
                voice.sample_rate
            );
            assert!(voice.speakers > 0, "{} carries no speakers", voice.id);
        }
    }

    #[test]
    fn a_gender_is_only_set_where_a_card_states_one() {
        for voice in VOICES {
            if let Some(gender) = voice.gender {
                assert!(
                    !gender.trim().is_empty(),
                    "{} carries an empty gender",
                    voice.id
                );
            }
        }
    }

    #[test]
    fn language_codes_look_like_piper_ones() {
        for voice in VOICES {
            let (family, region) = voice
                .language
                .split_once('_')
                .unwrap_or((voice.language, ""));
            assert!(
                family.len() == 2 && region.len() == 2,
                "{} names {} as a language",
                voice.id,
                voice.language
            );
            assert!(
                !voice.language_name.is_empty(),
                "{} has no language name",
                voice.id
            );
        }
    }

    #[test]
    fn the_runtime_is_downloadable_and_checkable() {
        // Read at runtime rather than compared as a constant, so a size of zero
        // would be a failing test rather than a build error.
        let bytes = RUNTIME.bytes;
        assert!(bytes > 0);
        assert!(
            RUNTIME.files.contains(&"piper.exe"),
            "the bundle has to include the binary"
        );
        assert!(!RUNTIME.dirs.is_empty(), "espeak-ng-data is a directory");
        match RUNTIME.source {
            Source::Zip(_, bytes) => assert_eq!(RUNTIME.bytes, bytes, "the runtime size drifted"),
            Source::Files(_) => panic!("the runtime arrives as one archive"),
        }
    }

    #[test]
    fn find_matches_whole_ids_only() {
        assert_eq!(find("en_US-amy-low").map(|voice| voice.name), Some("Amy"));
        assert!(find("en_US-amy").is_none());
        assert!(find("").is_none());
    }
}
