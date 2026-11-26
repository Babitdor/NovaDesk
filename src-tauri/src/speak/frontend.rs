//! Kale's text front end: text in, Kokoro phoneme ids out, inside this process.
//!
//! Kale is not a character-input engine. The shipped graph takes the same
//! phoneme ids Kokoro does, so a voice is useless without a grapheme-to-phoneme
//! step in front of it, and this module is that step. It ports the model's own
//! front end rather than calling eSpeak: espeak-ng is GPLv3, which is why Piper
//! runs as a child process today, while this is Apache-2.0 data - a word list,
//! a small GRU for the words the list has not got, and Kokoro's own id table.
//! Downloading a second binary to phonemise would also mean a second runtime
//! that can be missing, half-installed or the wrong version.
//!
//! What is ported, exactly:
//!
//! - the word list (lexicon.tsv) and its overrides (override.public.tsv),
//!   keyed by lower-case word, valued by an IPA reading;
//! - the inflection search: a word that is not listed is looked up again with
//!   the plural and past endings removed, and the ending's sound is appended;
//! - acronym spelling ("GPU" as "gee pee you", one stress, letters apart);
//! - the GRU (g2p.bin, its own KG2P format) for words still unknown, and the
//!   two-listed-words split that beats it when the guess has more vowels;
//! - the letter-sound fallback for the rest, so nothing is ever dropped;
//! - Kokoro's id table (tokenizer.json), so the caller gets model input.
//!
//! What is deliberately not ported, each a project in its own right:
//!
//! - dizione's text rules: money, dates, times, units, codes, dotted file
//!   names, list commas and weekday abbreviations. Digits and decimals are
//!   spelled by numbers() below; everything else gets the plain reading of its
//!   digits, so "1.1x" is said "one point one x".
//! - homograph disambiguation ("read" past or present, "lead" the metal). A
//!   listed word keeps the word list's own reading.
//! - hyphenated compounds: the hyphen is a word break here, so "well-known"
//!   is read as two words instead of one joined word with a secondary stress.
//!
//! Every miss costs a pause or a plural, never silence: the word list, then the
//! GRU, then the letters, and a character the id table does not know is simply
//! not sent.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;

use crate::error::AppError;

/// Kokoro's own ceiling. A sentence longer than this would be refused by the
/// graph, so the caller sends sentences and this cuts at the cap as a last
/// resort rather than letting the session fail.
pub const MAX_TOKENS: usize = 512;

/// How a letter is said when nothing else knows the word. Kokoro's phoneme set
/// has no letters of its own, so these are the sounds, not the names.
const LETTER_SOUND: &[(char, &str)] = &[
    ('a', "\u{e6}"),
    ('b', "b"),
    ('c', "k"),
    ('d', "d"),
    ('e', "\u{25b}"),
    ('f', "f"),
    ('g', "\u{261}"),
    ('h', "h"),
    ('i', "\u{26a}"),
    ('j', "d\u{292}"),
    ('k', "k"),
    ('l', "l"),
    ('m', "m"),
    ('n', "n"),
    ('o', "\u{251}\u{2d0}"),
    ('p', "p"),
    ('q', "k"),
    ('r', "\u{279}"),
    ('s', "s"),
    ('t', "t"),
    ('u', "\u{28c}"),
    ('v', "v"),
    ('w', "w"),
    ('x', "ks"),
    ('y', "j"),
    ('z', "z"),
];

fn letter_sound(ch: char) -> Option<&'static str> {
    LETTER_SOUND
        .iter()
        .find(|(letter, _)| *letter == ch)
        .map(|(_, sound)| *sound)
}

/// Letters from other alphabets that a word list entry may carry are folded to
/// the ASCII ones the tables were built from; any other character is left
/// alone and stops at the id table, which filters rather than guesses.
const FOLD: &[(char, &str)] = &[
    ('\u{e6}', "ae"),
    ('\u{c6}', "Ae"),
    ('\u{153}', "oe"),
    ('\u{152}', "Oe"),
    ('\u{f8}', "o"),
    ('\u{d8}', "O"),
    ('\u{df}', "ss"),
    ('\u{142}', "l"),
    ('\u{141}', "L"),
    ('\u{111}', "d"),
    ('\u{110}', "D"),
    ('\u{f0}', "d"),
    ('\u{d0}', "D"),
    ('\u{fe}', "th"),
    ('\u{de}', "Th"),
    ('\u{131}', "i"),
    ('\u{2bb}', ""),
    ('\u{2bc}', ""),
    ('\u{2018}', "'"),
    ('\u{2019}', "'"),
    ('\u{60}', "'"),
];

/// Combining marks are dropped rather than folded: the tables are precomposed,
/// so a decomposed letter plus accent has to become the plain letter to be
/// found at all.
fn is_combining_mark(ch: char) -> bool {
    matches!(
        ch as u32,
        0x0300..=0x036F
            | 0x0483..=0x0489
            | 0x1AB0..=0x1AFF
            | 0x1DC0..=0x1DFF
            | 0x20D0..=0x20FF
            | 0xFE20..=0xFE2F
    )
}

/// Vowels in the Kokoro phoneme set. A diphthong counts as one sound elsewhere
/// (see vowel_count); here one character is enough to place the stress.
const VOWELS: &str = "aeiou\u{251}\u{250}\u{252}\u{e6}\u{254}\u{259}\u{258}\u{25a}\u{25b}\u{25c}\u{25d}\u{25e}\u{268}\u{26a}\u{28a}\u{28c}\u{264}\u{f8}\u{153}\u{276}\u{26f}\u{275}\u{289}\u{28f}\u{1d7b}";

fn is_vowel(ch: char) -> bool {
    VOWELS.contains(ch)
}

/// Endings that hiss, so a plural on them is a whole syllable rather than a
/// consonant ("buses" is two, "cats" is one).
const SIBILANT: &[&str] = &["s", "z", "\u{283}", "\u{292}", "t\u{283}", "d\u{292}"];
/// Voiceless endings take "t" in the past and "s" in the plural.
const VOICELESS: &[&str] = &["p", "t", "k", "f", "\u{3b8}"];

/// A function word with nothing after it is said in its strong form; before a
/// word the word list's weak form is the right one.
fn final_strong(low: &str) -> Option<&'static str> {
    match low {
        "the" => Some("\u{f0}\u{2c8}\u{28c}"),
        "a" => Some("\u{2c8}e\u{26a}"),
        "an" => Some("\u{2c8}\u{e6}n"),
        "to" => Some("t\u{2c8}u\u{2d0}"),
        _ => None,
    }
}

fn unstress(ipa: &str) -> String {
    ipa.chars()
        .filter(|ch| *ch != '\u{2c8}' && *ch != '\u{2cc}')
        .collect()
}

fn secondary(ipa: &str) -> String {
    ipa.chars()
        .map(|ch| if ch == '\u{2c8}' { '\u{2cc}' } else { ch })
        .collect()
}

fn bare(ipa: &str) -> String {
    ipa.chars()
        .filter(|ch| !matches!(*ch, '\u{2c8}' | '\u{2cc}' | '\u{2d0}'))
        .collect()
}

fn stress_first_vowel(ipa: &str) -> String {
    let plain = unstress(ipa);
    let mut out = String::with_capacity(plain.len() + 2);
    let mut placed = false;
    for ch in plain.chars() {
        if !placed && is_vowel(ch) {
            out.push('\u{2c8}');
            placed = true;
        }
        out.push(ch);
    }
    out
}

/// How many vowel sounds a reading has. The two-listed-words split competes
/// with the GRU's guess on this count, exactly as the reference does.
fn vowel_count(ipa: &str) -> usize {
    const DIPHTHONGS: &[&str] = &[
        "e\u{26a}",
        "a\u{26a}",
        "a\u{28a}",
        "\u{254}\u{26a}",
        "o\u{28a}",
        "\u{259}\u{28a}",
        "\u{251}\u{2d0}",
        "i\u{2d0}",
        "u\u{2d0}",
        "\u{25c}\u{2d0}",
        "\u{254}\u{2d0}",
        "o\u{2d0}",
    ];
    let chars: Vec<char> = ipa.chars().collect();
    let mut count = 0;
    let mut at = 0;
    while at < chars.len() {
        let pair: String = chars[at..(at + 2).min(chars.len())].iter().collect();
        if DIPHTHONGS.contains(&pair.as_str()) {
            count += 1;
            at += 2;
            continue;
        }
        if is_vowel(chars[at]) {
            count += 1;
        }
        at += 1;
    }
    count
}

fn edit_distance(a: &str, b: &str) -> usize {
    let x: Vec<char> = a.chars().collect();
    let y: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=y.len()).collect();
    for i in 1..=x.len() {
        let mut next = vec![i; y.len() + 1];
        for j in 1..=y.len() {
            next[j] = (row[j] + 1)
                .min(next[j - 1] + 1)
                .min(row[j - 1] + usize::from(x[i - 1] != y[j - 1]));
        }
        row = next;
    }
    row[y.len()]
}

fn plural_suffix(ipa: &str) -> &'static str {
    if SIBILANT.iter().any(|sound| ipa.ends_with(sound)) {
        "\u{1d7b}z"
    } else if VOICELESS.iter().any(|sound| ipa.ends_with(sound)) {
        "s"
    } else {
        "z"
    }
}

fn past_suffix(ipa: &str) -> &'static str {
    if ipa.ends_with('t') || ipa.ends_with('d') {
        return "\u{1d7b}d";
    }
    if VOICELESS.iter().any(|sound| ipa.ends_with(sound))
        || ipa.ends_with('s')
        || ipa.ends_with('\u{283}')
    {
        return "t";
    }
    "d"
}

/// The ending a stem search added, and therefore the sound to put back on.
#[derive(Debug, Clone, Copy)]
enum Suffix {
    Plural,
    Nothing,
    Past,
    Ing,
    Fixed(&'static str),
}

impl Suffix {
    fn sound(self, base: &str) -> &'static str {
        match self {
            Suffix::Plural => plural_suffix(base),
            Suffix::Nothing => "",
            Suffix::Past => past_suffix(base),
            Suffix::Ing => "\u{26a}\u{14b}",
            Suffix::Fixed(sound) => sound,
        }
    }
}

/// Endings with a fixed sound, because the word list spells the stem without
/// them and the ending is the same wherever it lands.
const SUFFIXES: &[(&str, &str)] = &[
    ("ly", "li"),
    ("ness", "n\u{259}s"),
    ("ful", "f\u{259}l"),
    ("less", "l\u{259}s"),
    ("er", "\u{25a}"),
    ("est", "\u{26a}st"),
    ("ment", "m\u{259}nt"),
];

/// Two sentences are two graph calls, and the second one starts inside the
/// first one's tail unless a moment of silence is put between them.
pub const GAP_SECONDS: f32 = 0.2;

// ------------------------------------------------------------------------------------------------ numbers

const ONES: &[&str] = &[
    "zero",
    "one",
    "two",
    "three",
    "four",
    "five",
    "six",
    "seven",
    "eight",
    "nine",
    "ten",
    "eleven",
    "twelve",
    "thirteen",
    "fourteen",
    "fifteen",
    "sixteen",
    "seventeen",
    "eighteen",
    "nineteen",
];
const TENS: &[&str] = &[
    "", "", "twenty", "thirty", "forty", "fifty", "sixty", "seventy", "eighty", "ninety",
];
const SCALES: &[(u64, &str)] = &[
    (1_000_000_000_000, "trillion"),
    (1_000_000_000, "billion"),
    (1_000_000, "million"),
    (1_000, "thousand"),
];

fn below_thousand(value: u64) -> String {
    let mut parts: Vec<String> = Vec::new();
    let hundreds = value / 100;
    let rest = value % 100;
    if hundreds > 0 {
        parts.push(format!("{} hundred", ONES[hundreds as usize]));
    }
    if rest > 0 {
        if rest < 20 {
            parts.push(ONES[rest as usize].to_string());
        } else {
            let tens = TENS[(rest / 10) as usize];
            let ones = rest % 10;
            if ones == 0 {
                parts.push(tens.to_string());
            } else {
                parts.push(format!("{tens} {}", ONES[ones as usize]));
            }
        }
    }
    parts.join(" ")
}

fn integer_words(mut value: u64) -> String {
    if value == 0 {
        return ONES[0].to_string();
    }
    let mut parts: Vec<String> = Vec::new();
    for (scale, name) in SCALES {
        if value >= *scale {
            parts.push(format!("{} {name}", integer_words(value / *scale)));
            value %= *scale;
        }
    }
    if value > 0 {
        parts.push(below_thousand(value));
    }
    parts.join(" ")
}

/// Digits as words, so they reach the word list at all: its keys are words, and
/// the id table has no digit entries, so an unspelled "42" is a word nothing
/// downstream can read. A comma-separated run keeps its thousands, a decimal
/// point is said "point", and a run too long for u64 is said digit by digit.
fn numbers(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    while at < chars.len() {
        let starts_a_run =
            chars[at].is_ascii_digit() && (at == 0 || !chars[at - 1].is_ascii_alphanumeric());
        if !starts_a_run {
            out.push(chars[at]);
            at += 1;
            continue;
        }
        let mut integer = String::new();
        let mut fraction = String::new();
        while at < chars.len() && (chars[at].is_ascii_digit() || chars[at] == ',') {
            if chars[at] != ',' {
                integer.push(chars[at]);
            }
            at += 1;
        }
        if at + 1 < chars.len() && chars[at] == '.' && chars[at + 1].is_ascii_digit() {
            at += 1;
            while at < chars.len() && chars[at].is_ascii_digit() {
                fraction.push(chars[at]);
                at += 1;
            }
        }
        if !out.is_empty() && !out.ends_with(' ') {
            out.push(' ');
        }
        match integer.parse::<u64>() {
            Ok(value) => out.push_str(&integer_words(value)),
            Err(_) => {
                for (index, digit) in integer.chars().enumerate() {
                    if index > 0 {
                        out.push(' ');
                    }
                    out.push_str(ONES[digit.to_digit(10).unwrap_or(0) as usize]);
                }
            }
        }
        if !fraction.is_empty() {
            out.push_str(" point");
            for digit in fraction.chars() {
                out.push(' ');
                out.push_str(ONES[digit.to_digit(10).unwrap_or(0) as usize]);
            }
        }
        if at < chars.len() && !chars[at].is_whitespace() {
            out.push(' ');
        }
    }
    out
}

// ------------------------------------------------------------------------------------------------ the GRU

/// One gate matrix set as the file stores it: row-major, with the row length
/// taken from the vector being multiplied rather than stored here.
#[derive(Debug, Clone)]
struct Gru {
    w_ih: Vec<f64>,
    w_hh: Vec<f64>,
    b_ih: Vec<f64>,
    b_hh: Vec<f64>,
    size: usize,
}

fn half_to_f64(bits: u16) -> f64 {
    let sign = if bits & 0x8000 != 0 { -1.0 } else { 1.0 };
    let exponent = (bits >> 10) & 0x1f;
    let mantissa = bits & 0x3ff;
    let value = if exponent == 0 {
        sign * f64::from(mantissa) * 2f64.powi(-24)
    } else if exponent == 0x1f {
        if mantissa == 0 {
            sign * f64::INFINITY
        } else {
            f64::NAN
        }
    } else {
        sign * (1.0 + f64::from(mantissa) / 1024.0) * 2f64.powi(i32::from(exponent) - 15)
    };
    // The reference rounds every weight to f32 as it reads it, so matching that
    // keeps this port's arithmetic comparable with the one it was ported from.
    f64::from(value as f32)
}

fn mat_vec(matrix: &[f64], vector: &[f64], rows: usize, bias: Option<&[f64]>) -> Vec<f64> {
    let mut out = vec![0.0; rows];
    for (row, slot) in out.iter_mut().enumerate() {
        let mut sum = bias.map(|values| values[row]).unwrap_or(0.0);
        let base = row * vector.len();
        for (index, value) in vector.iter().enumerate() {
            sum += matrix[base + index] * value;
        }
        *slot = sum;
    }
    out
}

fn sigmoid(x: f64) -> f64 {
    1.0 / (1.0 + (-x).exp())
}

fn dot_row(gru: &Gru, row: usize, hidden: &[f64]) -> f64 {
    let base = row * gru.size;
    let mut sum = gru.b_hh[row];
    for (index, value) in hidden.iter().enumerate() {
        sum += gru.w_hh[base + index] * value;
    }
    sum
}

/// One step of PyTorch's GRU: reset and update gates from the input and the
/// previous state, then a candidate from the reset gate. The order of the three
/// gate blocks in the stored matrices is what makes this portable at all.
fn gru_cell(gru: &Gru, input: &[f64], hidden: &[f64]) -> Vec<f64> {
    let size = gru.size;
    let dot_in = |row: usize| -> f64 {
        let base = row * input.len();
        let mut sum = gru.b_ih[row];
        for (index, value) in input.iter().enumerate() {
            sum += gru.w_ih[base + index] * value;
        }
        sum
    };
    let mut out = vec![0.0; size];
    for k in 0..size {
        let reset = sigmoid(dot_in(k) + dot_row(gru, k, hidden));
        let update = sigmoid(dot_in(size + k) + dot_row(gru, size + k, hidden));
        let candidate = (dot_in(2 * size + k) + reset * dot_row(gru, 2 * size + k, hidden)).tanh();
        out[k] = (1.0 - update) * candidate + update * hidden[k];
    }
    out
}

/// The front end's word model: a two-way GRU encoder, an attention decoder and
/// an output projection, all read from the model's own G2P file.
struct G2pModel {
    e: usize,
    h: usize,
    d: usize,
    max_out: usize,
    outputs: Vec<u32>,
    input_index: HashMap<u32, usize>,
    emb_in: Vec<f64>,
    enc_fwd: Gru,
    enc_bwd: Gru,
    init_w: Vec<f64>,
    init_b: Vec<f64>,
    emb_out: Vec<f64>,
    dec: Gru,
    att_w: Vec<f64>,
    comb_w: Vec<f64>,
    comb_b: Vec<f64>,
    out_w: Vec<f64>,
    out_b: Vec<f64>,
}

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn int(&mut self) -> Result<i32, AppError> {
        let end = self.at + 4;
        let slice = self
            .bytes
            .get(self.at..end)
            .ok_or_else(|| AppError::Speak("the g2p weights end early".to_string()))?;
        self.at = end;
        Ok(i32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]))
    }

    fn floats(&mut self, count: usize) -> Result<Vec<f64>, AppError> {
        let end = self.at + count * 2;
        let slice = self
            .bytes
            .get(self.at..end)
            .ok_or_else(|| AppError::Speak("the g2p weights end early".to_string()))?;
        self.at = end;
        Ok(slice
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| half_to_f64(u16::from_le_bytes(*pair)))
            .collect())
    }

    /// The four matrices of one GRU, in the order the reference reads them.
    fn gru(&mut self, cols: usize, size: usize) -> Result<Gru, AppError> {
        Ok(Gru {
            w_ih: self.floats(3 * size * cols)?,
            w_hh: self.floats(3 * size * size)?,
            b_ih: self.floats(3 * size)?,
            b_hh: self.floats(3 * size)?,
            size,
        })
    }
}

impl G2pModel {
    /// Reads g2p.bin. Every count in the header is used to check the file: the
    /// weights must be consumed exactly, or the layout this build assumes is
    /// not the layout the file has.
    fn load(path: &Path) -> Result<Self, AppError> {
        let bytes = std::fs::read(path)
            .map_err(|err| AppError::Speak(format!("the g2p model could not be read: {err}")))?;
        if bytes.len() < 8 || &bytes[0..4] != b"KG2P" {
            return Err(AppError::Speak(
                "the g2p model is not in the KG2P format this build reads".to_string(),
            ));
        }
        let mut reader = Reader {
            bytes: &bytes,
            at: 4,
        };
        let version = reader.int()?;
        if version != 1 {
            return Err(AppError::Speak(format!(
                "the g2p model is version {version}; this build reads version 1"
            )));
        }
        let e = reader.int()? as usize;
        let h = reader.int()? as usize;
        let d = reader.int()? as usize;
        let inputs = reader.int()? as usize;
        let outputs = reader.int()? as usize;
        let max_out = reader.int()? as usize;
        let mut input_codes = Vec::with_capacity(inputs);
        for _ in 0..inputs {
            input_codes.push(reader.int()? as u32);
        }
        let mut output_codes = Vec::with_capacity(outputs);
        for _ in 0..outputs {
            output_codes.push(reader.int()? as u32);
        }
        let emb_in = reader.floats(inputs * e)?;
        let enc_fwd = reader.gru(e, h)?;
        let enc_bwd = reader.gru(e, h)?;
        let init_w = reader.floats(d * 2 * h)?;
        let init_b = reader.floats(d)?;
        let emb_out = reader.floats(outputs * e)?;
        let dec = reader.gru(e + 2 * h, d)?;
        let att_w = reader.floats(2 * h * d)?;
        let comb_w = reader.floats(d * (d + 2 * h))?;
        let comb_b = reader.floats(d)?;
        let out_w = reader.floats(outputs * d)?;
        let out_b = reader.floats(outputs)?;
        if reader.at != bytes.len() {
            return Err(AppError::Speak(format!(
                "the g2p model carries {} bytes this build does not read",
                bytes.len() - reader.at
            )));
        }
        // Code point 0 is the blank the encoder starts from, so it is not a
        // letter: only the codes after it are looked up per character.
        let mut input_index = HashMap::with_capacity(input_codes.len());
        for (index, code) in input_codes.iter().enumerate().skip(1) {
            input_index.insert(*code, index);
        }
        Ok(Self {
            e,
            h,
            d,
            max_out,
            outputs: output_codes,
            input_index,
            emb_in,
            enc_fwd,
            enc_bwd,
            init_w,
            init_b,
            emb_out,
            dec,
            att_w,
            comb_w,
            comb_b,
            out_w,
            out_b,
        })
    }

    /// The reading of a word the word list has not got, or None when the model
    /// cannot spell it either: a character outside its alphabet, a guess with
    /// no vowel in it, or one that never reached its end token.
    fn phonemes(&self, word: &str) -> Option<String> {
        let chars: Vec<char> = word.chars().collect();
        if chars.is_empty() || chars.len() > 24 {
            return None;
        }
        let mut ids = Vec::with_capacity(chars.len());
        for ch in &chars {
            ids.push(*self.input_index.get(&(*ch as u32))?);
        }
        let embed = |table: &[f64], id: usize| table[id * self.e..(id + 1) * self.e].to_vec();
        let mut forward: Vec<Vec<f64>> = Vec::with_capacity(ids.len());
        let mut hidden = vec![0.0; self.h];
        for id in &ids {
            hidden = gru_cell(&self.enc_fwd, &embed(&self.emb_in, *id), &hidden);
            forward.push(hidden.clone());
        }
        let mut backward: Vec<Vec<f64>> = vec![Vec::new(); ids.len()];
        let mut hidden = vec![0.0; self.h];
        for at in (0..ids.len()).rev() {
            hidden = gru_cell(&self.enc_bwd, &embed(&self.emb_in, ids[at]), &hidden);
            backward[at] = hidden.clone();
        }
        let encoded: Vec<Vec<f64>> = forward
            .iter()
            .zip(backward.iter())
            .map(|(front, back)| {
                let mut joined = front.clone();
                joined.extend_from_slice(back);
                joined
            })
            .collect();
        let mut seed = forward.last()?.clone();
        seed.extend_from_slice(backward.first()?);
        let mut state: Vec<f64> = mat_vec(&self.init_w, &seed, self.d, Some(&self.init_b))
            .into_iter()
            .map(f64::tanh)
            .collect();
        let mut context = vec![0.0; 2 * self.h];
        let mut previous = 1usize;
        let mut said = String::new();
        let limit = self.max_out.min(3 * ids.len() + 6);
        let mut ended = false;
        for _ in 0..limit {
            let mut input = embed(&self.emb_out, previous);
            input.extend_from_slice(&context);
            state = gru_cell(&self.dec, &input, &state);
            let query = mat_vec(&self.att_w, &state, 2 * self.h, None);
            let mut scores: Vec<f64> = encoded
                .iter()
                .map(|vector| {
                    vector
                        .iter()
                        .zip(query.iter())
                        .map(|(a, b)| a * b)
                        .sum::<f64>()
                })
                .collect();
            let top = scores.iter().cloned().fold(f64::MIN, f64::max);
            let mut total = 0.0;
            for score in scores.iter_mut() {
                *score = (*score - top).exp();
                total += *score;
            }
            let mut next_context = vec![0.0; 2 * self.h];
            for (at, vector) in encoded.iter().enumerate() {
                let weight = scores[at] / total;
                for (slot, value) in next_context.iter_mut().zip(vector.iter()) {
                    *slot += weight * value;
                }
            }
            context = next_context;
            let mut combined = state.clone();
            combined.extend_from_slice(&context);
            let hidden: Vec<f64> = mat_vec(&self.comb_w, &combined, self.d, None)
                .into_iter()
                .zip(self.comb_b.iter())
                .map(|(value, bias)| (value + bias).tanh())
                .collect();
            let logits = mat_vec(&self.out_w, &hidden, self.outputs.len(), Some(&self.out_b));
            let mut best = 0;
            for (at, logit) in logits.iter().enumerate() {
                if *logit > logits[best] {
                    best = at;
                }
            }
            if best == 2 {
                ended = true;
                break;
            }
            if best < 3 {
                return None;
            }
            said.push(char::from_u32(self.outputs[best])?);
            previous = best;
        }
        if !ended || !said.chars().any(is_vowel) {
            return None;
        }
        Some(said)
    }
}

// ------------------------------------------------------------------------------------------------ the front end

/// One table of "word<TAB>reading" lines, as the model's own loader reads it:
/// blank lines and lines starting with "#" are skipped, and the override file
/// is read into the same map so it wins over the word list.
fn read_table(path: &Path, into: &mut HashMap<String, String>) -> Result<(), AppError> {
    let text = std::fs::read_to_string(path)
        .map_err(|err| AppError::Speak(format!("{} could not be read: {err}", path.display())))?;
    for line in text.lines() {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((word, reading)) = line.split_once('\t') {
            into.insert(word.to_string(), reading.to_string());
        }
    }
    Ok(())
}

/// Kokoro's id for every phoneme the graph knows, from tokenizer.json. A symbol
/// missing from this map is a symbol the graph was not trained on, so the ids
/// are filtered rather than padded with a guess.
fn read_vocab(path: &Path) -> Result<HashMap<char, i64>, AppError> {
    let text = std::fs::read_to_string(path)
        .map_err(|err| AppError::Speak(format!("tokenizer.json could not be read: {err}")))?;
    let json: serde_json::Value = serde_json::from_str(&text)
        .map_err(|err| AppError::Speak(format!("tokenizer.json is not valid JSON: {err}")))?;
    let vocab = json
        .get("model")
        .and_then(|model| model.get("vocab"))
        .and_then(|vocab| vocab.as_object())
        .ok_or_else(|| {
            AppError::Speak("tokenizer.json has no model.vocab to read ids from".to_string())
        })?;
    let mut map = HashMap::with_capacity(vocab.len());
    for (symbol, id) in vocab {
        let mut chars = symbol.chars();
        if let (Some(ch), None) = (chars.next(), chars.next()) {
            if let Some(id) = id.as_i64() {
                map.insert(ch, id);
            }
        }
    }
    Ok(map)
}

/// The sentences to read, one graph call each: the model was trained sentence
/// by sentence and its ids stop at 512, so a reply is split here rather than
/// truncated there. A one-word sentence rides with the next one, because the
/// small voices garble a word said on its own, and the text's last word rides
/// with the sentence before it.
fn sentences(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut carry = String::new();
    for part in split_sentences(text) {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let joined = if carry.is_empty() {
            part.to_string()
        } else {
            format!("{carry} {part}")
        };
        if word_count(&joined) <= 1 {
            carry = joined;
            continue;
        }
        out.push(joined);
        carry.clear();
    }
    if !carry.is_empty() {
        match out.last_mut() {
            Some(last) => {
                last.push(' ');
                last.push_str(&carry);
            }
            None => out.push(carry),
        }
    }
    out
}

fn split_sentences(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\n' {
            if !current.trim().is_empty() {
                out.push(std::mem::take(&mut current));
            }
            continue;
        }
        current.push(ch);
        if matches!(ch, '.' | '!' | '?' | '\u{2026}') {
            // A closing quote or bracket belongs to the sentence it closes.
            while let Some(next) = chars.peek() {
                if matches!(next, '"' | '\'' | ')' | ']' | '\u{201d}' | '\u{2019}') {
                    current.push(*next);
                    chars.next();
                } else {
                    break;
                }
            }
            out.push(std::mem::take(&mut current));
        }
    }
    if !current.trim().is_empty() {
        out.push(current);
    }
    out
}

fn word_count(text: &str) -> usize {
    text.split(|ch: char| !(ch.is_alphabetic() || ch.is_ascii_digit() || ch == '\''))
        .filter(|part| !part.is_empty())
        .count()
}

/// One token of a sentence: a word to look up, or a mark to leave on the word
/// before it. The final full stop is what tells the model to stop, so marks are
/// ids like any other and are not dropped with the spaces.
#[derive(Debug, Clone)]
enum Token {
    Word(String),
    Mark(char),
}

fn mark_for(ch: char) -> Option<char> {
    match ch {
        '.' | ',' | '!' | '?' | ';' | ':' | '\u{2026}' => Some(ch),
        '\u{2014}' | '\u{2013}' => Some(','),
        _ => None,
    }
}

/// Words and marks in the order they are read. Everything else separates words,
/// the hyphen included: the reference joins compounds instead, and that is one
/// of the rules this port leaves out.
fn tokens(text: &str) -> Vec<Token> {
    let mut out = Vec::new();
    let mut word = String::new();
    for ch in text.chars() {
        if ch.is_alphabetic() || ch == '\'' || ch.is_ascii_digit() {
            word.push(ch);
            continue;
        }
        if !word.is_empty() {
            out.push(Token::Word(std::mem::take(&mut word)));
        }
        if let Some(mark) = mark_for(ch) {
            out.push(Token::Mark(mark));
        }
    }
    if !word.is_empty() {
        out.push(Token::Word(word));
    }
    out
}

/// Text to model ids, one vector per sentence, each ready to be a graph input.
pub struct Frontend {
    words: HashMap<String, String>,
    model: Option<G2pModel>,
    vocab: HashMap<char, i64>,
    /// Words the GRU has already been asked about: a repeated unknown word
    /// costs one inference per synthesis, not one per mention.
    learnt: RefCell<HashMap<String, Option<String>>>,
}

impl Frontend {
    /// Reads the front-end files from the voice's own folder. The word list,
    /// the GRU and the id table are all required: without the GRU an unknown
    /// word falls straight to its letters, which reads names and new terms
    /// badly enough to be a worse experience than a refused voice.
    pub fn load(dir: &Path) -> Result<Self, AppError> {
        let mut words = HashMap::new();
        read_table(&dir.join("lexicon.tsv"), &mut words)?;
        read_table(&dir.join("override.public.tsv"), &mut words)?;
        let model = G2pModel::load(&dir.join("g2p.bin"))?;
        let vocab = read_vocab(&dir.join("tokenizer.json"))?;
        Ok(Self {
            words,
            model: Some(model),
            vocab,
            learnt: RefCell::new(HashMap::new()),
        })
    }

    /// Every sentence of the text as ids. A sentence that ends up with fewer
    /// than two ids has nothing the graph can pronounce, so it is dropped
    /// rather than sent as a click.
    pub fn ids(&self, text: &str) -> Vec<Vec<i64>> {
        let mut out = Vec::new();
        for sentence in sentences(&numbers(text)) {
            let ids: Vec<i64> = self
                .phonemes(&sentence)
                .chars()
                .filter_map(|ch| self.vocab.get(&ch).copied())
                .take(MAX_TOKENS)
                .collect();
            if ids.len() >= 2 {
                out.push(ids);
            }
        }
        out
    }

    /// One sentence as the IPA the graph was trained on: words separated by a
    /// space, and each mark left on the word it followed.
    fn phonemes(&self, sentence: &str) -> String {
        let mut folded = String::with_capacity(sentence.len());
        for ch in sentence.chars() {
            if is_combining_mark(ch) {
                continue;
            }
            match FOLD.iter().find(|(from, _)| *from == ch) {
                Some((_, to)) => folded.push_str(to),
                None => folded.push(ch),
            }
        }
        let tokens = tokens(&folded);
        let mut out: Vec<String> = Vec::new();
        for (at, token) in tokens.iter().enumerate() {
            match token {
                Token::Mark(mark) => {
                    if let Some(last) = out.last_mut() {
                        last.push(*mark);
                    }
                }
                Token::Word(word) => {
                    let next = match tokens.get(at + 1) {
                        Some(Token::Word(next)) => Some(next.as_str()),
                        _ => None,
                    };
                    let ipa = self.word(word, next);
                    if !ipa.is_empty() {
                        out.push(ipa);
                    }
                }
            }
        }
        out.join(" ")
    }

    fn word(&self, token: &str, next: Option<&str>) -> String {
        let low = token.to_lowercase();
        let shouty = token.chars().count() > 1 && *token == token.to_uppercase();
        if shouty {
            // A word list entry for the capitals wins, then a listed word of
            // three letters or more ("OK", "NASA" are words in it), and
            // otherwise the letters are its reading.
            if let Some(reading) = self.words.get(token) {
                return reading.clone();
            }
            if !(token.len() >= 3 && self.words.contains_key(&low)) && token.len() <= 5 {
                return self.spell(&low);
            }
        }
        if next.is_none() {
            if let Some(strong) = final_strong(&low) {
                return strong.to_string();
            }
        }
        self.lookup(&low, true)
    }

    fn lookup(&self, low: &str, original: bool) -> String {
        if let Some(reading) = self.words.get(low) {
            return self.letters(low, reading.clone());
        }
        if !original {
            return String::new();
        }
        for (stems, suffix) in self.stems(low) {
            for stem in stems {
                if let Some(base) = self.words.get(&stem) {
                    return format!("{base}{}", suffix.sound(base));
                }
            }
        }
        if low.chars().count() == 1 {
            let ch = low.chars().next().unwrap_or(' ');
            return self
                .words
                .get(low)
                .cloned()
                .or_else(|| letter_sound(ch).map(str::to_string))
                .unwrap_or_default();
        }
        if let Some(said) = self.fallback(low) {
            return self.joined(low, &said).unwrap_or(said);
        }
        // Two listed words beat letter sounds: "barcode" guessed wrongly is
        // still close, but "bar" and "code" are exact.
        let chars: Vec<char> = low.chars().collect();
        for at in 3..chars.len().saturating_sub(2) {
            let head: String = chars[..at].iter().collect();
            let tail: String = chars[at..].iter().collect();
            if let (Some(first), Some(second)) = (self.words.get(&head), self.words.get(&tail)) {
                return format!("{first}{}", secondary(second));
            }
        }
        self.chunks(low)
    }

    /// A guess of six letters or more that is also two listed words of three
    /// letters or more each, and whose guess has a vowel the pair has not, is
    /// read as the pair: the front end's own rule for compounds.
    fn joined(&self, low: &str, said: &str) -> Option<String> {
        let chars: Vec<char> = low.chars().collect();
        if chars.len() < 6 {
            return None;
        }
        for at in 3..=chars.len().saturating_sub(3) {
            let head: String = chars[..at].iter().collect();
            let tail: String = chars[at..].iter().collect();
            let (first, second) = match (self.words.get(&head), self.words.get(&tail)) {
                (Some(first), Some(second)) => (first, second),
                _ => continue,
            };
            let two = format!("{first}{}", secondary(second));
            let closer = vowel_count(said) > vowel_count(&two)
                && edit_distance(&bare(said), &bare(&two)) <= 1;
            return if closer { Some(two) } else { None };
        }
        None
    }

    fn fallback(&self, low: &str) -> Option<String> {
        let model = self.model.as_ref()?;
        if let Some(known) = self.learnt.borrow().get(low) {
            return known.clone();
        }
        let said = model.phonemes(low);
        let mut learnt = self.learnt.borrow_mut();
        if learnt.len() >= 4_096 {
            learnt.clear();
        }
        learnt.insert(low.to_string(), said.clone());
        said
    }

    /// The readings to try for a word that is not listed, in the order the
    /// reference tries them: its endings removed, and the sound each ending
    /// adds once the stem is found.
    fn stems(&self, word: &str) -> Vec<(Vec<String>, Suffix)> {
        let mut out: Vec<(Vec<String>, Suffix)> = Vec::new();
        let chars: Vec<char> = word.chars().collect();
        let stripped = |cut: usize| -> String { chars[..chars.len() - cut].iter().collect() };
        let doubled = |base: &str| -> Vec<String> {
            let base_chars: Vec<char> = base.chars().collect();
            let last = base_chars.last().copied().unwrap_or(' ');
            let before = base_chars.iter().rev().nth(1).copied().unwrap_or(' ');
            if base_chars.len() > 2 && last == before {
                vec![base_chars[..base_chars.len() - 1].iter().collect()]
            } else {
                Vec::new()
            }
        };
        if word.ends_with("'s") {
            out.push((vec![stripped(2)], Suffix::Plural));
        }
        if word.ends_with("s'") {
            out.push((vec![stripped(1)], Suffix::Nothing));
        }
        if word.ends_with("ies") && chars.len() > 4 {
            out.push((vec![format!("{}y", stripped(3))], Suffix::Plural));
        }
        if word.ends_with("es") && chars.len() > 3 {
            out.push((vec![stripped(2)], Suffix::Plural));
        }
        if word.ends_with('s') && !word.ends_with("ss") && chars.len() > 3 {
            out.push((vec![stripped(1)], Suffix::Plural));
        }
        if word.ends_with("ied") && chars.len() > 4 {
            out.push((vec![format!("{}y", stripped(3))], Suffix::Past));
        }
        if word.ends_with("ed") && chars.len() > 4 {
            let base = stripped(2);
            let mut candidates = vec![base.clone(), format!("{base}e")];
            candidates.extend(doubled(&base));
            out.push((candidates, Suffix::Past));
        }
        if word.ends_with("ing") && chars.len() > 5 {
            let base = stripped(3);
            let mut candidates = vec![base.clone(), format!("{base}e")];
            candidates.extend(doubled(&base));
            out.push((candidates, Suffix::Ing));
        }
        for (suffix, sound) in SUFFIXES {
            if word.ends_with(suffix) && chars.len() > suffix.len() + 2 {
                let base = stripped(suffix.chars().count());
                let mut candidates = vec![base.clone()];
                if base.ends_with('i') {
                    candidates.push(format!("{}y", &base[..base.len() - 1]));
                }
                candidates.extend(doubled(&base));
                out.push((candidates, Suffix::Fixed(sound)));
            }
        }
        out
    }

    /// A listed reading that is only the word's letter sounds is spelled the
    /// way an acronym is. The word list has "cpu" and "pr" as their letters;
    /// read as one word they are unintelligible, so they are spelled instead.
    fn letters(&self, low: &str, ipa: String) -> String {
        let count = low.chars().count();
        let all_letters = low.chars().all(|ch| ch.is_ascii_alphabetic());
        if !all_letters || !(2..=5).contains(&count) || low == "ok" {
            return ipa;
        }
        let mut names = String::new();
        for ch in low.chars() {
            if ch == 'a' {
                names.push_str("e\u{26a}");
            } else {
                let reading = self.words.get(&ch.to_string()).cloned().unwrap_or_default();
                names.push_str(&bare(&reading));
            }
        }
        if bare(&ipa) == names {
            self.spell(low)
        } else {
            ipa
        }
    }

    /// Letters said as names, only the last one stressed so they run on as one
    /// name rather than as separate words.
    fn spell(&self, word: &str) -> String {
        let mut parts: Vec<String> = Vec::new();
        for ch in word.chars() {
            let low = ch.to_lowercase().next().unwrap_or(ch);
            let part = match low {
                'a' => "\u{2c8}e\u{26a}".to_string(),
                'i' => "\u{2c8}a\u{26a}".to_string(),
                _ => self
                    .words
                    .get(&low.to_string())
                    .cloned()
                    .or_else(|| letter_sound(low).map(str::to_string))
                    .unwrap_or_default(),
            };
            if !part.is_empty() {
                parts.push(part);
            }
        }
        let last = match parts.pop() {
            Some(last) => last,
            None => return String::new(),
        };
        let mut out: Vec<String> = parts.iter().map(|part| secondary(part)).collect();
        out.push(last);
        out.join(" ")
    }

    /// The last resort: the longest listed piece of the word, then letter
    /// sounds, with the stress put on its first vowel so it stays one word.
    fn chunks(&self, word: &str) -> String {
        let chars: Vec<char> = word.chars().collect();
        let mut out = String::new();
        let mut at = 0;
        while at < chars.len() {
            let mut matched = false;
            let mut end = chars.len();
            while end > at + 2 {
                let piece: String = chars[at..end].iter().collect();
                if let Some(reading) = self.words.get(&piece) {
                    out.push_str(&unstress(reading));
                    at = end;
                    matched = true;
                    break;
                }
                end -= 1;
            }
            if !matched {
                out.push_str(letter_sound(chars[at]).unwrap_or(""));
                at += 1;
            }
        }
        stress_first_vowel(&out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digits_reach_the_word_list_as_words() {
        assert_eq!(numbers("42 apples"), "forty two apples");
        assert_eq!(numbers("2,026"), "two thousand twenty six");
        assert_eq!(numbers("3.5"), "three point five");
    }

    #[test]
    fn a_digit_run_inside_a_word_is_left_alone() {
        assert_eq!(numbers("a4 paper"), "a4 paper");
        assert_eq!(numbers("x86"), "x86");
    }

    #[test]
    fn sentences_split_at_their_marks_and_short_ones_ride_along() {
        assert_eq!(
            sentences("One. Two words here. Three. Four words here."),
            vec!["One. Two words here.", "Three. Four words here."]
        );
    }

    #[test]
    fn a_long_sentence_stays_one_sentence() {
        let text = "The quick brown fox jumps over the lazy dog.";
        assert_eq!(sentences(text), vec![text]);
    }

    #[test]
    fn endings_take_the_plural_that_matches_their_last_sound() {
        assert_eq!(plural_suffix("k\u{e6}t"), "s");
        assert_eq!(plural_suffix("b\u{251}g"), "z");
        assert_eq!(plural_suffix("b\u{28c}s"), "\u{1d7b}z");
    }

    #[test]
    fn endings_take_the_past_that_matches_their_last_sound() {
        assert_eq!(past_suffix("w\u{251}nt"), "\u{1d7b}d");
        assert_eq!(past_suffix("l\u{28a}k"), "t");
        assert_eq!(past_suffix("ple\u{26a}"), "d");
    }

    #[test]
    fn a_single_precision_weight_is_read_the_way_the_reference_reads_it() {
        assert_eq!(half_to_f64(0x3c00), 1.0);
        assert_eq!(half_to_f64(0xc000), -2.0);
        assert_eq!(half_to_f64(0x0000), 0.0);
    }

    #[test]
    fn words_and_marks_come_out_in_reading_order() {
        let tokens = tokens("Hello, world. Fine");
        assert!(matches!(&tokens[0], Token::Word(word) if word == "Hello"));
        assert!(matches!(tokens[1], Token::Mark(',')));
        assert!(matches!(&tokens[2], Token::Word(word) if word == "world"));
        assert!(matches!(tokens[3], Token::Mark('.')));
        assert!(matches!(&tokens[4], Token::Word(word) if word == "Fine"));
    }
}
