//! `yggterm server app audio` — the NATIVE notification audio path.
//!
//! **Why this is not `document::eval` into the shell webview.** Measured on the
//! GUI host 2026-07-26 with the user away from the keyboard: an injected chime
//! had `ctx.resume()` resolve, the PipeWire sink go SUSPENDED → RUNNING, the
//! sink-input present on the right sink at 100%, unmuted and uncorked — and
//! produced **complete silence**, while a system tone through the same speaker
//! seconds later was clearly audible. The remaining explanation is WebKitGTK's
//! autoplay gate: without a real user gesture the context streams SILENT
//! samples. An agent cannot synthesize a qualifying gesture, so the webview can
//! never satisfy the stated use case ("invoke a notification when you want my
//! attention"). A native A/B on the live host confirmed it: native PCM through
//! the platform sink is AUDIBLE where the webview path is silent.
//!
//! So this renders the chime to PCM in Rust and hands it to the platform sink.
//! No webview, no GUI, no daemon — it works on a headless host with no app
//! running at all.
//!
//! **The tune is not defined here.** `yggterm_core::notification_audio` owns
//! it, and the GUI's webview script reads the same constants, so the two
//! players cannot drift into two different chimes.

use std::io::Write;
use std::process::{Command, Stdio};

use anyhow::{Context, Result, anyhow, bail};
use yggterm_core::notification_audio::{
    self, ChimeTone, FLUSH_TAIL_SECONDS, PREROLL_SECONDS, SAMPLE_RATE_HZ,
};

/// The subcommands `audio` answers, in the order the help prints them.
const AUDIO_SUBCOMMANDS: &[&str] = &["play", "tune"];

/// Options each subcommand accepts, and nothing else.
///
/// ⚠ An unrecognised token must never be IGNORED here. `--voluem 0.2` playing
/// at full volume, or a bare `--tone` playing the default tone, is a silent
/// wrong-value in the one lane whose entire thesis is "never a silent
/// success": the user asked for one thing, heard another, and nothing said so.
const PLAY_OPTIONS: &[&str] = &["--tone", "--repeat", "--gap-ms", "--preroll", "--volume"];
const TUNE_OPTIONS: &[&str] = &["--notes", "--repeat", "--gap-ms", "--preroll", "--volume"];

/// Whether to prepend the A2DP wake-up pre-roll.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrerollMode {
    On,
    Off,
    /// Default: pre-roll unless a chime played very recently, matching the
    /// GUI's link-awake heuristic in spirit. Without shared state across CLI
    /// invocations there is nothing to remember, so `auto` resolves to `on` —
    /// a wasted pre-roll beats a clipped alert.
    Auto,
}

impl PrerollMode {
    fn parse(value: &str) -> Result<PrerollMode> {
        match value.trim().to_ascii_lowercase().as_str() {
            "on" | "true" | "1" => Ok(PrerollMode::On),
            "off" | "false" | "0" => Ok(PrerollMode::Off),
            "auto" => Ok(PrerollMode::Auto),
            other => bail!("--preroll must be on|off|auto, got {other:?}"),
        }
    }

    fn seconds(self) -> f32 {
        match self {
            PrerollMode::Off => 0.0,
            PrerollMode::On | PrerollMode::Auto => PREROLL_SECONDS,
        }
    }
}

/// A resolved player: the binary plus the argv that makes it read a WAV on
/// stdin. Piping avoids a temp file, so nothing is left behind if we are killed
/// mid-chime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioPlayer {
    pub binary: &'static str,
    pub args: &'static [&'static str],
}

/// Candidates in preference order: PipeWire first, then PulseAudio, then bare
/// ALSA as the last resort.
pub const PLAYER_CANDIDATES: &[AudioPlayer] = &[
    AudioPlayer {
        binary: "pw-play",
        args: &["-"],
    },
    AudioPlayer {
        binary: "paplay",
        args: &[],
    },
    AudioPlayer {
        binary: "aplay",
        args: &["-q", "-"],
    },
];

/// Find a usable player by looking it up on PATH.
///
/// Deliberately NOT `<binary> --version`: some of these open the audio device
/// on startup, and a probe that makes noise or blocks is worse than no probe.
pub fn resolve_player() -> Option<&'static AudioPlayer> {
    PLAYER_CANDIDATES
        .iter()
        .find(|player| binary_on_path(player.binary))
}

fn binary_on_path(binary: &str) -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path).any(|dir| {
        let candidate = dir.join(binary);
        candidate.is_file()
    })
}

/// One chime request, fully resolved.
#[derive(Debug, Clone, PartialEq)]
pub struct AudioPlayRequest {
    pub notes: Vec<notification_audio::ChimeNote>,
    /// What to call this in output — a tone name, or "custom" for `tune`.
    pub label: String,
    /// Envelope time-stretch and tail-cut. A registry tone brings its own; a
    /// hand-written `--notes` tune is played with the envelope AS MEASURED,
    /// because the per-tone stretch belongs to the measured tones.
    pub ring: f32,
    pub tail_cut: f32,
    pub repeat: u32,
    pub gap_ms: u64,
    pub preroll: PrerollMode,
    pub volume: f32,
}

impl AudioPlayRequest {
    pub fn render(&self) -> Vec<u8> {
        notification_audio::render_wav_mono_s16le(
            &self.notes,
            self.ring,
            self.tail_cut,
            self.preroll.seconds(),
            FLUSH_TAIL_SECONDS,
            self.volume,
            SAMPLE_RATE_HZ,
        )
    }
}

/// Every option, paired with its value, with NOTHING dropped on the floor.
///
/// Rejecting is the whole point: a typo'd flag or a flag whose value is missing
/// must be an error, never a default quietly substituted for what the user
/// asked for.
fn parse_options<'a>(
    args: &'a [String],
    subcommand: &str,
    allowed: &[&str],
) -> Result<Vec<(&'a str, &'a str)>> {
    let mut options: Vec<(&str, &str)> = Vec::new();
    let mut index = 0usize;
    while index < args.len() {
        let token = args[index].as_str();
        if !allowed.contains(&token) {
            bail!(
                "unknown option {token:?} for `audio {subcommand}` (accepted: {})",
                allowed.join(" "),
            );
        }
        let Some(value) = args.get(index + 1).map(String::as_str) else {
            bail!("{token} needs a value — it was given as the last word of the command");
        };
        if value.starts_with("--") {
            bail!("{token} needs a value, but the next word is the option {value:?}");
        }
        if let Some((_, first)) = options.iter().find(|(name, _)| *name == token) {
            bail!("{token} was given twice ({first:?} then {value:?}) — say which one you meant");
        }
        options.push((token, value));
        index += 2;
    }
    Ok(options)
}

fn option_value<'a>(options: &[(&'a str, &'a str)], flag: &str) -> Option<&'a str> {
    options
        .iter()
        .find(|(name, _)| *name == flag)
        .map(|(_, value)| *value)
}

/// Parse `audio play` / `audio tune` argv into a request.
///
/// Separated from execution so every parse rule is unit-testable on a host with
/// no speaker.
pub fn parse_play_request(args: &[String]) -> Result<AudioPlayRequest> {
    // argv is `server app audio <subcommand> …`, so the subcommand is index 3
    // and its options start at index 4.
    let subcommand = args.get(3).map(String::as_str).unwrap_or("");
    if !AUDIO_SUBCOMMANDS.contains(&subcommand) {
        bail!(
            "audio needs a subcommand ({}), got {subcommand:?}",
            AUDIO_SUBCOMMANDS.join("|"),
        );
    }
    let rest = args.get(4..).unwrap_or(&[]);
    let allowed = if subcommand == "tune" {
        TUNE_OPTIONS
    } else {
        PLAY_OPTIONS
    };
    let options = parse_options(rest, subcommand, allowed)?;

    let (notes, label, ring, tail_cut) = if subcommand == "tune" {
        let raw = option_value(&options, "--notes")
            .ok_or_else(|| anyhow!("audio tune needs --notes '[[startSec,freqHz,peak], …]'"))?;
        let notes = notification_audio::parse_notes_json(raw).map_err(|err| anyhow!(err))?;
        (notes, "custom".to_string(), 1.0_f32, 0.0_f32)
    } else {
        let tone_name = option_value(&options, "--tone").unwrap_or("success");
        let tone = ChimeTone::parse(tone_name).ok_or_else(|| {
            anyhow!("--tone must be one of info|success|warning|error, got {tone_name:?}")
        })?;
        (
            notification_audio::tone_notes(tone).to_vec(),
            tone.as_key().to_string(),
            notification_audio::tone_ring(tone),
            notification_audio::tone_tail_cut(tone),
        )
    };

    let repeat = match option_value(&options, "--repeat") {
        Some(raw) => raw
            .parse::<u32>()
            .with_context(|| format!("--repeat {raw:?} is not a whole number"))?,
        None => 1,
    };
    if repeat == 0 {
        bail!("--repeat must be at least 1");
    }
    let gap_ms = match option_value(&options, "--gap-ms") {
        Some(raw) => raw
            .parse::<u64>()
            .with_context(|| format!("--gap-ms {raw:?} is not a whole number"))?,
        None => 3_000,
    };
    let preroll = match option_value(&options, "--preroll") {
        Some(raw) => PrerollMode::parse(raw)?,
        None => PrerollMode::Auto,
    };
    let volume = match option_value(&options, "--volume") {
        Some(raw) => {
            let value: f32 = raw
                .parse()
                .with_context(|| format!("--volume {raw:?} is not a number"))?;
            if !(0.0..=1.0).contains(&value) {
                bail!("--volume must be within 0..=1, got {value}");
            }
            value
        }
        None => 1.0,
    };

    Ok(AudioPlayRequest {
        notes,
        label,
        ring,
        tail_cut,
        repeat,
        gap_ms,
        preroll,
        volume,
    })
}

/// Play one rendered WAV by piping it to the resolved player.
fn play_once(player: &AudioPlayer, wav: &[u8]) -> Result<()> {
    let mut child = Command::new(player.binary)
        .args(player.args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("failed to start {}", player.binary))?;
    child
        .stdin
        .as_mut()
        .ok_or_else(|| anyhow!("{} took no stdin", player.binary))?
        .write_all(wav)
        .with_context(|| format!("failed to pipe audio to {}", player.binary))?;
    // Dropping stdin signals end-of-stream; without it the player waits forever.
    drop(child.stdin.take());
    let output = child
        .wait_with_output()
        .with_context(|| format!("{} did not exit cleanly", player.binary))?;
    if !output.status.success() {
        bail!(
            "{} exited with {}: {}",
            player.binary,
            output.status,
            String::from_utf8_lossy(&output.stderr).trim(),
        );
    }
    Ok(())
}

/// `yggterm server app audio <play|tune> …`
pub fn run_audio_command(args: &[String]) -> Result<()> {
    let subcommand = args.get(3).map(String::as_str).unwrap_or("");
    if matches!(subcommand, "" | "--help" | "-h" | "help") {
        print_audio_help();
        return Ok(());
    }
    if !AUDIO_SUBCOMMANDS.contains(&subcommand) {
        // `state` reads the SHELL WEBVIEW's AudioContext, which needs an
        // app-control round-trip to the GUI. The data it would report now
        // exists (`window.__yggtermChimeAudio`, written by every chime), but
        // the transport is not wired — so say so rather than answer with
        // native-side facts under a name that promises webview ones.
        //
        // ⚠ Every line below ends with a bare `\` — a Rust line CONTINUATION,
        // which also eats the following line's indentation. `\\` is an escaped
        // backslash and would print as one, followed by this file's own
        // indentation, in the user's terminal.
        bail!(
            "unknown audio subcommand {subcommand:?} (expected {}).\n\
             `audio state` is NOT implemented: it must report the shell webview's \
             AudioContext, which needs an app-control round-trip. The chime script \
             already records the data at window.__yggtermChimeAudio.",
            AUDIO_SUBCOMMANDS.join("|"),
        );
    }

    let request = parse_play_request(args)?;
    let Some(player) = resolve_player() else {
        // Degrade honestly: name every binary we looked for, so the fix is
        // obvious and nobody concludes "the audio path is broken".
        let looked_for = PLAYER_CANDIDATES
            .iter()
            .map(|p| p.binary)
            .collect::<Vec<_>>()
            .join(", ");
        bail!(
            "no audio player found on PATH (looked for: {looked_for}). \
             Install one of them, or run this on a host with a sink."
        );
    };

    let wav = request.render();
    for index in 0..request.repeat {
        play_once(player, &wav)?;
        println!(
            "chime {}/{} played ({}) via {}",
            index + 1,
            request.repeat,
            request.label,
            player.binary,
        );
        if index + 1 < request.repeat && request.gap_ms > 0 {
            std::thread::sleep(std::time::Duration::from_millis(request.gap_ms));
        }
    }
    Ok(())
}

/// Reached by `server app audio` with no subcommand AND by
/// `server app audio --help|-h|help`. The second only works because
/// `classify_builtin_cli_command` knows this subcommand owns its own help; a
/// generic `server app … --help` interception makes this function dead code.
pub fn print_audio_help() {
    println!(
        "usage:
  yggterm server app audio play [--tone info|success|warning|error]
                                [--repeat <n>] [--gap-ms <n>]
                                [--preroll on|off|auto] [--volume 0..1]
  yggterm server app audio tune --notes '[[startSec,freqHz,peak], …]'
                                [--repeat <n>] [--gap-ms <n>]
                                [--preroll on|off|auto] [--volume 0..1]

Renders the chime to PCM in Rust and plays it through the platform sink
(pw-play, paplay or aplay). NO webview: WebKitGTK's autoplay gate streams
silent samples without a real user gesture, which an agent cannot produce.

The tones carry meaning by PATTERN, as in an aircraft cabin:
  info, success  one chime          (the passenger-call single note)
  warning        hi-lo              (the crew-interphone pair)
  error          hi-lo, three times (the emergency pattern)

The tune is owned by yggterm_core::notification_audio and shared with the
GUI's chime, so both paths play the same thing. It is MEASURED from real
cabin-chime recordings — re-measure rather than re-tune by ear."
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(rest: &[&str]) -> Vec<String> {
        let mut out = vec!["server".to_string(), "app".to_string()];
        out.extend(rest.iter().map(|s| s.to_string()));
        out
    }

    #[test]
    fn play_defaults_to_the_success_tone_once_at_full_volume() {
        let request = parse_play_request(&argv(&["audio", "play"])).expect("defaults parse");
        assert_eq!(request.label, "success");
        assert_eq!(request.repeat, 1);
        assert_eq!(request.volume, 1.0);
        assert_eq!(request.preroll, PrerollMode::Auto);
        assert_eq!(
            request.notes,
            notification_audio::tone_notes(ChimeTone::Success),
        );
    }

    #[test]
    fn every_tone_name_selects_the_registry_tune_and_its_registry_envelope() {
        for tone in ChimeTone::ALL {
            let request = parse_play_request(&argv(&["audio", "play", "--tone", tone.as_key()]))
                .expect("tone parses");
            assert_eq!(request.label, tone.as_key());
            assert_eq!(request.notes, notification_audio::tone_notes(*tone));
            // The envelope shape travels WITH the tone. A tone played at
            // ring 1.0 when the registry says 1.8 is a different chime, and
            // one played stretched but uncut is the "aftertaste" the user
            // rejected — neither may be reachable from the CLI.
            assert_eq!(request.ring, notification_audio::tone_ring(*tone));
            assert_eq!(request.tail_cut, notification_audio::tone_tail_cut(*tone));
        }
    }

    #[test]
    fn tune_plays_a_hand_written_note_list_with_the_envelope_as_measured() {
        let request = parse_play_request(&argv(&[
            "audio",
            "tune",
            "--notes",
            "[[0,880,0.3],[0.2,660,0.3]]",
        ]))
        .expect("tune parses");
        assert_eq!(request.label, "custom");
        assert_eq!(request.notes.len(), 2);
        assert_eq!(request.notes[0].freq_hz, 880.0);
        // A hand-written tune gets the measured envelope untouched: the
        // per-tone stretch and tail-cut are properties of the MEASURED tones,
        // and silently applying them would make an audition lie about what it
        // is auditioning.
        assert_eq!(request.ring, 1.0);
        assert_eq!(request.tail_cut, 0.0);
    }

    #[test]
    fn tune_without_notes_is_refused() {
        let err = parse_play_request(&argv(&["audio", "tune"])).unwrap_err();
        assert!(
            err.to_string().contains("--notes"),
            "the error must say what is missing: {err}",
        );
    }

    #[test]
    fn arguments_that_would_play_the_wrong_thing_are_refused() {
        for (args, why) in [
            (vec!["audio", "play", "--tone", "chirp"], "unknown tone"),
            (vec!["audio", "play", "--volume", "2"], "volume above 1"),
            (vec!["audio", "play", "--volume", "-1"], "negative volume"),
            (vec!["audio", "play", "--volume", "loud"], "volume NaN"),
            (vec!["audio", "play", "--repeat", "0"], "zero repeats"),
            (vec!["audio", "play", "--repeat", "-1"], "negative repeats"),
            (vec!["audio", "play", "--preroll", "maybe"], "bad preroll"),
            (
                vec!["audio", "tune", "--notes", "[[0,40000,0.3]]"],
                "above Nyquist",
            ),
        ] {
            assert!(
                parse_play_request(&argv(&args)).is_err(),
                "accepted {args:?} which is {why}",
            );
        }
    }

    /// A typo'd flag or a flag with no value must FAIL. Anything else is a
    /// silent wrong-value: `--voluem 0.2` played at full volume and a bare
    /// `--tone` played the default tone, both exiting 0, in the one lane whose
    /// whole thesis is "never a silent success".
    #[test]
    fn an_unknown_flag_or_a_dangling_flag_is_refused_by_name() {
        // (1) A typo. The old parser scanned for known flags and ignored the
        // rest, so this played at FULL volume and said nothing.
        let err = parse_play_request(&argv(&["audio", "play", "--voluem", "0.2"]))
            .expect_err("a misspelled option must not be ignored");
        let text = err.to_string();
        assert!(
            text.contains("--voluem"),
            "the error must name the token it rejected: {text}",
        );
        for accepted in PLAY_OPTIONS {
            assert!(
                text.contains(accepted),
                "the error must list {accepted} as an accepted option: {text}",
            );
        }
        // The typo'd request must not have parsed into a default-volume play.
        assert!(
            parse_play_request(&argv(&["audio", "play", "--voluem", "0.2"])).is_err(),
            "a typo'd volume flag must never resolve to the default volume",
        );

        // (2) A dangling flag: the value is simply missing.
        let dangling = parse_play_request(&argv(&["audio", "play", "--tone"]))
            .expect_err("a flag with no value must not fall back to the default");
        assert!(
            dangling.to_string().contains("--tone"),
            "the error must name the flag that is missing its value: {dangling}",
        );

        // (3) A flag whose "value" is the next flag — the same hole, one word
        // later: `--tone --volume 0.5` used to play the DEFAULT tone.
        let swallowed = parse_play_request(&argv(&["audio", "play", "--tone", "--volume", "0.5"]))
            .expect_err("a flag may not swallow the next option as its value");
        let swallowed = swallowed.to_string();
        assert!(
            swallowed.contains("--tone") && swallowed.contains("--volume"),
            "the error must name both the starved flag and what it nearly ate: {swallowed}",
        );

        // (4) Flags belonging to the OTHER subcommand are unknown here, not
        // silently ignored: `tune --tone error` must not play a custom tune
        // while the user believes they asked for the error tone.
        for (args, why) in [
            (
                vec![
                    "audio",
                    "tune",
                    "--notes",
                    "[[0,880,0.3]]",
                    "--tone",
                    "error",
                ],
                "--tone is not a tune option",
            ),
            (
                vec!["audio", "play", "--notes", "[[0,880,0.3]]"],
                "--notes is not a play option",
            ),
            (vec!["audio", "play", "extra"], "a bare positional word"),
            (
                vec!["audio", "play", "--volume", "0.5", "0.9"],
                "a dangling value",
            ),
            (
                vec!["audio", "play", "--volume", "0.2", "--volume", "0.9"],
                "the same flag twice — one of the two values is silently discarded",
            ),
        ] {
            assert!(
                parse_play_request(&argv(&args)).is_err(),
                "accepted {args:?} which is {why}",
            );
        }
    }

    #[test]
    fn preroll_off_actually_shortens_the_render() {
        let with = parse_play_request(&argv(&["audio", "play", "--preroll", "on"]))
            .expect("parses")
            .render();
        let without = parse_play_request(&argv(&["audio", "play", "--preroll", "off"]))
            .expect("parses")
            .render();
        assert!(
            with.len() > without.len(),
            "the pre-roll must add real frames, not just a flag",
        );
        let expected_delta = (SAMPLE_RATE_HZ as f32 * PREROLL_SECONDS) as usize * 2;
        assert!(
            (with.len() as i64 - without.len() as i64 - expected_delta as i64).abs() < 8,
            "pre-roll delta should be {expected_delta} bytes",
        );
        // `auto` resolves to on — a wasted pre-roll beats a clipped alert.
        assert_eq!(PrerollMode::Auto.seconds(), PrerollMode::On.seconds());
    }

    #[test]
    fn a_rendered_chime_is_a_playable_wav_with_audio_in_it() {
        let wav = parse_play_request(&argv(&["audio", "play"]))
            .expect("parses")
            .render();
        assert_eq!(&wav[0..4], b"RIFF");
        let loudest = wav[44..]
            .chunks_exact(2)
            .map(|c| i16::from_le_bytes([c[0], c[1]]).saturating_abs())
            .max()
            .unwrap_or(0);
        assert!(loudest > 1000, "rendered chime is silent (peak {loudest})");
    }

    #[test]
    fn the_player_list_degrades_through_pipewire_pulse_then_alsa() {
        let order: Vec<&str> = PLAYER_CANDIDATES.iter().map(|p| p.binary).collect();
        assert_eq!(order, vec!["pw-play", "paplay", "aplay"]);
        // Every candidate must be able to read the stream from stdin, or the
        // pipe in `play_once` silently plays nothing.
        for player in PLAYER_CANDIDATES {
            assert!(
                player.binary == "paplay" || player.args.contains(&"-"),
                "{} needs an explicit stdin argument",
                player.binary,
            );
        }
    }

    #[test]
    fn a_missing_player_is_named_not_swallowed() {
        // The failure mode this guards: "the command returned 0 and I heard
        // nothing". If no player exists the command must FAIL and say which
        // binaries it looked for.
        let saved = std::env::var_os("PATH");
        unsafe { std::env::set_var("PATH", "/nonexistent-for-this-test") };
        let resolved = resolve_player();
        let err = run_audio_command(&argv(&["audio", "play"]));
        match saved {
            Some(path) => unsafe { std::env::set_var("PATH", path) },
            None => unsafe { std::env::remove_var("PATH") },
        }
        assert!(
            resolved.is_none(),
            "no player should resolve on an empty PATH"
        );
        let err = err.expect_err("a missing player must be an error, never a silent success");
        let text = err.to_string();
        // The EXACT joined list, not per-binary `contains`: "paplay" contains
        // "aplay", so a loop of substring checks passes on an error that never
        // mentioned aplay at all. Assert the whole list, once.
        let looked_for = PLAYER_CANDIDATES
            .iter()
            .map(|p| p.binary)
            .collect::<Vec<_>>()
            .join(", ");
        assert_eq!(
            looked_for, "pw-play, paplay, aplay",
            "the candidate list changed — update the message this test locks",
        );
        assert!(
            text.contains(&format!("looked for: {looked_for}")),
            "the error must name every binary it looked for, in order: {text}",
        );
    }

    #[test]
    fn audio_state_is_refused_rather_than_answered_with_the_wrong_facts() {
        let err = run_audio_command(&argv(&["audio", "state"]))
            .expect_err("state is not implemented yet");
        let text = err.to_string();
        assert!(
            text.contains("NOT implemented") && text.contains("__yggtermChimeAudio"),
            "the refusal must say what is missing and where the data already is: {text}",
        );
        // …and it must READ like prose in a terminal. A `\\` where a line
        // continuation `\` was meant prints a literal backslash followed by
        // this file's own source indentation, and the two substring checks
        // above passed straight over that garbage.
        assert!(
            !text.contains('\\'),
            "the refusal prints a literal backslash — a `\\\\` escape where a \
             line continuation was meant: {text:?}",
        );
        for line in text.lines() {
            assert!(
                !line.starts_with(' '),
                "the refusal leaks source indentation into the terminal: {line:?}",
            );
        }
        assert!(
            !text.contains("  "),
            "the refusal carries a run of spaces from the source layout: {text:?}",
        );
    }

    #[test]
    fn audio_help_is_reachable_and_names_both_subcommands() {
        // `server app audio --help` only reaches this because
        // `classify_builtin_cli_command` knows this subcommand owns its help;
        // the classifier side is locked in main.rs. This is the other half:
        // the dispatcher must actually answer all four spellings.
        for spelling in ["", "--help", "-h", "help"] {
            let args = if spelling.is_empty() {
                argv(&["audio"])
            } else {
                argv(&["audio", spelling])
            };
            run_audio_command(&args)
                .unwrap_or_else(|err| panic!("`audio {spelling}` must print help, got {err}"));
        }
        // An unknown subcommand is still an error, not silent help.
        assert!(run_audio_command(&argv(&["audio", "warble"])).is_err());
    }
}
