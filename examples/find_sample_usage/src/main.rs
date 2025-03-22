use eyre::WrapErr;
use eyre::eyre;
use phantasy_init::init;
use serde::Deserialize;
use serde::Serialize;
use std::collections::HashMap;
use std::fs::File;
use std::fs::{self};
use std::io::BufReader;
use std::io::BufWriter;
use std::path::Path;
use std::path::PathBuf;
use tokio::process::Command;
use tracing::debug;
use tracing::info;
use tracing::warn;

#[tokio::main]
async fn main() -> eyre::Result<()> {
    init()?;

    // Read environment variables
    let music_dir = var("MUSIC_DIR")?;
    let music_dir = PathBuf::from(music_dir);

    let mut sample_path = PathBuf::from(var("SAMPLE_PATH")?);
    let sample_begin = var("SAMPLE_BEGIN")?.parse::<f32>()?;
    let sample_end = var("SAMPLE_END")?.parse::<f32>()?;

    // Ensure sample is OGG, else convert
    sample_path = ensure_ogg(sample_path).await?;
    info!("Using sample OGG: {:?}", sample_path);

    // Decode sample snippet
    let sample_pcm = decode_ogg_to_mono_f32(&sample_path)?;
    let sample_rate = 48_000.0; // Hard-coded for simplicity; real code should detect from decode
    let snippet = extract_snippet(&sample_pcm, sample_rate, sample_begin, sample_end);

    // Compute (or load) fingerprint of sample snippet
    // We'll do it in-memory for the snippet itself
    let snippet_fp = compute_fingerprint(&snippet, sample_rate as usize)?;

    info!("Snippet fingerprint length: {}", snippet_fp.pairs.len());

    // Gather OGG files
    let mut ogg_files = Vec::new();
    for entry in fs::read_dir(&music_dir)? {
        let path = entry?.path();
        if path.extension().map_or(false, |ext| ext == "ogg") {
            ogg_files.push(path);
        }
    }
    info!("Found {} OGG files", ogg_files.len());

    // For each track, load (or build) a fingerprint, then compare with snippet's fingerprint
    for track_path in &ogg_files {
        match find_matches(track_path, &snippet_fp, sample_rate as usize).await {
            Ok(Some((best_offset_sec, best_count))) => {
                info!(
                    "Likely match in {} at ~{:.2} sec (overlap count = {})",
                    track_path.display(),
                    best_offset_sec,
                    best_count
                );
            }
            Ok(None) => {
                info!("No strong match in {}", track_path.display());
            }
            Err(e) => {
                warn!("Error matching {}: {:?}", track_path.display(), e);
            }
        }
    }

    Ok(())
}

// Read an env var or bail
fn var(key: &str) -> eyre::Result<String> {
    std::env::var(key).map_err(|_| eyre!("Missing env var: {}", key))
}

/// Ensure the given path is OGG. If not, convert via `ffmpeg`.
async fn ensure_ogg(path: PathBuf) -> eyre::Result<PathBuf> {
    if path.extension().map_or(false, |ext| ext == "ogg") {
        return Ok(path);
    }
    // Convert
    let new_path = path.with_extension("ogg");
    if !new_path.exists() {
        info!("Converting to OGG: {:?}", path);
        let mut cmd = Command::new("ffmpeg");
        let parent_dir = path.parent().ok_or(eyre!("Invalid path: {:?}", path))?;
        cmd.current_dir(parent_dir);
        cmd.args(&["-i", &path.to_string_lossy()]);
        cmd.arg("-vn"); // drop video streams
        cmd.arg("-c:a").arg("libvorbis");
        cmd.arg("-q:a").arg("5");
        cmd.arg("-y")
            .arg(new_path.file_name().ok_or(eyre!("Missing filename"))?);
        let status = cmd
            .status()
            .await
            .wrap_err("ffmpeg failed to convert to OGG")?;
        if !status.success() {
            return Err(eyre!("ffmpeg returned non-zero status"));
        }
    }
    Ok(new_path)
}

/// Extract snippet from PCM given time range in seconds.
fn extract_snippet<'a>(pcm: &'a [f32], sr: f32, begin: f32, end: f32) -> &'a [f32] {
    let start_idx = (begin * sr).round() as usize;
    let end_idx = (end * sr).round() as usize;
    let start_idx = start_idx.min(pcm.len());
    let end_idx = end_idx.min(pcm.len());
    &pcm[start_idx..end_idx]
}

/// Decode an OGG file to raw mono f32 PCM (using i16 as intermediate).
fn decode_ogg_to_mono_f32(path: &Path) -> eyre::Result<Vec<f32>> {
    use lewton::inside_ogg::OggStreamReader;
    use std::fs::File;
    use std::io::BufReader;

    let file = File::open(path)?;
    let mut reader = BufReader::new(file);
    let mut ogg_reader = OggStreamReader::new(&mut reader)?;

    let mut pcm = Vec::new();
    while let Some(packet) = ogg_reader.read_dec_packet_generic::<Vec<Vec<i16>>>()? {
        let num_channels = packet.len();
        if num_channels == 0 {
            continue;
        }
        let samples_per_channel = packet[0].len();
        for i in 0..samples_per_channel {
            let mut sum = 0.0;
            for ch in 0..num_channels {
                sum += packet[ch][i] as f32;
            }
            pcm.push(sum / num_channels as f32);
        }
    }
    Ok(pcm)
}

//
// Shazam-Style Fingerprint
//

#[derive(Debug, Clone, Serialize, Deserialize)]
struct FingerprintData {
    /// Pairs of (f1, f2, deltaTime), mapped to the "anchor time" offset
    /// We store them in a Vec for demonstration, but you might store differently.
    pairs: Vec<FPHashEntry>,
}

// Each "hash" from a peak pair
#[derive(Debug, Clone, Serialize, Deserialize)]
struct FPHashEntry {
    f1: u16,
    f2: u16,
    delta_t: u16,
    /// The offset (in spectrogram frames) when this pair occurred
    anchor_time: u32,
}

/// Build a basic fingerprint from PCM data
fn compute_fingerprint(pcm: &[f32], sample_rate: usize) -> eyre::Result<FingerprintData> {
    // 1) Build a spectrogram
    //    For demonstration, we’ll keep it smaller windows to be faster
    let window_size = 1024;
    let hop_size = 512;
    let spec = compute_spectrogram(pcm, sample_rate, window_size, hop_size)?;

    // 2) Find local maxima in each time slice
    let peaks_by_time = find_peaks(&spec);

    // 3) Create pairs (f1, f2, delta_t)
    //    We'll pair each peak with a handful of future peaks to get (f1, f2, Δt).
    let fan_value = 5; // how many peaks to pair with
    let mut pairs = Vec::new();

    for (t, peaks) in peaks_by_time.iter().enumerate() {
        for (_i, &f1) in peaks.iter().enumerate() {
            // Pair with up to fan_value subsequent peaks in next frames
            for future_t in (t + 1)..(t + 10).min(peaks_by_time.len()) {
                // pick up to fan_value peaks from the future frame
                let future_peaks = &peaks_by_time[future_t];
                for (j, &f2) in future_peaks.iter().enumerate() {
                    if j >= fan_value {
                        break;
                    }
                    let delta_t = (future_t - t) as u16;
                    pairs.push(FPHashEntry {
                        f1,
                        f2,
                        delta_t,
                        anchor_time: t as u32,
                    });
                }
            }
        }
    }

    Ok(FingerprintData { pairs })
}

/// Compute a spectrogram of `pcm` with Hann window. Return matrix of shape (n_freq, n_frames).
fn compute_spectrogram(
    pcm: &[f32],
    _sample_rate: usize,
    window_size: usize,
    hop_size: usize,
) -> eyre::Result<Vec<Vec<f32>>> {
    use rustfft::FftPlanner;
    use rustfft::num_complex::Complex;
    use rustfft::num_traits::Zero;

    let n_hops = (pcm.len().saturating_sub(window_size)) / hop_size + 1;
    let n_freqs = window_size / 2;

    let mut planner = FftPlanner::new();
    let fft = planner.plan_fft_forward(window_size);

    let mut spectrogram = vec![vec![0.0; n_hops]; n_freqs];
    let mut buffer = vec![Complex::<f32>::zero(); window_size];

    // Hann window
    let window_func: Vec<f32> = (0..window_size)
        .map(|i| 0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / window_size as f32).cos())
        .collect();

    for hop_idx in 0..n_hops {
        let offset = hop_idx * hop_size;
        for i in 0..window_size {
            buffer[i].re = pcm[offset + i] * window_func[i];
            buffer[i].im = 0.0;
        }
        fft.process(&mut buffer);

        for freq_bin in 0..n_freqs {
            let re = buffer[freq_bin].re;
            let im = buffer[freq_bin].im;
            let mag = (re * re + im * im).sqrt();
            spectrogram[freq_bin][hop_idx] = mag;
        }
    }

    Ok(spectrogram)
}

/// Find "peaks" per time slice — naive approach: pick top N frequencies by magnitude.
fn find_peaks(spectrogram: &Vec<Vec<f32>>) -> Vec<Vec<u16>> {
    // spectrogram[freq_bin][time]
    let n_freqs = spectrogram.len();
    if n_freqs == 0 {
        return Vec::new();
    }
    let n_hops = spectrogram[0].len();
    let top_n = 5;

    let mut peaks_by_time = Vec::with_capacity(n_hops);
    for time_idx in 0..n_hops {
        // gather (freq_bin, magnitude)
        let mut freq_mags: Vec<(u16, f32)> = (0..n_freqs)
            .map(|f| (f as u16, spectrogram[f][time_idx]))
            .collect();
        // sort by magnitude descending
        freq_mags.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
        // pick top N
        let top_peaks: Vec<u16> = freq_mags.into_iter().take(top_n).map(|(f, _)| f).collect();

        peaks_by_time.push(top_peaks);
    }

    peaks_by_time
}

/// Load or build a track’s fingerprint, then see how many collisions it has with `snippet_fp`.
async fn find_matches(
    track_path: &Path,
    snippet_fp: &FingerprintData,
    sample_rate: usize,
) -> eyre::Result<Option<(f32, usize)>> {
    // 1) Load or build track fingerprint
    let track_fp = load_or_build_fingerprint(track_path, sample_rate)?;

    // 2) Map (f1, f2, delta_t) -> list of anchor_times for the track
    //    We could store that directly in the fingerprint, or we can reconstruct it here.
    let mut track_map: HashMap<(u16, u16, u16), Vec<u32>> = HashMap::new();
    for hash_ent in &track_fp.pairs {
        let key = (hash_ent.f1, hash_ent.f2, hash_ent.delta_t);
        track_map.entry(key).or_default().push(hash_ent.anchor_time);
    }

    // 3) For each snippet hash, check collisions
    //    We'll compute an "offset difference" = track_anchor_time - snippet_anchor_time
    //    The best match is the offset that appears the most frequently
    let mut offset_count: HashMap<i32, usize> = HashMap::new();

    for snippet_ent in &snippet_fp.pairs {
        let key = (snippet_ent.f1, snippet_ent.f2, snippet_ent.delta_t);
        if let Some(track_times) = track_map.get(&key) {
            for &track_anchor_time in track_times {
                let diff = track_anchor_time as i32 - snippet_ent.anchor_time as i32;
                *offset_count.entry(diff).or_insert(0) += 1;
            }
        }
    }

    // 4) Find best offset by collisions
    if offset_count.is_empty() {
        return Ok(None);
    }
    let (best_offset, best_count) = offset_count.into_iter().max_by_key(|(_, c)| *c).unwrap();

    // 5) Convert that offset from spectrogram frames to seconds
    //    Each "time step" in the spectrogram corresponds to `hop_size / sample_rate` seconds.
    //    (We used hop_size=512 in the fingerprint, so offset in frames * 512 / sr)
    let hop_size = 512;
    let offset_sec = best_offset as f32 * (hop_size as f32 / sample_rate as f32);

    // If best_count is above some arbitrary threshold, consider it a match
    // For real usage, you'll want a more systematic approach
    if best_count > 5 {
        Ok(Some((offset_sec, best_count)))
    } else {
        Ok(None)
    }
}

/// Load from `hashes/` if possible, else build and save
fn load_or_build_fingerprint(
    track_path: &Path,
    sample_rate: usize,
) -> eyre::Result<FingerprintData> {
    let hash_dir = PathBuf::from("hashes");
    if !hash_dir.exists() {
        fs::create_dir_all(&hash_dir)?;
    }

    let file_stem = track_path.file_stem().unwrap_or_default().to_string_lossy();
    let hash_file = hash_dir.join(format!("{}.json", file_stem));

    if hash_file.exists() {
        // load
        debug!("Loading fingerprint from {:?}", hash_file);
        let f = File::open(&hash_file)?;
        let reader = BufReader::new(f);
        let data: FingerprintData = serde_json::from_reader(reader)?;
        Ok(data)
    } else {
        // build
        info!("Building fingerprint for {:?}", track_path);
        let pcm = decode_ogg_to_mono_f32(track_path)?;
        let data = compute_fingerprint(&pcm, sample_rate)?;
        // save
        let f = File::create(&hash_file)?;
        let writer = BufWriter::new(f);
        serde_json::to_writer_pretty(writer, &data)?;
        Ok(data)
    }
}
