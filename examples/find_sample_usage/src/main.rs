use eyre::OptionExt;
use eyre::WrapErr;
use eyre::eyre;
use ndarray::Array2;
use ndarray::s;
use phantasy_init::init;
use rustfft::FftPlanner;
use rustfft::num_complex::Complex;
use rustfft::num_traits::Zero;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::fs;
use tokio::process::Command;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;
use tracing::debug;
use tracing::info;
use tracing::warn;

#[tokio::main]
async fn main() -> eyre::Result<()> {
    init()?;

    let music_dir = PathBuf::from(var("MUSIC_DIR")?);
    let mut sample_path = PathBuf::from(var("SAMPLE_PATH")?);
    if sample_path.extension().map_or(false, |ext| ext != "ogg") {
        let new_sample_path = sample_path.with_extension("ogg");
        if !new_sample_path.exists() {
            info!("Converting sample to OGG: {:?}", sample_path);
            let mut cmd = Command::new("ffmpeg");
            cmd.current_dir(
                &sample_path
                    .parent()
                    .ok_or_eyre(format!("Invalid sample path: {:?}", sample_path))?,
            );
            cmd.args(&["-i", &sample_path.to_string_lossy()]);
            cmd.arg("-vn"); // drop video streams
            cmd.arg("-c:a").arg("libvorbis");
            cmd.arg("-q:a").arg("5");
            cmd.arg("-y").arg(
                new_sample_path
                    .file_name()
                    .ok_or_eyre("Failed to get file name")?,
            );
            let status = cmd.status().await?;
            if !status.success() {
                return Err(eyre!("ffmpeg failed: {:?}", status));
            }
        }
        sample_path = new_sample_path;
    }
    let sample_begin = var("SAMPLE_BEGIN")?.parse::<f32>()?;
    let sample_end = var("SAMPLE_END")?.parse::<f32>()?;

    // --- 1) Decode the sample snippet
    info!("Decoding sample snippet from {:?}", sample_path);
    let sample_pcm =
        decode_ogg_to_mono_f32(&sample_path).wrap_err("Failed to decode sample OGG")?;

    // We only care about the portion [sample_begin, sample_end] in seconds
    let sample_rate = 48000.0; // Hardcode or detect from decode
    let start_idx = (sample_begin * sample_rate) as usize;
    let end_idx = (sample_end * sample_rate) as usize;
    let snippet = &sample_pcm[start_idx.min(sample_pcm.len())..end_idx.min(sample_pcm.len())];

    // Create spectrogram for sample snippet
    let snippet_spectrogram = compute_spectrogram(snippet, sample_rate as usize)?;
    info!("Sample spectrogram shape: {:?}", snippet_spectrogram.dim());

    // --- 2) Scan the target directory for .ogg files
    let mut dir = fs::read_dir(&music_dir).await?;
    let mut ogg_paths = Vec::new();
    while let Some(entry) = dir.next_entry().await? {
        let path = entry.path();
        if path.extension().map_or(false, |ext| ext == "ogg") {
            ogg_paths.push(path);
        }
    }
    info!("Found {} OGG files in {:?}", ogg_paths.len(), music_dir);

    // --- 3) Compare sample snippet to each .ogg track
    let mut join_set: JoinSet<eyre::Result<_>> = JoinSet::new();
    let rate_limit = Arc::new(Semaphore::new(4));
    for path in ogg_paths {
        let path = path.clone();
        let snippet_spectrogram = snippet_spectrogram.clone();
        let rate_limit = rate_limit.clone();
        join_set.spawn(async move {
            let _permit = rate_limit.acquire().await;
            match process_one_track(&path, &snippet_spectrogram, sample_rate as usize).await {
                Ok(Some((best_offset_sec, best_score))) => {
                    info!(
                        "Potential match in {} at ~{:.2} seconds, score={:.4}",
                        path.display(),
                        best_offset_sec,
                        best_score
                    );
                }
                Ok(None) => {
                    info!("No strong match in {}", path.display());
                }
                Err(e) => {
                    warn!("Skipping {} due to error: {:?}", path.display(), e);
                }
            }
            Ok(())
        });
    }
    use std::time::Instant;

    let total_tasks = join_set.len();
    let start_time = Instant::now();

    while let Some(task) = join_set.join_next().await {
        task??; // Handle the result of the task
        let completed = total_tasks - join_set.len();
        let elapsed = start_time.elapsed();

        if completed > 0 {
            // Calculate the average time per task
            let avg_duration = elapsed / completed as u32;
            // Estimate the remaining time
            let eta = avg_duration * join_set.len() as u32;
            info!("{} tasks remain, ETA: {:?}", join_set.len(), eta);
        } else {
            info!("Calculating ETA...");
        }
    }

    Ok(())
}

// ENV helper
fn var(key: &str) -> eyre::Result<String> {
    std::env::var(key).map_err(|_| eyre!("Missing env var: {}", key))
}

/// Decode an OGG file to raw mono f32 PCM.
/// This version explicitly uses i16 as the sample type.
fn decode_ogg_to_mono_f32(path: &PathBuf) -> eyre::Result<Vec<f32>> {
    use std::fs::File;
    use std::io::BufReader;
    let file = File::open(path)?;
    let mut reader = BufReader::new(file);
    let mut ogg_reader = lewton::inside_ogg::OggStreamReader::new(&mut reader)?;

    let mut pcm = Vec::new();
    // Explicitly specify the sample type
    while let Some(packet) = ogg_reader.read_dec_packet_generic::<Vec<Vec<i16>>>()? {
        // packet is Vec<Vec<i16>>
        let num_channels = packet.len();
        let samples_in_channel = packet[0].len();
        for i in 0..samples_in_channel {
            let mut sum = 0.0;
            for channel in &packet {
                sum += *channel.get(i).unwrap() as f32;
            }
            let avg = sum / num_channels as f32;
            pcm.push(avg);
        }
    }
    Ok(pcm)
}

/// Compute a simple magnitude spectrogram of a PCM buffer.
/// window_size: e.g., 2048 samples
/// hop_size: e.g., 512 samples
fn compute_spectrogram(pcm: &[f32], _sample_rate: usize) -> eyre::Result<Array2<f32>> {
    let window_size = 2048;
    let hop_size = 512;
    let mut planner = FftPlanner::new();
    let fft = planner.plan_fft_forward(window_size);

    // Number of columns in our spectrogram
    let n_hops = (pcm.len().saturating_sub(window_size)) / hop_size + 1;
    // We'll store magnitude for each frequency bin
    let n_freqs = window_size / 2; // ignoring the Nyquist duplicate

    let mut spectrogram = ndarray::Array2::<f32>::zeros((n_freqs, n_hops));
    let mut buffer = vec![Complex::zero(); window_size];

    // Hann window (optional)
    let window_func: Vec<f32> = (0..window_size)
        .map(|i| 0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / (window_size as f32)).cos())
        .collect();

    for hop_idx in 0..n_hops {
        let offset = hop_idx * hop_size;
        // Fill buffer and apply window
        for i in 0..window_size {
            let sample = pcm[offset + i];
            buffer[i].re = sample * window_func[i];
            buffer[i].im = 0.0;
        }

        // Process FFT in place
        fft.process(&mut buffer);

        // Compute magnitude for each frequency bin
        for freq_bin in 0..n_freqs {
            let re = buffer[freq_bin].re;
            let im = buffer[freq_bin].im;
            let mag = (re * re + im * im).sqrt();
            spectrogram[[freq_bin, hop_idx]] = mag;
        }
    }

    Ok(spectrogram)
}

/// Process a single track: decode OGG, compute spectrogram, do sliding-window compare.
///
/// Returns Some((best_offset_seconds, best_score)) if there's a match above some threshold.
/// Returns None if no match found.
async fn process_one_track(
    path: &PathBuf,
    snippet_spec: &Array2<f32>,
    sample_rate: usize,
) -> eyre::Result<Option<(f32, f32)>> {
    debug!("Processing {:?}", path);
    let pcm = tokio::task::spawn_blocking({
        let p = path.clone();
        debug!("Decoding {:?}", p);
        move || decode_ogg_to_mono_f32(&p)
    })
    .await??;
    debug!("Computing spectrogram for {:?}", path);
    let track_spec = compute_spectrogram(&pcm, sample_rate)?;

    // Slide snippet_spec over track_spec in the "time" dimension.
    let snippet_cols = snippet_spec.shape()[1];
    let track_cols = track_spec.shape()[1];
    if snippet_cols > track_cols {
        return Ok(None);
    }

    let mut best_score = f32::MIN;
    let mut best_col = 0_usize;

    // For each possible alignment, compute cosine similarity.
    let end = track_cols - snippet_cols;
    debug!("Comparing spectrograms for {:?}, will take {end} iterations", path);
    for col_start in 0..=end {
        let window = track_spec.slice(s![.., col_start..col_start + snippet_cols]);
        // Convert the view to an owned array to match the expected type.
        let score = cosine_similarity(snippet_spec, &window.to_owned())?;
        if score > best_score {
            best_score = score;
            best_col = col_start;
        }
    }

    // Define a match threshold.
    let match_threshold = 0.5;
    if best_score >= match_threshold {
        // Each spectrogram column advances by hop_size samples.
        let hop_size = 512;
        let offset_samples = best_col * hop_size;
        let offset_sec = offset_samples as f32 / sample_rate as f32;
        Ok(Some((offset_sec, best_score)))
    } else {
        Ok(None)
    }
}

/// Cosine similarity between two same-shaped 2D arrays.
fn cosine_similarity(a: &Array2<f32>, b: &Array2<f32>) -> eyre::Result<f32> {
    if a.shape() != b.shape() {
        return Err(eyre!(
            "cosine_similarity: shape mismatch: {:?} vs {:?}",
            a.shape(),
            b.shape()
        ));
    }
    let mut dot = 0.0;
    let mut norm_a = 0.0;
    let mut norm_b = 0.0;

    for (x, y) in a.iter().zip(b.iter()) {
        dot += x * y;
        norm_a += x * x;
        norm_b += y * y;
    }

    norm_a = norm_a.sqrt();
    norm_b = norm_b.sqrt();

    if norm_a < 1e-9 || norm_b < 1e-9 {
        Ok(0.0)
    } else {
        Ok(dot / (norm_a * norm_b))
    }
}
