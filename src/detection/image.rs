use super::{database::{ImageHashRecord, RuleDatabase}, Detection, DetectionKind};
use image::{imageops::FilterType, DynamicImage, GenericImageView, GrayImage, Luma};
use sha2::{Digest, Sha256};
use std::{collections::{HashMap, VecDeque}, sync::RwLock};

const HASH_BITS: f64 = 64.0;
const PHASH_PREFILTER: f64 = 0.70;
const MIN_PHASH_SIMILARITY: f64 = 0.70;
const MIN_DHASH_SIMILARITY: f64 = 0.70;
const PHASH_WEIGHT: f64 = 0.60;
const DHASH_WEIGHT: f64 = 0.40;
const AHASH_WEIGHT: f64 = 0.00;

#[derive(Clone, Debug)]
pub struct ImageMatch { pub filename: String, pub campaign: String, pub confidence: f64 }
#[derive(Clone, Debug)]
pub struct ImageScan { pub sha256: String, pub confidence: f64, pub matches: Vec<ImageMatch> }

pub struct ImageDetector {
    records: Vec<ImageHashRecord>, exact: HashMap<String, usize>,
    cache: RwLock<(HashMap<String, (u64, u64, u64)>, VecDeque<String>)>, cache_limit: usize, threshold: f64,
}
impl ImageDetector {
    pub fn from_rules(db: &RuleDatabase, cache_limit: usize, threshold: f64) -> Self {
        let exact = db.image_hashes.iter().enumerate().map(|(i, r)| (r.sha256.clone(), i)).collect();
        Self { records: db.image_hashes.clone(), exact, cache: RwLock::new((HashMap::new(), VecDeque::new())), cache_limit, threshold }
    }
    pub fn scan(&self, bytes: &[u8]) -> anyhow::Result<Option<ImageScan>> {
        self.scan_with_threshold(bytes, self.threshold)
    }
    pub fn scan_with_threshold(&self, bytes: &[u8], threshold: f64) -> anyhow::Result<Option<ImageScan>> {
        if bytes.is_empty() { return Ok(None); }
        let sha = hex::encode(Sha256::digest(bytes));
        if let Some(index) = self.exact.get(&sha) {
            let r = &self.records[*index];
            return Ok(Some(ImageScan { sha256: sha, confidence: 1.0, matches: vec![
                ImageMatch { filename: r.filename.clone(), campaign: r.campaign.clone(), confidence: 1.0 }
            ]}));
        }
        let cached = self.cache.read().ok().and_then(|cache| cache.0.get(&sha).copied());
        let hashes = if let Some(value) = cached {
            if let Ok(mut cache) = self.cache.write() {
                cache.1.retain(|key| key != &sha);
                cache.1.push_back(sha.clone());
            }
            value
        } else {
            let image = normalize_image(image::load_from_memory(bytes)?);
            let value = generate_hashes(&image);
            if let Ok(mut cache) = self.cache.write() {
                if self.cache_limit > 0 {
                    while cache.0.len() >= self.cache_limit {
                        if let Some(oldest) = cache.1.pop_front() { cache.0.remove(&oldest); }
                        else { break; }
                    }
                    cache.0.insert(sha.clone(), value);
                    cache.1.retain(|key| key != &sha);
                    cache.1.push_back(sha.clone());
                }
            }
            value
        };
        let mut matches = Vec::new();
        for record in &self.records {
            let p = similarity(hashes.0, record.phash);
            if p < PHASH_PREFILTER { continue; }
            let d = similarity(hashes.1, record.dhash);
            let a = similarity(hashes.2, record.ahash);
            let confidence = p * PHASH_WEIGHT + d * DHASH_WEIGHT + a * AHASH_WEIGHT;
            if confidence >= threshold && p >= MIN_PHASH_SIMILARITY && d >= MIN_DHASH_SIMILARITY {
                matches.push(ImageMatch { filename: record.filename.clone(), campaign: record.campaign.clone(), confidence });
            }
        }
        matches.sort_by(|a, b| b.confidence.total_cmp(&a.confidence));
        if matches.is_empty() { return Ok(None); }
        Ok(Some(ImageScan { sha256: sha, confidence: matches[0].confidence, matches }))
    }
}
fn normalize_image(image: DynamicImage) -> DynamicImage {
    let (w, h) = image.dimensions();
    let scale = (512.0 / w.max(h) as f64).min(1.0);
    if scale >= 1.0 { return image.to_rgb8().into(); }
    let nw = ((w as f64 * scale).round() as u32).max(1);
    let nh = ((h as f64 * scale).round() as u32).max(1);
    image.resize_exact(nw, nh, FilterType::CatmullRom).to_rgb8().into()
}
fn pillow_grayscale(image: &DynamicImage, width: u32, height: u32) -> GrayImage {
    let rgb = if image.width() == width && image.height() == height {
        image.to_rgb8()
    } else {
        image.resize_exact(width, height, FilterType::Lanczos3).to_rgb8()
    };
    GrayImage::from_fn(width, height, |x, y| {
        let pixel = rgb.get_pixel(x, y);
        // Pillow's RGB -> L conversion uses ITU-R 601 luma coefficients.
        let value = (19_595_u32 * pixel[0] as u32
            + 38_470_u32 * pixel[1] as u32
            + 7_471_u32 * pixel[2] as u32
            + 32_768) >> 16;
        Luma([value as u8])
    })
}
fn grayscale_square(image: &DynamicImage, size: u32) -> GrayImage {
    pillow_grayscale(image, size, size)
}
fn generate_hashes(image: &DynamicImage) -> (u64, u64, u64) {
    (perceptual_hash(image), difference_hash(image), average_hash(image))
}
fn perceptual_hash(image: &DynamicImage) -> u64 {
    const N: usize = 32;
    let gray = grayscale_square(image, N as u32);
    let mut coeff = [[0.0_f64; 8]; 8];
    for u in 0..8 { for v in 0..8 {
        let mut sum = 0.0;
        for x in 0..N { for y in 0..N {
            let pixel = gray.get_pixel(x as u32, y as u32)[0] as f64;
            let cu = ((std::f64::consts::PI * (2*y+1) as f64 * u as f64)/(2.0*N as f64)).cos();
            let cv = ((std::f64::consts::PI * (2*x+1) as f64 * v as f64)/(2.0*N as f64)).cos();
            sum += pixel * cu * cv;
        }}
        coeff[u][v] = sum;
    }}
    let mut values = Vec::with_capacity(64);
    for row in &coeff { for value in row { values.push(*value); } }
    let mut sorted = values.clone();
    sorted.sort_by(f64::total_cmp);
    let median = (sorted[31] + sorted[32]) / 2.0;
    let mut hash = 0_u64;
    for (i, value) in values.iter().enumerate() {
        if *value > median { hash |= 1_u64 << (63 - i); }
    }
    hash
}
fn difference_hash(image: &DynamicImage) -> u64 {
    let gray = pillow_grayscale(image, 9, 8);
    let mut hash = 0_u64;
    for y in 0..8 { for x in 0..8 {
        if gray.get_pixel(x+1, y)[0] > gray.get_pixel(x, y)[0] { hash |= 1_u64 << (63 - (y*8+x)); }
    }}
    hash
}
fn average_hash(image: &DynamicImage) -> u64 {
    let gray = grayscale_square(image, 8);
    let average = gray.pixels().map(|p| p[0] as u64).sum::<u64>() as f64 / 64.0;
    let mut hash = 0_u64;
    for (i, pixel) in gray.pixels().enumerate() { if pixel[0] as f64 > average { hash |= 1_u64 << (63 - i); } }
    hash
}
fn similarity(a: u64, b: u64) -> f64 { 1.0 - ((a ^ b).count_ones() as f64 / HASH_BITS) }

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn identical_hashes_have_full_similarity() { assert_eq!(similarity(0x1234, 0x1234), 1.0); }
    #[test]
    fn hashes_are_deterministic() {
        let image = DynamicImage::ImageRgb8(image::RgbImage::from_fn(32, 32, |x,y| image::Rgb([(x*7) as u8,(y*5) as u8,((x+y)*3) as u8])));
        assert_eq!(generate_hashes(&image), generate_hashes(&image));
    }
    #[test]
    fn hashes_match_python_imagehash_golden_vector() {
        // Golden values generated with Pillow + ImageHash (pHash, dHash, average_hash).
        let image = DynamicImage::ImageRgb8(image::RgbImage::from_fn(32, 32, |x,y| image::Rgb([(x*7) as u8,(y*5) as u8,((x+y)*3) as u8])));
        let (phash, dhash, ahash) = generate_hashes(&image);
        assert_eq!(format!("{phash:016x}"), "a218bb7c5f6d2a28");
        assert_eq!(format!("{dhash:016x}"), "ffffffffffffffff");
        assert_eq!(format!("{ahash:016x}"), "000103071f3f7fff");
    }
}
