//! Comparador visual.
//!
//! `visual-comparator REFERENCIA.png NATIVO.png [--diff DIFERENCA.png] [--min-psnr 20]`
//!
//! Mede a fidelidade da captura nativa em relação à captura de referência do cliente web (mesma câmera, mesma resolução):
//! PSNR em dB, diferença média por canal (0–255) e porcentagem de pixels com diferença perceptível (> 24 em algum canal).
//! Com `--diff`, grava um mapa de calor das diferenças. Sai com código 1 se o PSNR ficar abaixo de `--min-psnr`.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

use image::{Rgb, RgbImage};
use std::process::ExitCode;

#[derive(Debug, PartialEq)]
pub struct Score {
    pub psnr: f64,
    pub mean_abs: f64,
    pub changed_pct: f64,
}

pub fn compare(a: &RgbImage, b: &RgbImage) -> Option<(Score, RgbImage)> {
    if a.dimensions() != b.dimensions() {
        return None;
    }
    let (w, h) = a.dimensions();
    let mut se = 0f64;
    let mut abs = 0f64;
    let mut changed = 0u64;
    let mut diff = RgbImage::new(w, h);
    for (x, y, pa) in a.enumerate_pixels() {
        let pb = b.get_pixel(x, y);
        let mut m = 0u8;
        for c in 0..3 {
            let d = (i16::from(pa[c]) - i16::from(pb[c])).unsigned_abs() as u8;
            m = m.max(d);
            abs += f64::from(d);
            se += f64::from(d) * f64::from(d);
        }
        if m > 24 {
            changed += 1;
        }
        // cinza = igual, vermelho = muito diferente
        let g = pa.0.iter().map(|&v| u16::from(v)).sum::<u16>() / 12;
        diff.put_pixel(x, y, Rgb([(g as u8).saturating_add(m.saturating_mul(2)), g as u8, g as u8]));
    }
    let n = f64::from(w) * f64::from(h) * 3.0;
    let mse = se / n;
    let psnr = if mse == 0.0 { f64::INFINITY } else { 10.0 * (255.0f64 * 255.0 / mse).log10() };
    Some((Score { psnr, mean_abs: abs / n, changed_pct: changed as f64 * 100.0 / (f64::from(w) * f64::from(h)) }, diff))
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let opt = |k: &str| args.iter().position(|a| a == k).and_then(|i| args.get(i + 1)).cloned();
    let files: Vec<&String> = args
        .iter()
        .enumerate()
        .filter(|(i, a)| !a.starts_with("--") && (*i == 0 || !args[i - 1].starts_with("--")))
        .map(|(_, a)| a)
        .collect();
    let [ra, rb] = files.as_slice() else {
        eprintln!("uso: visual-comparator REFERENCIA.png NATIVO.png [--diff DIFERENCA.png] [--min-psnr 20]");
        return ExitCode::from(2);
    };
    let load = |p: &str| image::open(p).map(|i| i.to_rgb8()).map_err(|e| format!("{p}: {e}"));
    let (a, b) = match (load(ra), load(rb)) {
        (Ok(a), Ok(b)) => (a, b),
        (Err(e), _) | (_, Err(e)) => {
            eprintln!("{e}");
            return ExitCode::from(2);
        }
    };
    let Some((s, diff)) = compare(&a, &b) else {
        eprintln!("tamanhos diferentes: {:?} e {:?}", a.dimensions(), b.dimensions());
        return ExitCode::from(2);
    };
    println!("PSNR {:.2} dB · diferença média {:.2} · pixels diferentes {:.1}%", s.psnr, s.mean_abs, s.changed_pct);
    if let Some(p) = opt("--diff")
        && let Err(e) = diff.save(&p)
    {
        eprintln!("{p}: {e}");
    }
    let min: f64 = opt("--min-psnr").and_then(|v| v.parse().ok()).unwrap_or(0.0);
    if s.psnr < min { ExitCode::from(1) } else { ExitCode::SUCCESS }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_and_different() {
        let a = RgbImage::from_pixel(4, 4, Rgb([10, 20, 30]));
        let (s, _) = compare(&a, &a).unwrap();
        assert!(s.psnr.is_infinite());
        assert_eq!(s.changed_pct, 0.0);
        let b = RgbImage::from_pixel(4, 4, Rgb([60, 20, 30]));
        let (s, _) = compare(&a, &b).unwrap();
        assert_eq!(s.changed_pct, 100.0);
        assert!(s.psnr < 30.0);
        assert!(compare(&a, &RgbImage::new(2, 2)).is_none());
    }
}
