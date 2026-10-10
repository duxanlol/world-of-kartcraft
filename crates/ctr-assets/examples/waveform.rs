//! Plots every WAV in a directory as a waveform strip (PNG) and prints duration, peak, RMS and,
//! for looped files (`smpl` chunk), the loop seam jump compared with the typical sample step.
//! Looped files are drawn twice back to back (separated by a red tick) to show the seam.
//!
//! `cargo run -p ctr-assets --example waveform -- kart-data/audio kart-data/audio-waveforms.png`

const W: usize = 1200;
const ROW: usize = 90;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let dir = std::path::Path::new(&args[1]);
    let mut files: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "wav"))
        .collect();
    files.sort();
    let mut img = vec![[24u8, 26, 32]; W * ROW * files.len()];
    for (row, f) in files.iter().enumerate() {
        let d = std::fs::read(f).unwrap();
        let channels = u16::from_le_bytes([d[22], d[23]]).max(1) as usize;
        let rate = u32::from_le_bytes(d[24..28].try_into().unwrap());
        let len = u32::from_le_bytes(d[40..44].try_into().unwrap()) as usize;
        // First (left) channel.
        let pcm: Vec<i16> = d[44..44 + len]
            .chunks(2 * channels)
            .map(|c| i16::from_le_bytes([c[0], c[1]]))
            .collect();
        let looped = d.windows(4).any(|w| w == b"smpl");
        let peak = pcm.iter().map(|s| s.saturating_abs()).max().unwrap_or(0);
        let rms =
            (pcm.iter().map(|&s| (s as f64).powi(2)).sum::<f64>() / pcm.len().max(1) as f64).sqrt();
        let steps: Vec<i32> = pcm
            .windows(2)
            .map(|w| (w[1] as i32 - w[0] as i32).abs())
            .collect();
        let mean_step = steps.iter().map(|&s| s as f64).sum::<f64>() / steps.len().max(1) as f64;
        let seam = (pcm[0] as i32 - *pcm.last().unwrap() as i32).abs();
        println!(
            "{:16} {:6} Hz {:6.3} s  peak {:5}  rms {:7.0}{}",
            f.file_name().unwrap().to_string_lossy(),
            rate,
            pcm.len() as f64 / rate as f64,
            peak,
            rms,
            if looped {
                format!("  loop seam step {seam} (mean step {mean_step:.0})")
            } else {
                String::new()
            }
        );
        let shown: Vec<i16> = if looped {
            pcm.iter().chain(pcm.iter()).copied().collect()
        } else {
            pcm.clone()
        };
        let y0 = row * ROW;
        for x in 0..W {
            let a = x * shown.len() / W;
            let b = ((x + 1) * shown.len() / W).max(a + 1).min(shown.len());
            let (lo, hi) = shown[a..b]
                .iter()
                .fold((i16::MAX, i16::MIN), |(l, h), &s| (l.min(s), h.max(s)));
            let to_y = |s: i16| (ROW / 2) as i32 - (s as i32 * (ROW as i32 / 2 - 4)) / 32768;
            for y in to_y(hi)..=to_y(lo) {
                img[(y0 + y as usize) * W + x] = [120, 200, 255];
            }
            img[(y0 + ROW / 2) * W + x] = [70, 70, 80];
        }
        if looped {
            for y in 0..ROW {
                img[(y0 + y) * W + W / 2] = [255, 60, 60];
            }
        }
        for x in 0..W {
            img[(y0 + ROW - 1) * W + x] = [60, 60, 70];
        }
    }
    let file = std::fs::File::create(&args[2]).unwrap();
    let mut enc = png::Encoder::new(
        std::io::BufWriter::new(file),
        W as u32,
        (ROW * files.len()) as u32,
    );
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    let data: Vec<u8> = img.iter().flatten().copied().collect();
    enc.write_header().unwrap().write_image_data(&data).unwrap();
    println!("wrote {} (rows in file-name order)", args[2]);
}
