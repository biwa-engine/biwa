//! 音のアセットを読み込んで、ステレオの PCM にデコードする (symphonia)。
//!
//! 外から見えるのは [`SoundLoader`] / [`decode_file`] と、`core::sound` の型だけである。
//!
//! - 段 1 の形式は Tier 1 の MP3・WAV (PCM)・Ogg Vorbis (`docs/sound.md`)。
//!   m4a (AAC) はここでは読まない (段 2 で OS のデコーダを使う)。読めない形式は誤りを返し、
//!   エンジンは長さ 0 の音として扱う。
//! - モノラルはステレオに広げる。3 チャンネル以上はまだ扱わない (誤りを返す)。
//! - 読み込みとデコードは専用のスレッドで行う (画像と同じ。数 MB の BGM のデコードは描画を止める)。
//!
//! symphonia は MPL-2.0 である。改変せずに使い、ソースの所在を告知する義務は配布物の側で負う
//! (告知を同梱する仕組みは別 issue)。

use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};

use biwa_player_core::sound::{DecodedSound, SoundHandle, SoundRequest};
use symphonia::core::codecs::audio::AudioDecoderOptions;
use symphonia::core::errors::Error;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, TrackType};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;

/// 読み込みの結果。
pub type LoadResult = (SoundHandle, Result<Arc<DecodedSound>, String>);

/// 音を読む者。依頼を受けて裏のスレッドで読み、結果をためておく。
pub struct SoundLoader {
    requests: Sender<SoundRequest>,
    results: Receiver<LoadResult>,
}

impl SoundLoader {
    /// `assets` (パッケージの `assets/` に当たるディレクトリ) から読む。
    ///
    /// `notify` は結果が 1 つ届くたびに (裏のスレッドから) 呼ばれる。イベントループを起こすのに使う。
    pub fn new(assets: PathBuf, notify: impl Fn() + Send + 'static) -> std::io::Result<Self> {
        let (requests, request_rx) = mpsc::channel::<SoundRequest>();
        let (result_tx, results) = mpsc::channel();
        std::thread::Builder::new().name("biwa-sound".into()).spawn(move || {
            for request in request_rx {
                let file = assets.join(&request.path);
                let result = decode_file(&file)
                    .map(Arc::new)
                    .map_err(|e| format!("failed to load \"{}\" (looked for {}): {e}", request.path, file.display()));
                if result_tx.send((request.sound, result)).is_err() {
                    return;
                }
                notify();
            }
        })?;
        Ok(Self { requests, results })
    }

    /// 読み込みを依頼する。
    pub fn request(&self, request: SoundRequest) {
        // 送れないのは裏のスレッドが終わったあとで、そのときは結果も来ない。
        let _ = self.requests.send(request);
    }

    /// 届いている結果を 1 つ取る。
    pub fn try_recv(&self) -> Option<LoadResult> {
        self.results.try_recv().ok()
    }
}

/// ファイルをデコードする。
pub fn decode_file(path: &Path) -> Result<DecodedSound, String> {
    let file = File::open(path).map_err(|e| e.to_string())?;
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    decode(MediaSourceStream::new(Box::new(file), Default::default()), &hint)
}

fn decode(source: MediaSourceStream<'static>, hint: &Hint) -> Result<DecodedSound, String> {
    let mut format = symphonia::default::get_probe()
        .probe(hint, source, FormatOptions::default(), MetadataOptions::default())
        .map_err(|e| format!("unsupported or broken audio file ({e}); supported formats are MP3, WAV (PCM) and Ogg Vorbis"))?;
    let track = format.default_track(TrackType::Audio).ok_or("the file has no audio track")?;
    let params = track
        .codec_params
        .as_ref()
        .and_then(|p| p.audio())
        .ok_or("the file has no audio codec parameters")?;
    let mut decoder = symphonia::default::get_codecs()
        .make_audio_decoder(params, &AudioDecoderOptions::default())
        .map_err(|e| format!("unsupported audio codec ({e}); supported formats are MP3, WAV (PCM) and Ogg Vorbis"))?;
    let track_id = track.id;

    let mut out = Vec::new();
    let mut sample_rate = 0;
    let mut buffer: Vec<f32> = Vec::new();
    loop {
        let packet = match format.next_packet() {
            Ok(Some(packet)) => packet,
            Ok(None) => break,
            Err(Error::ResetRequired) => break,
            Err(e) => return Err(e.to_string()),
        };
        if packet.track_id != track_id {
            continue;
        }
        let audio = match decoder.decode(&packet) {
            Ok(audio) => audio,
            // 壊れたパケットは飛ばす (MP3 の先頭などで起きる)。
            Err(Error::DecodeError(_)) => continue,
            Err(e) => return Err(e.to_string()),
        };
        let spec = audio.spec();
        sample_rate = spec.rate();
        let channels = spec.channels().count();
        if channels == 0 || channels > 2 {
            return Err(format!(
                "{channels}-channel audio is not supported yet (use mono or stereo files)"
            ));
        }
        buffer.resize(audio.samples_interleaved(), 0.0);
        audio.copy_to_slice_interleaved(&mut buffer);
        push_stereo(&mut out, &buffer, channels);
    }
    if sample_rate == 0 {
        return Err("the file contains no audio".into());
    }
    Ok(DecodedSound { sample_rate, samples: out })
}

/// 交互に並んだサンプルを、ステレオに揃えて足す (モノラルは左右に同じものを流す)。
fn push_stereo(out: &mut Vec<f32>, interleaved: &[f32], channels: usize) {
    if channels == 1 {
        out.reserve(interleaved.len() * 2);
        for s in interleaved {
            out.push(*s);
            out.push(*s);
        }
    } else {
        out.extend_from_slice(interleaved);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 16 bit の PCM の WAV を作る。
    fn wav(channels: u16, rate: u32, frames: &[i16]) -> Vec<u8> {
        let data_len = (frames.len() * 2) as u32;
        let mut v = Vec::new();
        v.extend_from_slice(b"RIFF");
        v.extend_from_slice(&(36 + data_len).to_le_bytes());
        v.extend_from_slice(b"WAVEfmt ");
        v.extend_from_slice(&16u32.to_le_bytes());
        v.extend_from_slice(&1u16.to_le_bytes());
        v.extend_from_slice(&channels.to_le_bytes());
        v.extend_from_slice(&rate.to_le_bytes());
        v.extend_from_slice(&(rate * u32::from(channels) * 2).to_le_bytes());
        v.extend_from_slice(&(channels * 2).to_le_bytes());
        v.extend_from_slice(&16u16.to_le_bytes());
        v.extend_from_slice(b"data");
        v.extend_from_slice(&data_len.to_le_bytes());
        for s in frames {
            v.extend_from_slice(&s.to_le_bytes());
        }
        v
    }

    fn temp(name: &str, bytes: &[u8]) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("biwa-sound-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        std::fs::write(&path, bytes).unwrap();
        path
    }

    #[test]
    fn mono_wav_is_widened_to_stereo() {
        let samples: Vec<i16> = (0..4800).map(|i| if i % 2 == 0 { 16384 } else { -16384 }).collect();
        let path = temp("mono.wav", &wav(1, 48000, &samples));
        let decoded = decode_file(&path).unwrap();
        assert_eq!(decoded.sample_rate, 48000);
        assert_eq!(decoded.frames(), 4800);
        assert!((decoded.length_ms() - 100.0).abs() < 1e-9);
        assert!((decoded.samples[0] - 0.5).abs() < 1e-3);
        assert_eq!(decoded.samples[0], decoded.samples[1]);
    }

    #[test]
    fn loader_reports_missing_and_unsupported_files() {
        let dir = temp("stereo.wav", &wav(2, 44100, &[0; 8820])).parent().unwrap().to_path_buf();
        std::fs::write(dir.join("broken.mp3"), b"not audio").unwrap();
        let (tx, rx) = mpsc::channel();
        let loader = SoundLoader::new(dir, move || {
            let _ = tx.send(());
        })
        .unwrap();
        for (i, path) in ["stereo.wav", "missing.ogg", "broken.mp3"].iter().enumerate() {
            loader.request(SoundRequest { sound: SoundHandle(i as u32 + 1), path: (*path).into() });
        }
        for _ in 0..3 {
            rx.recv().unwrap();
        }
        let (h, ok) = loader.try_recv().unwrap();
        assert_eq!(h, SoundHandle(1));
        assert!((ok.unwrap().length_ms() - 100.0).abs() < 1e-9);
        assert!(loader.try_recv().unwrap().1.unwrap_err().contains("missing.ogg"));
        assert!(loader.try_recv().unwrap().1.is_err());
    }
}
