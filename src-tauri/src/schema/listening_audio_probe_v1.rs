use super::listening_runtime_v1::{
    ListeningAudioIssueCodeV1, ListeningAudioProbeStatusV1, ListeningAudioProbeV1,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::{self, Read};
use std::path::Path;
use symphonia::core::audio::GenericAudioBufferRef;
use symphonia::core::codecs::audio::{AudioCodecParameters, AudioDecoderOptions};
use symphonia::core::codecs::audio::well_known::{
    CODEC_ID_AAC, CODEC_ID_MP3, CODEC_ID_PCM_F32BE,
    CODEC_ID_PCM_F32LE, CODEC_ID_PCM_F64BE, CODEC_ID_PCM_F64LE, CODEC_ID_PCM_S16BE,
    CODEC_ID_PCM_S16LE, CODEC_ID_PCM_S24BE, CODEC_ID_PCM_S24LE, CODEC_ID_PCM_S32BE,
    CODEC_ID_PCM_S32LE, CODEC_ID_PCM_S8, CODEC_ID_PCM_U16BE, CODEC_ID_PCM_U16LE,
    CODEC_ID_PCM_U24BE, CODEC_ID_PCM_U24LE, CODEC_ID_PCM_U32BE, CODEC_ID_PCM_U32LE,
    CODEC_ID_PCM_U8,
};
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, TrackType};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;

pub const LISTENING_AUDIO_PROBE_V1_PROVIDER: &str = "symphonia";
pub const LISTENING_AUDIO_PROBE_V1_PROVIDER_VERSION: &str = "0.6.0";
pub const LISTENING_AUDIO_PROBE_RESULT_V1_SCHEMA_VERSION: &str =
    "ListeningAudioProbeResultV1";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct ListeningAudioProbePolicyV1 {
    pub supported_mimes: Vec<String>,
    pub near_silent_rms_threshold: f64,
    pub severe_clipping_sample_ratio: f64,
}

impl Default for ListeningAudioProbePolicyV1 {
    fn default() -> Self {
        Self {
            supported_mimes: vec![
                "audio/wav".to_string(),
                "audio/mpeg".to_string(),
                "audio/mp4".to_string(),
            ],
            near_silent_rms_threshold: 0.001,
            severe_clipping_sample_ratio: 0.01,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct ListeningAudioSignalMetricsV1 {
    pub decoded_sample_count: u64,
    pub peak_amplitude: f64,
    pub rms_amplitude: f64,
    pub clipped_sample_ratio: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct ListeningAudioProbeResultV1 {
    pub schema_version: String,
    pub file_name: String,
    pub byte_length: u64,
    pub sha256: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mime: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub container: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub codec: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channels: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sample_rate_hz: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signal: Option<ListeningAudioSignalMetricsV1>,
    pub probe: ListeningAudioProbeV1,
    pub details: Vec<String>,
}

impl ListeningAudioProbeResultV1 {
    pub fn is_passed(&self) -> bool {
        self.probe.status == ListeningAudioProbeStatusV1::Passed
            && self.probe.issue_codes.is_empty()
    }
}

#[derive(Default)]
struct SignalAccumulator {
    samples: u64,
    sum_squares: f64,
    peak: f64,
    clipped: u64,
}

impl SignalAccumulator {
    fn add(&mut self, sample: f32) {
        let sample = f64::from(sample);
        let absolute = sample.abs();
        self.samples += 1;
        self.sum_squares += sample * sample;
        self.peak = self.peak.max(absolute);
        if absolute >= 0.999 {
            self.clipped += 1;
        }
    }

    fn finish(self) -> ListeningAudioSignalMetricsV1 {
        let denominator = self.samples.max(1) as f64;
        ListeningAudioSignalMetricsV1 {
            decoded_sample_count: self.samples,
            peak_amplitude: self.peak,
            rms_amplitude: (self.sum_squares / denominator).sqrt(),
            clipped_sample_ratio: self.clipped as f64 / denominator,
        }
    }
}

struct DecodedAudioFacts {
    codec: String,
    duration_ms: u64,
    channels: u16,
    sample_rate_hz: u32,
    signal: ListeningAudioSignalMetricsV1,
}

pub fn probe_listening_audio_v1(
    path: &Path,
    expected_sha256: Option<&str>,
    policy: &ListeningAudioProbePolicyV1,
) -> ListeningAudioProbeResultV1 {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("audio")
        .to_string();
    let mut details = Vec::new();
    let mut issue_codes = Vec::new();
    let (byte_length, sha256) = match hash_file(path) {
        Ok(value) => value,
        Err(error) => {
            details.push(format!("audio_open_failed:{error}"));
            issue_codes.push(ListeningAudioIssueCodeV1::AudioDecodeFailed);
            (0, String::new())
        }
    };
    if let Some(expected) = expected_sha256 {
        if !sha256.eq_ignore_ascii_case(expected) {
            issue_codes.push(ListeningAudioIssueCodeV1::AudioHashMismatch);
        }
    }

    let (container, mime) = container_and_mime(path);
    if mime
        .as_ref()
        .is_none_or(|mime| !policy.supported_mimes.iter().any(|allowed| allowed == mime))
    {
        issue_codes.push(ListeningAudioIssueCodeV1::AudioCodecUnsupported);
    }

    let decoded = if byte_length > 0 {
        match decode_audio(path) {
            Ok(decoded) => Some(decoded),
            Err(error) => {
                details.push(format!("audio_decode_failed:{error}"));
                issue_codes.push(ListeningAudioIssueCodeV1::AudioDecodeFailed);
                None
            }
        }
    } else {
        None
    };

    if let Some(decoded) = &decoded {
        if decoded.signal.rms_amplitude <= policy.near_silent_rms_threshold {
            issue_codes.push(ListeningAudioIssueCodeV1::AudioNearSilent);
        }
        if decoded.signal.clipped_sample_ratio >= policy.severe_clipping_sample_ratio {
            issue_codes.push(ListeningAudioIssueCodeV1::AudioSevereClipping);
        }
    }
    let mut unique_issue_codes = Vec::with_capacity(issue_codes.len());
    for issue_code in issue_codes {
        if !unique_issue_codes.contains(&issue_code) {
            unique_issue_codes.push(issue_code);
        }
    }
    let issue_codes = unique_issue_codes;
    let status = if issue_codes.is_empty() {
        ListeningAudioProbeStatusV1::Passed
    } else {
        ListeningAudioProbeStatusV1::Blocked
    };
    ListeningAudioProbeResultV1 {
        schema_version: LISTENING_AUDIO_PROBE_RESULT_V1_SCHEMA_VERSION.to_string(),
        file_name,
        byte_length,
        sha256,
        mime,
        container,
        codec: decoded.as_ref().map(|facts| facts.codec.clone()),
        duration_ms: decoded.as_ref().map(|facts| facts.duration_ms),
        channels: decoded.as_ref().map(|facts| facts.channels),
        sample_rate_hz: decoded.as_ref().map(|facts| facts.sample_rate_hz),
        signal: decoded.map(|facts| facts.signal),
        probe: ListeningAudioProbeV1 {
            status,
            provider: LISTENING_AUDIO_PROBE_V1_PROVIDER.to_string(),
            provider_version: LISTENING_AUDIO_PROBE_V1_PROVIDER_VERSION.to_string(),
            probed_at: Utc::now().to_rfc3339(),
            issue_codes,
        },
        details,
    }
}

fn hash_file(path: &Path) -> io::Result<(u64, String)> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    let mut byte_length = 0_u64;
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        byte_length += read as u64;
        hasher.update(&buffer[..read]);
    }
    Ok((byte_length, format!("{:x}", hasher.finalize())))
}

fn container_and_mime(path: &Path) -> (Option<String>, Option<String>) {
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.to_ascii_lowercase());
    match extension.as_deref() {
        Some("wav") | Some("wave") => (Some("wav".to_string()), Some("audio/wav".to_string())),
        Some("mp3") => (Some("mp3".to_string()), Some("audio/mpeg".to_string())),
        Some("m4a") | Some("mp4") => (Some("isomp4".to_string()), Some("audio/mp4".to_string())),
        _ => (extension, None),
    }
}

fn codec_name(parameters: &AudioCodecParameters) -> Option<&'static str> {
    match parameters.codec {
        CODEC_ID_AAC => Some("aac"),
        CODEC_ID_MP3 => Some("mp3"),
        CODEC_ID_PCM_S8 => Some("pcm_s8"),
        CODEC_ID_PCM_U8 => Some("pcm_u8"),
        CODEC_ID_PCM_S16LE => Some("pcm_s16le"),
        CODEC_ID_PCM_S16BE => Some("pcm_s16be"),
        CODEC_ID_PCM_U16LE => Some("pcm_u16le"),
        CODEC_ID_PCM_U16BE => Some("pcm_u16be"),
        CODEC_ID_PCM_S24LE => Some("pcm_s24le"),
        CODEC_ID_PCM_S24BE => Some("pcm_s24be"),
        CODEC_ID_PCM_U24LE => Some("pcm_u24le"),
        CODEC_ID_PCM_U24BE => Some("pcm_u24be"),
        CODEC_ID_PCM_S32LE => Some("pcm_s32le"),
        CODEC_ID_PCM_S32BE => Some("pcm_s32be"),
        CODEC_ID_PCM_U32LE => Some("pcm_u32le"),
        CODEC_ID_PCM_U32BE => Some("pcm_u32be"),
        CODEC_ID_PCM_F32LE => Some("pcm_f32le"),
        CODEC_ID_PCM_F32BE => Some("pcm_f32be"),
        CODEC_ID_PCM_F64LE => Some("pcm_f64le"),
        CODEC_ID_PCM_F64BE => Some("pcm_f64be"),
        _ => None,
    }
}

fn accumulate(
    decoded: GenericAudioBufferRef<'_>,
    samples: &mut SignalAccumulator,
) -> (u64, u32, u16) {
    let frames = decoded.frames() as u64;
    let sample_rate_hz = decoded.spec().rate();
    let channels = decoded.spec().channels().count() as u16;
    let mut buffer = vec![0.0_f32; decoded.samples_interleaved()];
    decoded.copy_to_slice_interleaved(&mut buffer);
    for sample in buffer {
        samples.add(sample);
    }
    (frames, sample_rate_hz, channels)
}

fn decode_audio(path: &Path) -> Result<DecodedAudioFacts, String> {
    let file = File::open(path).map_err(|error| error.to_string())?;
    let stream = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(extension) = path.extension().and_then(|value| value.to_str()) {
        hint.with_extension(extension);
    }
    let mut format = symphonia::default::get_probe()
        .probe(
            &hint,
            stream,
            FormatOptions::default(),
            MetadataOptions::default(),
        )
        .map_err(|error| error.to_string())?;
    let track = format
        .default_track(TrackType::Audio)
        .ok_or_else(|| "audio_track_missing".to_string())?;
    let track_id = track.id;
    let codec_parameters = track
        .codec_params
        .as_ref()
        .and_then(|parameters| parameters.audio())
        .cloned()
        .ok_or_else(|| "audio_codec_parameters_missing".to_string())?;
    let codec = codec_name(&codec_parameters)
        .ok_or_else(|| "audio_codec_unsupported".to_string())?
        .to_string();
    let mut decoder = symphonia::default::get_codecs()
        .make_audio_decoder(&codec_parameters, &AudioDecoderOptions::default())
        .map_err(|error| error.to_string())?;
    let mut sample_frames = 0_u64;
    let mut sample_rate_hz = 0_u32;
    let mut channels = 0_u16;
    let mut signal = SignalAccumulator::default();

    loop {
        let packet = match format.next_packet() {
            Ok(Some(packet)) => packet,
            Ok(None) => break,
            Err(error) => return Err(error.to_string()),
        };
        if packet.track_id != track_id {
            continue;
        }
        let decoded = decoder.decode(&packet).map_err(|error| error.to_string())?;
        let (frames, rate, channel_count) = accumulate(decoded, &mut signal);
        sample_frames += frames;
        sample_rate_hz = rate;
        channels = channel_count;
    }
    if sample_frames == 0 || sample_rate_hz == 0 || channels == 0 {
        return Err("audio_decoded_no_samples".to_string());
    }
    Ok(DecodedAudioFacts {
        codec,
        duration_ms: sample_frames.saturating_mul(1000) / u64::from(sample_rate_hz),
        channels,
        sample_rate_hz,
        signal: signal.finish(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Write;
    use uuid::Uuid;

    fn write_wav(path: &Path, samples: &[i16], sample_rate: u32) {
        let data_bytes = (samples.len() * 2) as u32;
        let mut bytes = Vec::with_capacity(44 + data_bytes as usize);
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&(36 + data_bytes).to_le_bytes());
        bytes.extend_from_slice(b"WAVEfmt ");
        bytes.extend_from_slice(&16_u32.to_le_bytes());
        bytes.extend_from_slice(&1_u16.to_le_bytes());
        bytes.extend_from_slice(&1_u16.to_le_bytes());
        bytes.extend_from_slice(&sample_rate.to_le_bytes());
        bytes.extend_from_slice(&(sample_rate * 2).to_le_bytes());
        bytes.extend_from_slice(&2_u16.to_le_bytes());
        bytes.extend_from_slice(&16_u16.to_le_bytes());
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&data_bytes.to_le_bytes());
        for sample in samples {
            bytes.extend_from_slice(&sample.to_le_bytes());
        }
        let mut file = File::create(path).unwrap();
        file.write_all(&bytes).unwrap();
    }

    fn fixture_path(label: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("phase7-{label}-{}.wav", Uuid::new_v4()))
    }

    #[test]
    fn valid_wav_decodes_and_reports_signal_facts() {
        let path = fixture_path("valid");
        let samples = (0..16_000)
            .map(|index| {
                let phase = (index as f64 / 16_000.0) * 440.0 * std::f64::consts::TAU;
                (phase.sin() * 8_000.0) as i16
            })
            .collect::<Vec<_>>();
        write_wav(&path, &samples, 16_000);
        let result = probe_listening_audio_v1(
            &path,
            None,
            &ListeningAudioProbePolicyV1::default(),
        );
        assert!(result.is_passed(), "{:?}", result.probe.issue_codes);
        assert_eq!(result.codec.as_deref(), Some("pcm_s16le"));
        assert_eq!(result.duration_ms, Some(1000));
        assert_eq!(result.channels, Some(1));
        assert_eq!(result.sample_rate_hz, Some(16_000));
        assert!(result.signal.as_ref().unwrap().rms_amplitude > 0.1);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn silent_clipped_corrupt_and_hash_mismatch_inputs_fail_closed() {
        let silent_path = fixture_path("silent");
        write_wav(&silent_path, &vec![0; 1600], 16_000);
        let silent = probe_listening_audio_v1(
            &silent_path,
            None,
            &ListeningAudioProbePolicyV1::default(),
        );
        assert!(silent
            .probe
            .issue_codes
            .contains(&ListeningAudioIssueCodeV1::AudioNearSilent));

        let clipped_path = fixture_path("clipped");
        write_wav(&clipped_path, &vec![i16::MAX; 1600], 16_000);
        let clipped = probe_listening_audio_v1(
            &clipped_path,
            None,
            &ListeningAudioProbePolicyV1::default(),
        );
        assert!(clipped
            .probe
            .issue_codes
            .contains(&ListeningAudioIssueCodeV1::AudioSevereClipping));

        let corrupt_path = fixture_path("corrupt");
        fs::write(&corrupt_path, b"not audio").unwrap();
        let corrupt = probe_listening_audio_v1(
            &corrupt_path,
            Some(&"0".repeat(64)),
            &ListeningAudioProbePolicyV1::default(),
        );
        assert!(corrupt
            .probe
            .issue_codes
            .contains(&ListeningAudioIssueCodeV1::AudioDecodeFailed));
        assert!(corrupt
            .probe
            .issue_codes
            .contains(&ListeningAudioIssueCodeV1::AudioHashMismatch));

        for path in [silent_path, clipped_path, corrupt_path] {
            let _ = fs::remove_file(path);
        }
    }

    #[test]
    fn declared_mp3_aac_and_wav_inputs_have_registered_decoders_and_mime_policy() {
        let codecs = symphonia::default::get_codecs();
        assert!(codecs.get_audio_decoder(CODEC_ID_MP3).is_some());
        assert!(codecs.get_audio_decoder(CODEC_ID_AAC).is_some());
        assert_eq!(
            container_and_mime(Path::new("fixture.mp3")),
            (Some("mp3".to_string()), Some("audio/mpeg".to_string()))
        );
        assert_eq!(
            container_and_mime(Path::new("fixture.m4a")),
            (Some("isomp4".to_string()), Some("audio/mp4".to_string()))
        );

        let unsupported_path = std::env::temp_dir().join(format!(
            "phase7-unsupported-extension-{}.bin",
            Uuid::new_v4()
        ));
        write_wav(&unsupported_path, &vec![1_000; 1600], 16_000);
        let result = probe_listening_audio_v1(
            &unsupported_path,
            None,
            &ListeningAudioProbePolicyV1::default(),
        );
        assert!(result
            .probe
            .issue_codes
            .contains(&ListeningAudioIssueCodeV1::AudioCodecUnsupported));
        assert_eq!(
            result
                .probe
                .issue_codes
                .iter()
                .filter(|code| **code == ListeningAudioIssueCodeV1::AudioCodecUnsupported)
                .count(),
            1
        );
        let _ = fs::remove_file(unsupported_path);
    }
}
