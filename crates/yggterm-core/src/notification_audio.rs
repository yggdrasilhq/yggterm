//! THE notification tune — one owner, two players.
//!
//! **Why this module exists.** The chime existed only as a JavaScript string
//! literal inside `notification_chime_script` (yggterm-shell), which made the
//! webview the only thing that could play it. That was fine until it wasn't:
//! measured on the GUI host 2026-07-26, an agent-injected chime had
//! `ctx.resume()` resolve, the sink go SUSPENDED → RUNNING, the sink-input
//! present, unmuted and uncorked — **and produced complete silence**, while a
//! system tone through the same speaker seconds later was clearly audible. The
//! remaining explanation is WebKitGTK's autoplay gate: with no real user
//! gesture the context streams SILENT samples. An agent cannot synthesize a
//! qualifying gesture, so the webview can never satisfy "make a noise when you
//! want my attention".
//!
//! So there is now a NATIVE player too. Two players means the tune must have
//! exactly one owner, or they drift and the user hears two different chimes
//! depending on which path fired. This module is that owner: the notes, the
//! envelope, the pre-roll and the flush tail live here, and BOTH the webview
//! script and the native renderer are derived from them.
//!
//! **The tune is user-approved (2026-07-27: "Approved — this is the spec").
//! Do not change any constant here without the user.**
//!
//! **The tune is MEASURED, not invented.** It was derived from three real
//! aircraft cabin-chime recordings (FFT + onset detection + amplitude
//! envelope), which settled three things that ear-guessing had got wrong:
//!
//! - every strike is a PURE SINE — one partial, no octave, no bell overtones;
//! - the hi-lo pair is a descending MINOR THIRD (D5 587.33 → B4 493.88), not
//!   the perfect fourth this tune shipped with before;
//! - the gap between hi and lo is ~1.03 s. The old 0.18 s read as rushed; the
//!   slow tempo is most of what makes the sound calm rather than urgent.
//!
//! Re-tuning by ear is how the first version went wrong. **Re-measure
//! instead** — the reference recordings and analysis scripts live outside this
//! repo (`ygg-chime-spec`).

/// The four tones. Distinguishable by PATTERN, exactly as in a real cabin: one
/// chime = passenger call, hi-lo = crew interphone, hi-lo ×3 = emergency. That
/// is what keeps them tellable apart by ear without reading the screen —
/// pitch alone never did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChimeTone {
    /// Info / attention ping: ONE chime (the passenger-call single note).
    Info,
    /// Completion: ONE chime. Deliberately the same pattern as `Info` — the
    /// cabin vocabulary has a single "something wants you", and inventing a
    /// fourth pattern to split them would make neither recognisable.
    Success,
    /// Warning: the hi-lo pair (the crew-interphone call).
    Warning,
    /// Error: the hi-lo pair three times over (the emergency pattern).
    Error,
}

impl ChimeTone {
    pub const ALL: &'static [ChimeTone] = &[
        ChimeTone::Info,
        ChimeTone::Success,
        ChimeTone::Warning,
        ChimeTone::Error,
    ];

    pub fn as_key(self) -> &'static str {
        match self {
            ChimeTone::Info => "info",
            ChimeTone::Success => "success",
            ChimeTone::Warning => "warning",
            ChimeTone::Error => "error",
        }
    }

    pub fn parse(value: &str) -> Option<ChimeTone> {
        match value.trim().to_ascii_lowercase().as_str() {
            "info" => Some(ChimeTone::Info),
            "success" => Some(ChimeTone::Success),
            "warning" | "warn" => Some(ChimeTone::Warning),
            "error" => Some(ChimeTone::Error),
            _ => None,
        }
    }
}

/// One note: when it starts relative to the chime origin, its pitch, and its
/// peak gain. Same triple shape as the JSON the webview consumes and as
/// `audio tune --notes`, so a tune auditioned on one path plays identically on
/// the other.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ChimeNote {
    pub start_s: f32,
    pub freq_hz: f32,
    pub peak: f32,
}

const fn note(start_s: f32, freq_hz: f32, peak: f32) -> ChimeNote {
    ChimeNote {
        start_s,
        freq_hz,
        peak,
    }
}

/// The high strike, measured on all three recordings.
const D5: f32 = 587.33;
/// A descending MINOR THIRD below D5 — measured, not chosen. NOT the perfect
/// fourth the pre-2026-07-27 tune used.
const B4: f32 = 493.88;
/// Approved level for the hi-lo strikes.
const PEAK_PAIR: f32 = 0.40;
/// The single note carries alone, so it sits a little higher.
const PEAK_SINGLE: f32 = 0.48;
/// Measured hi→lo gap. The old 0.18 read as rushed; the slow tempo is most of
/// what makes this calm rather than urgent.
const GAP: f32 = 1.03;
/// Pair period for the ×3 emergency pattern.
const PERIOD: f32 = 2.40;

const INFO_NOTES: &[ChimeNote] = &[note(0.0, D5, PEAK_SINGLE)];
const SUCCESS_NOTES: &[ChimeNote] = &[note(0.0, D5, PEAK_SINGLE)];
const WARNING_NOTES: &[ChimeNote] = &[note(0.0, D5, PEAK_PAIR), note(GAP, B4, PEAK_PAIR)];
const ERROR_NOTES: &[ChimeNote] = &[
    note(0.0, D5, PEAK_PAIR),
    note(GAP, B4, PEAK_PAIR),
    note(PERIOD, D5, PEAK_PAIR),
    note(PERIOD + GAP, B4, PEAK_PAIR),
    note(2.0 * PERIOD, D5, PEAK_PAIR),
    note(2.0 * PERIOD + GAP, B4, PEAK_PAIR),
];

pub fn tone_notes(tone: ChimeTone) -> &'static [ChimeNote] {
    match tone {
        ChimeTone::Info => INFO_NOTES,
        ChimeTone::Success => SUCCESS_NOTES,
        ChimeTone::Warning => WARNING_NOTES,
        ChimeTone::Error => ERROR_NOTES,
    }
}

/// Per-tone envelope time-stretch. The single note read as short against the
/// tone it replaced (the measured envelope is 1.08 s; the old one rang for
/// ~2.0 s), so it is stretched. The pairs are NOT stretched, because the notes
/// ringing into each other already carry the length.
pub fn tone_ring(tone: ChimeTone) -> f32 {
    match tone {
        ChimeTone::Info | ChimeTone::Success => 1.8,
        ChimeTone::Warning | ChimeTone::Error => 1.0,
    }
}

/// ⚠ RING WITHOUT TAIL_CUT IS WRONG. Stretching the envelope also stretches its
/// QUIET tail, which the user heard as "a resonant ending… like an aftertaste"
/// that a real cabin chime does not have. Cut once the envelope falls below
/// this fraction of peak, then fade out over [`ENV_FADE_SECONDS`]. Stretch the
/// body, never the tail. Unstretched tones need no cut (`0.0` disables).
pub fn tone_tail_cut(tone: ChimeTone) -> f32 {
    match tone {
        ChimeTone::Info | ChimeTone::Success => 0.08,
        ChimeTone::Warning | ChimeTone::Error => 0.0,
    }
}

/// Linear attack: 0 → peak. Measured at ~30 ms on the source recordings.
pub const ATTACK_SECONDS: f32 = 0.030;

/// Bluetooth A2DP wake-up pre-roll. The link sleeps; the first ~300 ms of a
/// cold stream is eaten priming it, which is heard as a clipped attack.
pub const PREROLL_SECONDS: f32 = 0.70;

/// Run-out after the last note so a Bluetooth sink's 100-300 ms of buffered
/// audio actually flushes before the stream ends. Without it the user hears the
/// ENDING clipped.
pub const FLUSH_TAIL_SECONDS: f32 = 1.10;

/// TPDF dither level, ~-57 dBFS (10^(-57/20) ≈ 0.0014 linear): real energy as
/// far as an audio stack is concerned, far below anything audible.
///
/// ⚠ THE DITHER MUST SPAN THE WHOLE RENDER, not just the pre-roll. An
/// aggressive A2DP sink sleeps on true digital silence and clips whatever wakes
/// it. This chime is mostly silence BY DURATION (1.03 s inside the pair, 2.4 s
/// between pairs), so a front-only pre-roll leaves every later note exposed.
/// Acceptance: ZERO 50 ms windows of digital silence across the whole buffer —
/// locked by `no_fifty_millisecond_window_of_the_chime_is_digitally_silent`.
pub const DITHER_PEAK_AMPLITUDE: f32 = 0.0014;

/// How long the tail-cut fade takes once the stretched envelope is cut.
pub const ENV_FADE_SECONDS: f32 = 0.060;

/// Render sample rate for the native path.
pub const SAMPLE_RATE_HZ: u32 = 48_000;

/// ⚠ MODEL CHANGE, not a constant swap. The pre-2026-07-27 envelope encoded Web
/// Audio semantics (linear attack, then exponential toward an absolute floor).
/// The approved sound has a SUSTAIN SHOULDER — still 89% of peak at 150 ms —
/// which no exponential reproduces, and that shoulder is the whole difference
/// between "cabin chime" and "beep". Measured from a real recording, 20 ms
/// grid, normalized to peak. Interpolate linearly; past the last point is
/// silence.
///
/// NOTE ON ITS LIMIT: the source recording's SECOND strike lands at 1.032 s and
/// masks the rest, so the true ring-down past ~1.03 s is UNOBSERVED, not known
/// to be zero. That is why [`tone_ring`] exists rather than a longer table.
pub const CABIN_ENV: &[(f32, f32)] = &[
    (0.000, 1.0000),
    (0.020, 0.9561),
    (0.040, 0.9561),
    (0.060, 0.9510),
    (0.080, 0.9510),
    (0.100, 0.9253),
    (0.120, 0.9253),
    (0.140, 0.8934),
    (0.160, 0.8745),
    (0.180, 0.8124),
    (0.200, 0.7831),
    (0.220, 0.7263),
    (0.240, 0.6762),
    (0.260, 0.6227),
    (0.280, 0.5657),
    (0.300, 0.5214),
    (0.320, 0.4643),
    (0.340, 0.4250),
    (0.360, 0.3659),
    (0.380, 0.3337),
    (0.400, 0.3010),
    (0.420, 0.2895),
    (0.440, 0.2572),
    (0.460, 0.2452),
    (0.480, 0.2137),
    (0.500, 0.2003),
    (0.520, 0.1758),
    (0.540, 0.1703),
    (0.560, 0.1514),
    (0.580, 0.1445),
    (0.600, 0.1271),
    (0.620, 0.1186),
    (0.640, 0.1028),
    (0.660, 0.0929),
    (0.680, 0.0786),
    (0.700, 0.0672),
    (0.720, 0.0574),
    (0.740, 0.0544),
    (0.760, 0.0510),
    (0.780, 0.0474),
    (0.800, 0.0445),
    (0.820, 0.0404),
    (0.840, 0.0379),
    (0.860, 0.0335),
    (0.880, 0.0311),
    (0.900, 0.0268),
    (0.920, 0.0243),
    (0.940, 0.0201),
    (0.960, 0.0175),
    (0.980, 0.0135),
    (1.000, 0.0088),
    (1.020, 0.0088),
    (1.050, 0.0000),
];

/// The measured envelope's own length, before any [`tone_ring`] stretch.
fn env_length_seconds() -> f32 {
    CABIN_ENV[CABIN_ENV.len() - 1].0
}

/// When the STRETCHED envelope first falls below `cut` (a fraction of peak).
/// `None` when the tone is not cut.
pub fn env_cut_time(cut: f32, ring: f32) -> Option<f32> {
    if cut <= 0.0 {
        return None;
    }
    CABIN_ENV
        .iter()
        .find(|(_, amplitude)| *amplitude < cut)
        .map(|(t, _)| t * ring)
}

/// The measured envelope at `t_after_attack` seconds — i.e. `t` counted from
/// the moment the linear attack reaches peak, normalized to peak.
///
/// `ring` time-stretches it (1.0 is exactly as measured, 1.8 rings ~80%
/// longer). `cut` ends it early with a short fade, killing the stretched tail.
pub fn env_at(t_after_attack: f32, ring: f32, cut: f32) -> f32 {
    if let Some(cut_t) = env_cut_time(cut, ring) {
        if t_after_attack >= cut_t + ENV_FADE_SECONDS {
            return 0.0;
        }
        if t_after_attack > cut_t {
            let at_cut = env_at(cut_t, ring, 0.0);
            return at_cut * (1.0 - (t_after_attack - cut_t) / ENV_FADE_SECONDS);
        }
    }
    let t = t_after_attack / ring;
    if t <= 0.0 {
        return 1.0;
    }
    if t >= env_length_seconds() {
        return 0.0;
    }
    let mut lo = 0usize;
    let mut hi = CABIN_ENV.len() - 1;
    while hi - lo > 1 {
        let mid = (lo + hi) / 2;
        if CABIN_ENV[mid].0 <= t {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    let (t0, a0) = CABIN_ENV[lo];
    let (t1, a1) = CABIN_ENV[hi];
    if t1 > t0 {
        a0 + (a1 - a0) * ((t - t0) / (t1 - t0))
    } else {
        a0
    }
}

/// The effective envelope as breakpoints `(secondsAfterAttack, gainFraction)`,
/// already stretched by `ring` and already tail-cut.
///
/// This is what makes the WEBVIEW a consumer of this module rather than a
/// second owner of the envelope: Web Audio cannot express a measured curve as
/// two ramps, so the script walks these points with `linearRampToValueAtTime`
/// and spells no numbers of its own. Linear interpolation across them
/// reproduces [`env_at`] exactly — locked by
/// `the_breakpoints_and_env_at_are_the_same_envelope`.
pub fn envelope_points(ring: f32, cut: f32) -> Vec<(f32, f32)> {
    let cut_t = env_cut_time(cut, ring);
    let mut points = Vec::with_capacity(CABIN_ENV.len() + 2);
    for (t, amplitude) in CABIN_ENV {
        let stretched = t * ring;
        if cut_t.is_some_and(|cut_t| stretched >= cut_t) {
            break;
        }
        points.push((stretched, *amplitude));
    }
    if let Some(cut_t) = cut_t {
        points.push((cut_t, env_at(cut_t, ring, 0.0)));
        points.push((cut_t + ENV_FADE_SECONDS, 0.0));
    }
    points
}

/// How long ONE note sounds, from the end of its attack to silence.
pub fn note_audible_seconds(ring: f32, cut: f32) -> f32 {
    let full = env_length_seconds() * ring;
    match env_cut_time(cut, ring) {
        Some(cut_t) => full.min(cut_t + ENV_FADE_SECONDS),
        None => full,
    }
}

/// The chime's own duration (last note start + attack + audible ring-down),
/// excluding any pre-roll and flush tail.
pub fn notes_duration_seconds(notes: &[ChimeNote], ring: f32, cut: f32) -> f32 {
    let last_start = notes.iter().map(|n| n.start_s).fold(0.0_f32, f32::max);
    last_start + ATTACK_SECONDS + note_audible_seconds(ring, cut)
}

/// The note table as the JSON array literal the webview script embeds.
pub fn notes_json(notes: &[ChimeNote]) -> String {
    let body = notes
        .iter()
        .map(|n| {
            format!(
                "[{},{},{}]",
                fmt_f32(n.start_s),
                fmt_f32(n.freq_hz),
                fmt_f32(n.peak)
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    format!("[{body}]")
}

/// The effective envelope as the JSON array literal the webview script embeds.
pub fn envelope_json(ring: f32, cut: f32) -> String {
    let body = envelope_points(ring, cut)
        .into_iter()
        .map(|(t, amplitude)| format!("[{},{}]", fmt_f32(t), fmt_f32(amplitude)))
        .collect::<Vec<_>>()
        .join(",");
    format!("[{body}]")
}

/// Shortest representation that still round-trips the constant, so the emitted
/// JS reads like a hand-written literal (`0.48`, not `0.479999989`).
fn fmt_f32(value: f32) -> String {
    let mut text = format!("{value}");
    if !text.contains('.') && !text.contains('e') {
        text.push_str(".0");
    }
    text
}

/// Parse the `[[startSec, freqHz, peak], …]` shape accepted by
/// `audio tune --notes` and used by the webview. Hand-rolled so this module
/// stays dependency-free and usable from every crate.
pub fn parse_notes_json(input: &str) -> Result<Vec<ChimeNote>, String> {
    let trimmed = input.trim();
    let inner = trimmed
        .strip_prefix('[')
        .and_then(|rest| rest.strip_suffix(']'))
        .ok_or_else(|| "notes must be a JSON array: [[startSec,freqHz,peak], …]".to_string())?;

    let mut notes = Vec::new();
    let mut depth = 0usize;
    let mut current = String::new();
    for ch in inner.chars() {
        match ch {
            '[' => {
                depth += 1;
                if depth == 1 {
                    current.clear();
                    continue;
                }
                return Err("notes may not nest deeper than one level".to_string());
            }
            ']' => {
                if depth == 0 {
                    return Err("unbalanced ] in notes".to_string());
                }
                depth -= 1;
                notes.push(parse_note_triple(&current)?);
                current.clear();
            }
            ',' if depth == 0 => {}
            _ if depth == 0 => {
                if !ch.is_whitespace() {
                    return Err(format!("unexpected {ch:?} between notes"));
                }
            }
            _ => current.push(ch),
        }
    }
    if depth != 0 {
        return Err("unbalanced [ in notes".to_string());
    }
    if notes.is_empty() {
        return Err("notes must contain at least one note".to_string());
    }
    Ok(notes)
}

fn parse_note_triple(body: &str) -> Result<ChimeNote, String> {
    let parts: Vec<&str> = body.split(',').map(str::trim).collect();
    if parts.len() != 3 {
        return Err(format!(
            "each note must be [startSec,freqHz,peak] — got {} value(s) in [{body}]",
            parts.len()
        ));
    }
    let parse = |raw: &str, what: &str| -> Result<f32, String> {
        raw.parse::<f32>()
            .map_err(|_| format!("{what} {raw:?} is not a number"))
    };
    let start_s = parse(parts[0], "note start")?;
    let freq_hz = parse(parts[1], "note frequency")?;
    let peak = parse(parts[2], "note peak")?;
    if !(start_s.is_finite() && freq_hz.is_finite() && peak.is_finite()) {
        return Err(format!("note [{body}] carries a non-finite value"));
    }
    if start_s < 0.0 {
        return Err(format!("note start {start_s} is negative"));
    }
    // A note above Nyquist would alias into an audible tone that is NOT the one
    // written down, which is the one failure a tune audition must never hide.
    if freq_hz <= 0.0 || freq_hz >= (SAMPLE_RATE_HZ as f32) / 2.0 {
        return Err(format!(
            "note frequency {freq_hz} Hz is outside (0, {}) — above Nyquist it would alias",
            (SAMPLE_RATE_HZ as f32) / 2.0
        ));
    }
    if !(0.0..=1.0).contains(&peak) {
        return Err(format!("note peak {peak} is outside 0..=1"));
    }
    Ok(ChimeNote {
        start_s,
        freq_hz,
        peak,
    })
}

/// Render the chime to a mono 16-bit PCM WAV.
///
/// `ring` and `cut` are the envelope shape (see [`tone_ring`] /
/// [`tone_tail_cut`]); a hand-written tune takes the envelope AS MEASURED
/// (`1.0`, `0.0`), because the per-tone stretch belongs to the measured tones.
/// `volume` scales the rendered signal (0..=1). `preroll_s` and `tail_s` are
/// taken as given so the caller can disable either; the defaults are
/// [`PREROLL_SECONDS`] and [`FLUSH_TAIL_SECONDS`].
pub fn render_wav_mono_s16le(
    notes: &[ChimeNote],
    ring: f32,
    cut: f32,
    preroll_s: f32,
    tail_s: f32,
    volume: f32,
    sample_rate: u32,
) -> Vec<u8> {
    let volume = volume.clamp(0.0, 1.0);
    let preroll_s = preroll_s.max(0.0);
    let tail_s = tail_s.max(0.0);
    let total_s = preroll_s + notes_duration_seconds(notes, ring, cut) + tail_s;
    let frames = ((sample_rate as f32) * total_s).ceil().max(1.0) as usize;

    // Bluetooth link keepalive across the WHOLE span, not just the front. An
    // aggressive A2DP sink sleeps on true digital silence and then clips the
    // attack of whatever wakes it. This chime is mostly silence BY DURATION
    // (1.03 s between hi and lo, 2.4 s between pairs), so a pre-roll alone
    // leaves every later note exposed. TPDF (triangular) dither — two uniform
    // draws summed, zero-centred — is below audibility but is not digital
    // silence, so the radio never lets go. Deterministic PRNG so a rendered
    // chime is byte-reproducible and therefore testable; the ear cannot tell
    // one noise seed from another.
    let mut rng = 0x2545_F491_4F6C_DD1D_u64;
    let mut buffer: Vec<f32> = (0..frames)
        .map(|_| {
            let a = next_unit_f32(&mut rng);
            let b = next_unit_f32(&mut rng);
            (a + b - 1.0) * DITHER_PEAK_AMPLITUDE * volume
        })
        .collect();

    let attack_frames = ((sample_rate as f32) * ATTACK_SECONDS).max(1.0) as usize;
    let note_frames =
        attack_frames + ((sample_rate as f32) * note_audible_seconds(ring, cut)) as usize;
    for n in notes {
        let start_frame = ((sample_rate as f32) * (preroll_s + n.start_s)) as usize;
        for i in 0..note_frames {
            let idx = start_frame + i;
            if idx >= frames {
                break;
            }
            let t = i as f32 / sample_rate as f32;
            // Volume scales the envelope's OUTPUT, never the peak fed into it.
            // With a normalized measured table the two are equivalent, but the
            // rule is kept because it is the one that stays true if the shape
            // ever changes again: caught red on the exponential envelope this
            // replaced, where half volume produced 52% of the peak, not 50%.
            let shape = if i < attack_frames {
                i as f32 / attack_frames as f32
            } else {
                env_at(t - ATTACK_SECONDS, ring, cut)
            };
            let env = n.peak * shape * volume;
            buffer[idx] += env * (std::f32::consts::TAU * n.freq_hz * t).sin();
        }
    }

    let mut pcm = Vec::with_capacity(frames * 2);
    for sample in &buffer {
        let clamped = (sample * 32767.0).clamp(-32767.0, 32767.0) as i16;
        pcm.extend_from_slice(&clamped.to_le_bytes());
    }
    wav_container(&pcm, sample_rate)
}

/// xorshift64* — a deterministic PRNG, so a rendered chime is reproducible.
fn next_unit_f32(state: &mut u64) -> f32 {
    let mut x = *state;
    x ^= x >> 12;
    x ^= x << 25;
    x ^= x >> 27;
    *state = x;
    let value = x.wrapping_mul(0x2545_F491_4F6C_DD1D);
    ((value >> 40) as f32) / ((1u32 << 24) as f32)
}

fn wav_container(pcm: &[u8], sample_rate: u32) -> Vec<u8> {
    let channels: u16 = 1;
    let bits: u16 = 16;
    let byte_rate = sample_rate * u32::from(channels) * u32::from(bits / 8);
    let block_align = channels * (bits / 8);

    let mut out = Vec::with_capacity(44 + pcm.len());
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&((36 + pcm.len()) as u32).to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&channels.to_le_bytes());
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&byte_rate.to_le_bytes());
    out.extend_from_slice(&block_align.to_le_bytes());
    out.extend_from_slice(&bits.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(pcm.len() as u32).to_le_bytes());
    out.extend_from_slice(pcm);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn peak_amplitude(wav: &[u8]) -> i16 {
        wav[44..]
            .chunks_exact(2)
            .map(|c| i16::from_le_bytes([c[0], c[1]]).saturating_abs())
            .max()
            .unwrap_or(0)
    }

    fn samples(wav: &[u8]) -> Vec<i16> {
        wav[44..]
            .chunks_exact(2)
            .map(|c| i16::from_le_bytes([c[0], c[1]]))
            .collect()
    }

    fn render_tone(tone: ChimeTone, preroll_s: f32, tail_s: f32, volume: f32) -> Vec<u8> {
        render_wav_mono_s16le(
            tone_notes(tone),
            tone_ring(tone),
            tone_tail_cut(tone),
            preroll_s,
            tail_s,
            volume,
            SAMPLE_RATE_HZ,
        )
    }

    /// The envelope read back off the RENDERED PCM at `seconds` into the file:
    /// the loudest sample in a one-period window around that offset, which is
    /// what a listener's ear integrates. This is how the port is verified
    /// against CABIN_ENV — from the samples, not from the model that made them.
    fn rendered_envelope_at(wav: &[u8], seconds: f32) -> f32 {
        let all = samples(wav);
        let centre = (seconds * SAMPLE_RATE_HZ as f32) as usize;
        // One full period of the lowest note (493.88 Hz ⇒ ~97 frames) around
        // the sample point, so the peak is the envelope regardless of phase.
        let half = 50usize;
        let lo = centre.saturating_sub(half);
        let hi = (centre + half).min(all.len());
        let peak = all[lo..hi]
            .iter()
            .map(|s| s.unsigned_abs() as f32)
            .fold(0.0_f32, f32::max);
        peak / 32767.0
    }

    #[test]
    fn every_tone_has_notes_and_a_stable_key() {
        for tone in ChimeTone::ALL {
            let notes = tone_notes(*tone);
            assert!(!notes.is_empty(), "{tone:?} has no notes");
            assert_eq!(
                ChimeTone::parse(tone.as_key()),
                Some(*tone),
                "{tone:?} key does not round-trip",
            );
        }
    }

    #[test]
    fn the_three_cabin_patterns_are_tellable_apart() {
        // Meaning is carried by PATTERN, as in a real cabin: one chime =
        // passenger call, hi-lo = crew interphone, hi-lo ×3 = emergency. Info
        // and Success SHARE the single deliberately; the three patterns must
        // stay distinct or the user hears "something happened" with no idea
        // what.
        assert_eq!(tone_notes(ChimeTone::Info).len(), 1, "one chime");
        assert_eq!(tone_notes(ChimeTone::Success).len(), 1, "one chime");
        assert_eq!(tone_notes(ChimeTone::Warning).len(), 2, "hi-lo");
        assert_eq!(tone_notes(ChimeTone::Error).len(), 6, "hi-lo ×3");
        assert_ne!(
            tone_notes(ChimeTone::Info),
            tone_notes(ChimeTone::Warning),
            "the single and the pair must not collapse into one pattern",
        );
        assert_ne!(
            tone_notes(ChimeTone::Warning),
            tone_notes(ChimeTone::Error),
            "the pair and the emergency triple must not collapse into one pattern",
        );
        // The pair is a DESCENDING MINOR THIRD, measured on all three source
        // recordings — not the perfect fourth this tune shipped with before.
        let pair = tone_notes(ChimeTone::Warning);
        assert!(pair[0].freq_hz > pair[1].freq_hz, "hi then lo");
        let ratio = pair[0].freq_hz / pair[1].freq_hz;
        assert!(
            (ratio - 2.0_f32.powf(3.0 / 12.0)).abs() < 0.005,
            "the drop must be a minor third (3 semitones), got ratio {ratio}",
        );
        // The slow tempo is most of what makes this calm rather than urgent.
        assert!(
            (pair[1].start_s - 1.03).abs() < 1e-6,
            "the measured hi→lo gap is 1.03 s; anything near 0.2 reads as rushed",
        );
    }

    #[test]
    fn the_error_pattern_is_the_warning_pair_three_times() {
        let pair = tone_notes(ChimeTone::Warning);
        let error = tone_notes(ChimeTone::Error);
        assert_eq!(error.len(), pair.len() * 3);
        for repeat in 0..3 {
            for (index, note) in pair.iter().enumerate() {
                let played = error[repeat * pair.len() + index];
                assert_eq!(played.freq_hz, note.freq_hz);
                assert_eq!(played.peak, note.peak);
                let expected = note.start_s + repeat as f32 * 2.40;
                assert!(
                    (played.start_s - expected).abs() < 1e-5,
                    "error note {repeat}/{index} starts at {} not {expected}",
                    played.start_s,
                );
            }
        }
    }

    #[test]
    fn notes_json_round_trips_through_the_parser() {
        // The webview embeds `notes_json`; `audio tune --notes` consumes the
        // same shape. If these two ever disagree, a tune auditioned on the CLI
        // would not be the tune the GUI plays.
        for tone in ChimeTone::ALL {
            let json = notes_json(tone_notes(*tone));
            let parsed = parse_notes_json(&json)
                .unwrap_or_else(|err| panic!("{tone:?} json {json} did not parse: {err}"));
            assert_eq!(parsed, tone_notes(*tone), "{tone:?} did not round-trip");
        }
    }

    #[test]
    fn notes_json_reads_like_a_hand_written_literal() {
        assert_eq!(
            notes_json(tone_notes(ChimeTone::Info)),
            "[[0.0,587.33,0.48]]"
        );
        assert_eq!(
            notes_json(tone_notes(ChimeTone::Warning)),
            "[[0.0,587.33,0.4],[1.03,493.88,0.4]]",
        );
    }

    #[test]
    fn the_envelope_has_the_sustain_shoulder_an_exponential_cannot_make() {
        // The whole reason CABIN_ENV is a measured table: the approved sound
        // still sits at ~89% of peak 150 ms in, and holds a shoulder through
        // the first ~100 ms. An exponential decay is already far below that,
        // and that difference is "cabin chime" vs "beep".
        assert!(
            env_at(0.150, 1.0, 0.0) > 0.85,
            "the shoulder is gone: {} at 150 ms",
            env_at(0.150, 1.0, 0.0),
        );
        assert!(env_at(0.100, 1.0, 0.0) > 0.90, "shoulder through 100 ms");
        // Ring-down milestones from the measurement.
        assert!(
            (env_at(0.305, 1.0, 0.0) - 0.5).abs() < 0.05,
            "50% should land near 305 ms, got {}",
            env_at(0.305, 1.0, 0.0),
        );
        assert!(
            (env_at(0.655, 1.0, 0.0) - 0.1).abs() < 0.03,
            "10% should land near 655 ms, got {}",
            env_at(0.655, 1.0, 0.0),
        );
        assert_eq!(env_at(0.0, 1.0, 0.0), 1.0, "the attack ends AT peak");
        assert_eq!(env_at(1.05, 1.0, 0.0), 0.0, "past the table is silence");
        // Monotonically non-increasing: a measured table that rose again would
        // be a transcription error, not a chime.
        let mut previous = 1.0_f32;
        let mut t = 0.0_f32;
        while t < 1.06 {
            let value = env_at(t, 1.0, 0.0);
            assert!(value <= previous + 1e-6, "envelope rose again at t={t}");
            previous = value;
            t += 0.005;
        }
    }

    #[test]
    fn the_ring_stretches_the_body_and_the_tail_cut_removes_the_aftertaste() {
        let ring = tone_ring(ChimeTone::Success);
        let cut = tone_tail_cut(ChimeTone::Success);
        assert!(ring > 1.0, "the single note is stretched");
        assert!(cut > 0.0, "a stretched tone MUST be tail-cut");
        assert_eq!(
            tone_ring(ChimeTone::Warning),
            1.0,
            "the pair is as measured"
        );
        assert_eq!(
            tone_tail_cut(ChimeTone::Warning),
            0.0,
            "an unstretched tone needs no cut",
        );

        // The BODY is stretched: what the measured envelope reaches at 300 ms
        // the stretched one reaches at 300 ms × ring.
        let measured = env_at(0.300, 1.0, 0.0);
        let stretched = env_at(0.300 * ring, ring, cut);
        assert!(
            (measured - stretched).abs() < 1e-5,
            "the ring must stretch the body: {measured} vs {stretched}",
        );

        // The TAIL is not. Without the cut the stretched envelope rings on
        // audibly past the cut point — that lingering is the "aftertaste" the
        // user rejected.
        let cut_t = env_cut_time(cut, ring).expect("a cut tone has a cut time");
        assert!(
            env_at(cut_t + ENV_FADE_SECONDS, ring, cut) == 0.0,
            "the cut must reach silence one fade after the cut point",
        );
        assert!(
            env_at(cut_t + ENV_FADE_SECONDS, ring, 0.0) > 0.0,
            "without the cut the stretched tail would still be ringing here — \
             that is the aftertaste this cut exists to remove",
        );
        assert!(
            note_audible_seconds(ring, cut) < note_audible_seconds(ring, 0.0),
            "the cut must actually shorten the note",
        );
    }

    #[test]
    fn the_breakpoints_and_env_at_are_the_same_envelope() {
        // The webview walks `envelope_points`; the native renderer calls
        // `env_at`. If these two could disagree the tune would have two owners
        // again, one per player, which is the whole failure this module exists
        // to prevent.
        for tone in ChimeTone::ALL {
            let (ring, cut) = (tone_ring(*tone), tone_tail_cut(*tone));
            let points = envelope_points(ring, cut);
            assert!(!points.is_empty(), "{tone:?} has no envelope");
            assert_eq!(points[0], (0.0, 1.0), "{tone:?} must start at peak");
            let (last_t, last_a) = points[points.len() - 1];
            assert_eq!(last_a, 0.0, "{tone:?} must end silent");
            assert!(
                (last_t - note_audible_seconds(ring, cut)).abs() < 1e-5,
                "{tone:?}: the breakpoints must span the audible length",
            );

            let mut t = 0.0_f32;
            while t <= last_t + 0.05 {
                let interpolated = interpolate(&points, t);
                let direct = env_at(t, ring, cut);
                assert!(
                    (interpolated - direct).abs() < 1e-5,
                    "{tone:?} at t={t}: breakpoints say {interpolated}, env_at says {direct}",
                );
                t += 0.002;
            }
        }
    }

    fn interpolate(points: &[(f32, f32)], t: f32) -> f32 {
        if t <= points[0].0 {
            return points[0].1;
        }
        for window in points.windows(2) {
            let (t0, a0) = window[0];
            let (t1, a1) = window[1];
            if t <= t1 {
                return a0 + (a1 - a0) * ((t - t0) / (t1 - t0));
            }
        }
        0.0
    }

    #[test]
    fn rendered_wav_is_a_wellformed_mono_s16_container() {
        let wav = render_tone(ChimeTone::Success, PREROLL_SECONDS, FLUSH_TAIL_SECONDS, 1.0);
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
        assert_eq!(&wav[36..40], b"data");
        assert_eq!(
            u32::from_le_bytes([wav[4], wav[5], wav[6], wav[7]]) as usize,
            wav.len() - 8,
            "RIFF size must describe the actual file",
        );
        assert_eq!(
            u32::from_le_bytes([wav[40], wav[41], wav[42], wav[43]]) as usize,
            wav.len() - 44,
            "data size must describe the actual payload",
        );
        assert_eq!(u16::from_le_bytes([wav[22], wav[23]]), 1, "mono");
        assert_eq!(
            u32::from_le_bytes([wav[24], wav[25], wav[26], wav[27]]),
            SAMPLE_RATE_HZ,
        );
        assert_eq!(u16::from_le_bytes([wav[34], wav[35]]), 16, "16-bit");
    }

    #[test]
    fn rendered_chime_actually_contains_audio() {
        // The whole defect class this module answers is "the call succeeded and
        // nothing was heard". A renderer that emits silence would reproduce it
        // natively, so assert on the samples.
        let wav = render_tone(ChimeTone::Success, PREROLL_SECONDS, FLUSH_TAIL_SECONDS, 1.0);
        assert!(
            peak_amplitude(&wav) > 1000,
            "rendered chime is effectively silent (peak {})",
            peak_amplitude(&wav),
        );
    }

    #[test]
    fn the_rendered_pcm_follows_the_measured_envelope() {
        // The port's acceptance test: read the ENVELOPE back off the rendered
        // samples at known offsets and compare it against CABIN_ENV. A renderer
        // that kept the old exponential would pass every structural test above
        // and fail here at the sustain shoulder.
        let wav = render_tone(ChimeTone::Warning, 0.0, 0.0, 1.0);
        let peak = tone_notes(ChimeTone::Warning)[0].peak;
        for (grid_t, expected) in [
            (0.020_f32, 0.9561_f32),
            (0.100, 0.9253),
            (0.150, 0.88395), // between the 0.140 and 0.160 grid points
            (0.300, 0.5214),
            (0.500, 0.2003),
            (0.700, 0.0672),
        ] {
            let measured = rendered_envelope_at(&wav, ATTACK_SECONDS + grid_t) / peak;
            assert!(
                (measured - expected).abs() < 0.02,
                "at {grid_t}s the rendered envelope is {measured}, CABIN_ENV says {expected}",
            );
        }
        // …and the shoulder specifically: an exponential from peak to 0.0008
        // over 1.15 s (the model this replaced) is at ~0.33 of peak by 150 ms.
        let shoulder = rendered_envelope_at(&wav, ATTACK_SECONDS + 0.150) / peak;
        assert!(
            shoulder > 0.80,
            "the sustain shoulder is missing ({shoulder} at 150 ms) — this is an \
             exponential decay, not the measured cabin envelope",
        );
    }

    #[test]
    fn the_stretched_single_rings_longer_and_still_ends() {
        let single = render_tone(ChimeTone::Info, 0.0, FLUSH_TAIL_SECONDS, 1.0);
        let pair = render_tone(ChimeTone::Warning, 0.0, FLUSH_TAIL_SECONDS, 1.0);
        // Half a second in, the STRETCHED envelope is still where the measured
        // one was at 0.5/1.8 ≈ 0.28 s. That is what "rings longer" means, read
        // off the samples rather than off the model.
        let stretched = rendered_envelope_at(&single, ATTACK_SECONDS + 0.5)
            / tone_notes(ChimeTone::Info)[0].peak;
        let measured = rendered_envelope_at(&pair, ATTACK_SECONDS + 0.5)
            / tone_notes(ChimeTone::Warning)[0].peak;
        assert!(
            stretched > measured * 1.5,
            "the single must ring longer than the measured pair: {stretched} vs {measured}",
        );

        // And it still ENDS. A stretched envelope also stretches its quiet
        // tail, and that lingering is the "aftertaste" the user rejected —
        // `tone_tail_cut` is what removes it, so the render must be silent
        // (bar the keepalive dither) the moment the envelope is over.
        let all = samples(&single);
        // An ABSOLUTE deadline, not one derived from the envelope functions
        // under test: the uncut stretched envelope runs 1.05 × 1.8 = 1.89 s and
        // is still at ~4% of peak here, which is audible ringing. The cut ends
        // it before 1.5 s.
        let deadline = (1.5 * SAMPLE_RATE_HZ as f32) as usize;
        assert!(
            all.len() > deadline,
            "the render must outlast the deadline or this proves nothing",
        );
        assert!(
            all[deadline..].iter().all(|s| s.unsigned_abs() < 200),
            "the single note is still ringing 1.5 s in — the aftertaste is back",
        );
        // …and it stops exactly where the envelope says it does.
        let audible_end = ((ATTACK_SECONDS
            + note_audible_seconds(tone_ring(ChimeTone::Info), tone_tail_cut(ChimeTone::Info)))
            * SAMPLE_RATE_HZ as f32) as usize;
        assert!(
            all[audible_end..].iter().all(|s| s.unsigned_abs() < 200),
            "the single note is still ringing after its own envelope ended",
        );
    }

    #[test]
    fn the_flush_tail_carries_dither_but_no_chime() {
        // The reported symptom was a CLIPPED ENDING: a Bluetooth sink holds
        // 100-300 ms of buffered audio, so a stream that ends at the last note
        // loses it. The tail must exist and must not be chime — but it is NOT
        // digital silence either, or the link lets go during it.
        let wav = render_tone(ChimeTone::Info, 0.0, FLUSH_TAIL_SECONDS, 1.0);
        let all = samples(&wav);
        let tail_frames = (SAMPLE_RATE_HZ as f32 * FLUSH_TAIL_SECONDS) as usize;
        assert!(
            all.len() > tail_frames,
            "rendered chime is shorter than its own flush tail",
        );
        let tail = &all[all.len() - tail_frames..];
        assert!(
            tail.iter().all(|s| s.unsigned_abs() < 200),
            "the flush tail must not be more chime",
        );
        assert!(
            tail.iter().any(|s| *s != 0),
            "the flush tail must still carry the keepalive dither",
        );
        let body = &all[..all.len() - tail_frames];
        assert!(
            body.iter().any(|s| s.unsigned_abs() > 1000),
            "everything before the tail should be the chime",
        );
    }

    #[test]
    fn the_preroll_is_quiet_noise_and_not_silence() {
        // Several Bluetooth stacks DROP silent frames and never prime the link,
        // which is why the pre-roll is dither noise. A pre-roll that rendered as
        // digital silence would look correct and defeat its own purpose.
        let wav = render_tone(ChimeTone::Info, PREROLL_SECONDS, 0.0, 1.0);
        let all = samples(&wav);
        let preroll_frames = (SAMPLE_RATE_HZ as f32 * PREROLL_SECONDS) as usize;
        let preroll = &all[..preroll_frames];
        assert!(
            preroll.iter().any(|s| *s != 0),
            "the pre-roll rendered as digital silence — the A2DP link will not prime",
        );
        let loudest = preroll.iter().map(|s| s.unsigned_abs()).max().unwrap_or(0);
        assert!(
            loudest < 500,
            "the pre-roll must stay at dither level, got {loudest}",
        );
    }

    #[test]
    fn no_fifty_millisecond_window_of_the_chime_is_digitally_silent() {
        // ⚠ The trap the whole-render dither exists for. The error tone is
        // mostly silence by duration (1.03 s inside a pair, 2.4 s between
        // pairs); a front-only pre-roll primes the link and then lets it sleep
        // again before the second pair, which clips every later strike. Every
        // tone is checked, because the tone that regresses will be the one
        // nobody thought to check.
        let window = (SAMPLE_RATE_HZ as f32 * 0.050) as usize;
        for tone in ChimeTone::ALL {
            let wav = render_tone(*tone, PREROLL_SECONDS, FLUSH_TAIL_SECONDS, 1.0);
            let all = samples(&wav);
            let mut run = 0usize;
            let mut longest = 0usize;
            for sample in &all {
                if *sample == 0 {
                    run += 1;
                    longest = longest.max(run);
                } else {
                    run = 0;
                }
            }
            assert!(
                longest < window,
                "{tone:?}: {longest} consecutive silent frames ({:.0} ms) — an A2DP \
                 sink sleeps on digital silence and clips whatever wakes it",
                longest as f32 * 1000.0 / SAMPLE_RATE_HZ as f32,
            );
        }
    }

    #[test]
    fn volume_scales_the_chime_and_zero_is_silent() {
        let full = render_tone(ChimeTone::Success, 0.0, 0.0, 1.0);
        let half = render_tone(ChimeTone::Success, 0.0, 0.0, 0.5);
        let silent = render_tone(ChimeTone::Success, 0.0, 0.0, 0.0);
        let (f, h) = (peak_amplitude(&full) as f32, peak_amplitude(&half) as f32);
        assert!(
            (h / f - 0.5).abs() < 0.02,
            "half volume should halve the peak: {h} vs {f}",
        );
        // Shape-preserving, not just peak-preserving: sample the envelope at
        // two points and check the RATIO is the same at both volumes. The
        // exponential model this replaced failed exactly here.
        for t in [0.100_f32, 0.400] {
            let a = rendered_envelope_at(&full, ATTACK_SECONDS + t);
            let b = rendered_envelope_at(&half, ATTACK_SECONDS + t);
            assert!(
                (b / a - 0.5).abs() < 0.03,
                "at {t}s half volume gave {} of full, not half",
                b / a,
            );
        }
        assert_eq!(peak_amplitude(&silent), 0, "volume 0 must be silent");
    }

    #[test]
    fn rendering_is_deterministic() {
        // Including the dither: a reproducible render is what lets a test
        // assert on bytes at all.
        let render = || render_tone(ChimeTone::Warning, PREROLL_SECONDS, FLUSH_TAIL_SECONDS, 1.0);
        assert_eq!(render(), render());
    }

    #[test]
    fn parser_refuses_the_shapes_that_would_play_the_wrong_thing() {
        for (input, why) in [
            ("", "not an array"),
            ("[]", "no notes"),
            ("[[0.0,440.0]]", "two values, not three"),
            ("[[0.0,440.0,0.07,9]]", "four values"),
            ("[[-1.0,440.0,0.07]]", "negative start"),
            ("[[0.0,0.0,0.07]]", "zero frequency"),
            ("[[0.0,40000.0,0.07]]", "above Nyquist, would alias"),
            ("[[0.0,440.0,2.0]]", "peak above 1"),
            ("[[0.0,440.0,-0.1]]", "negative peak"),
            ("[[0.0,abc,0.07]]", "not a number"),
        ] {
            assert!(
                parse_notes_json(input).is_err(),
                "parser accepted {input:?} which is {why}",
            );
        }
    }

    #[test]
    fn parser_accepts_a_hand_written_tune() {
        let notes = parse_notes_json("[[0, 880, 0.3], [0.2, 660, 0.3]]").expect("valid tune");
        assert_eq!(notes, vec![note(0.0, 880.0, 0.3), note(0.2, 660.0, 0.3)]);
    }

    #[test]
    fn duration_covers_the_last_note_in_full() {
        for tone in ChimeTone::ALL {
            let notes = tone_notes(*tone);
            let (ring, cut) = (tone_ring(*tone), tone_tail_cut(*tone));
            let last_start = notes.iter().map(|n| n.start_s).fold(0.0_f32, f32::max);
            let duration = notes_duration_seconds(notes, ring, cut);
            assert!(
                duration >= last_start + ATTACK_SECONDS + note_audible_seconds(ring, cut) - 1e-6,
                "{tone:?}: a chime must not end before its last note has sounded",
            );
            // …and the render is that long, plus pre-roll and tail.
            let wav = render_tone(*tone, PREROLL_SECONDS, FLUSH_TAIL_SECONDS, 1.0);
            let frames = (wav.len() - 44) / 2;
            let expected = ((PREROLL_SECONDS + duration + FLUSH_TAIL_SECONDS)
                * SAMPLE_RATE_HZ as f32) as usize;
            assert!(
                (frames as i64 - expected as i64).abs() <= 2,
                "{tone:?}: rendered {frames} frames, expected ~{expected}",
            );
        }
    }
}
