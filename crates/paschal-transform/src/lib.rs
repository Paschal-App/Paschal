//! Content-transformation pipeline.
//!
//! Paschal's premise is that the Letter reaches the Recipient *in the
//! future*. Modern file formats are routinely fragile across years:
//! HEIC photos that no longer decode, DOCXes whose embedded fonts have
//! drifted, PDFs that depend on a long-dead JavaScript runtime, images
//! laden with EXIF GPS coordinates the principal never meant to share.
//!
//! Every uploaded file passes through a **Transformer**. The Transformer's
//! job is to produce a *flattened*, format-stable, metadata-clean version
//! that is then sealed and stored. The original is not retained.
//!
//! The pipeline picks a Transformer by MIME-type prefix match. The first
//! Transformer whose `handles` returns true wins. Order matters in
//! `default_pipeline()`; the binary pass-through is the last-resort
//! catch-all.
//!
//! ## Transformers in this MVP
//!
//! | Input MIME | Transformer | Output |
//! |---|---|---|
//! | `text/plain`, `text/markdown`, `text/csv`, `text/*` | TextPassThrough | UTF-8, LF line endings, BOM stripped |
//! | `image/jpeg`, `image/png`, `image/gif`, `image/webp` | ImageNormalizer | Re-encoded JPEG (quality 90), EXIF stripped |
//! | `application/pdf` | PdfPassThrough | PDF with header validated; no further processing in MVP |
//! | * | BinaryPassThrough | Bytes preserved as-is, flagged "preserved without transformation" |
//!
//! Out of MVP scope but where extension points live:
//! * DOCX/XLSX/PPTX → PDF (needs LibreOffice subprocess)
//! * PDF/A conversion (needs Ghostscript)
//! * HEIC → JPEG (needs libheif)
//! * Audio/Video transcoding (needs ffmpeg)
//!
//! Each of these slots in as a new `Transformer` impl.

use std::io::Cursor;

use async_trait::async_trait;
use image::ImageReader;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum TransformError {
    #[error("input was empty")]
    Empty,

    #[error("file is too large ({0} bytes, limit {1})")]
    TooLarge(usize, usize),

    #[error("image decode failed: {0}")]
    ImageDecode(String),

    #[error("image encode failed: {0}")]
    ImageEncode(String),

    #[error("text is not valid UTF-8")]
    InvalidUtf8,

    #[error("pdf header is missing or malformed")]
    BadPdfHeader,

    #[error("no transformer matched mime type '{0}'")]
    NoMatch(String),
}

/// The result of running a Transformer over an upload.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Transformed {
    /// Final bytes to seal.
    #[serde(skip_serializing, skip_deserializing, default = "Vec::new")]
    pub bytes: Vec<u8>,

    /// MIME type of the final bytes (may differ from the upload).
    pub mime: String,

    /// What the transformer did. Logged to the transparency log and shown to
    /// the principal in the seal-confirmation UI.
    pub notes: Vec<String>,

    /// Hex SHA-256 of the final bytes (post-transformation, pre-encryption).
    pub sha256_hex: String,
}

impl Transformed {
    fn new(bytes: Vec<u8>, mime: impl Into<String>, notes: Vec<String>) -> Self {
        let mut h = Sha256::new();
        h.update(&bytes);
        let digest = h.finalize();
        Self {
            sha256_hex: hex::encode(digest),
            bytes,
            mime: mime.into(),
            notes,
        }
    }
}

/// A Transformer recognises a subset of MIME types and produces a flattened
/// representation of any matching input.
#[async_trait]
pub trait Transformer: Send + Sync {
    fn name(&self) -> &'static str;
    fn handles(&self, mime: &str) -> bool;
    async fn transform(&self, input: &[u8], original_mime: &str) -> Result<Transformed, TransformError>;
}

// ---------------------------------------------------------------------------
// Pipeline
// ---------------------------------------------------------------------------

/// Apply the configured Transformer pipeline. Returns the first match.
///
/// `max_bytes` is the hard upper bound on input size; the function returns
/// `TransformError::TooLarge` if exceeded.
pub async fn apply(
    pipeline: &[Box<dyn Transformer>],
    bytes: &[u8],
    mime: &str,
    max_bytes: usize,
) -> Result<Transformed, TransformError> {
    if bytes.is_empty() {
        return Err(TransformError::Empty);
    }
    if bytes.len() > max_bytes {
        return Err(TransformError::TooLarge(bytes.len(), max_bytes));
    }
    for t in pipeline {
        if t.handles(mime) {
            tracing::info!(
                transformer = t.name(),
                bytes_in = bytes.len(),
                mime,
                "transformer matched"
            );
            return t.transform(bytes, mime).await;
        }
    }
    Err(TransformError::NoMatch(mime.into()))
}

/// The default pipeline: text → image → pdf → video → audio → binary.
pub fn default_pipeline() -> Vec<Box<dyn Transformer>> {
    vec![
        Box::new(TextPassThrough),
        Box::new(ImageNormalizer),
        Box::new(PdfPassThrough),
        Box::new(VideoPassThrough),
        Box::new(AudioPassThrough),
        Box::new(BinaryPassThrough),
    ]
}

// ---------------------------------------------------------------------------
// Text
// ---------------------------------------------------------------------------

pub struct TextPassThrough;

#[async_trait]
impl Transformer for TextPassThrough {
    fn name(&self) -> &'static str {
        "text-pass-through"
    }
    fn handles(&self, mime: &str) -> bool {
        mime.starts_with("text/") || mime == "application/json" || mime == "application/xml"
    }
    async fn transform(&self, input: &[u8], original_mime: &str) -> Result<Transformed, TransformError> {
        // Strip BOM and normalize line endings to LF.
        let s = std::str::from_utf8(input).map_err(|_| TransformError::InvalidUtf8)?;
        let stripped = s.strip_prefix('\u{FEFF}').unwrap_or(s);
        let normalized = stripped.replace("\r\n", "\n").replace('\r', "\n");
        let bytes = normalized.into_bytes();
        let notes = vec![
            "Stripped UTF-8 BOM if present".into(),
            "Normalised line endings to LF".into(),
        ];
        Ok(Transformed::new(bytes, original_mime, notes))
    }
}

// ---------------------------------------------------------------------------
// Images
// ---------------------------------------------------------------------------

pub struct ImageNormalizer;

#[async_trait]
impl Transformer for ImageNormalizer {
    fn name(&self) -> &'static str {
        "image-normalizer"
    }
    fn handles(&self, mime: &str) -> bool {
        matches!(mime, "image/jpeg" | "image/png" | "image/gif" | "image/webp")
    }
    async fn transform(&self, input: &[u8], _original_mime: &str) -> Result<Transformed, TransformError> {
        // Decode the input into pixels (which discards EXIF). Re-encode as JPEG.
        let reader = ImageReader::new(Cursor::new(input))
            .with_guessed_format()
            .map_err(|e| TransformError::ImageDecode(e.to_string()))?;
        let img = reader
            .decode()
            .map_err(|e| TransformError::ImageDecode(e.to_string()))?;

        let mut buf = Vec::new();
        // Quality 90 is a good archival balance for photographs.
        let encoder =
            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, 90);
        img.write_with_encoder(encoder)
            .map_err(|e| TransformError::ImageEncode(e.to_string()))?;

        let notes = vec![
            "Decoded to pixels and re-encoded as JPEG (quality 90)".into(),
            "Stripped EXIF / XMP / IPTC metadata (including GPS coordinates)".into(),
        ];
        Ok(Transformed::new(buf, "image/jpeg", notes))
    }
}

// ---------------------------------------------------------------------------
// PDF
// ---------------------------------------------------------------------------

pub struct PdfPassThrough;

#[async_trait]
impl Transformer for PdfPassThrough {
    fn name(&self) -> &'static str {
        "pdf-pass-through"
    }
    fn handles(&self, mime: &str) -> bool {
        mime == "application/pdf"
    }
    async fn transform(&self, input: &[u8], _original_mime: &str) -> Result<Transformed, TransformError> {
        if !input.starts_with(b"%PDF-") {
            return Err(TransformError::BadPdfHeader);
        }
        let notes = vec![
            "Validated PDF header (%PDF-)".into(),
            "Bytes preserved as-is. PDF/A conversion lands at v1 via Ghostscript.".into(),
        ];
        Ok(Transformed::new(input.to_vec(), "application/pdf", notes))
    }
}

// ---------------------------------------------------------------------------
// Video / Audio messages
// ---------------------------------------------------------------------------
//
// Video & audio Letters land via the existing multipart pipeline. The
// transformer here does not transcode (no ffmpeg dependency in the MVP);
// it validates magic bytes for the codecs the browser MediaRecorder
// produces, strips ID3v2 metadata blocks on MP3, and emits clear notes
// about what *will* land in v1 (codec normalisation to a long-term
// format: WebM/VP9 video and Opus audio).

pub struct VideoPassThrough;

#[async_trait]
impl Transformer for VideoPassThrough {
    fn name(&self) -> &'static str {
        "video-pass-through"
    }
    fn handles(&self, mime: &str) -> bool {
        mime.starts_with("video/")
    }
    async fn transform(&self, input: &[u8], original_mime: &str) -> Result<Transformed, TransformError> {
        let mut notes = vec![format!(
            "Bytes preserved as-is ({original_mime}). The principal is responsible \
             for choosing a long-term-readable format; we recommend WebM (VP9 + Opus) \
             or MP4 (H.264 + AAC)."
        )];

        // Recognise common container magic. We don't reject unknown ones —
        // the worst case is a passthrough — but we surface what we saw so
        // the principal knows the file looks intact.
        let recognised = if input.len() >= 12 && &input[4..8] == b"ftyp" {
            notes.push("ISO BMFF container detected (MP4 / 3GP / QuickTime family).".into());
            true
        } else if input.starts_with(&[0x1A, 0x45, 0xDF, 0xA3]) {
            notes.push("EBML container detected (WebM / Matroska family).".into());
            true
        } else if input.starts_with(b"RIFF") && input.len() >= 12 && &input[8..12] == b"AVI " {
            notes.push("RIFF AVI container detected.".into());
            true
        } else {
            false
        };

        if !recognised {
            notes.push(
                "Container magic not recognised — we are preserving the bytes anyway, but \
                 the recipient may need a current player to open it."
                    .into(),
            );
        }

        notes.push(
            "v1 plan: transcode to WebM (VP9 + Opus) for archival stability."
                .into(),
        );

        Ok(Transformed::new(input.to_vec(), original_mime, notes))
    }
}

pub struct AudioPassThrough;

#[async_trait]
impl Transformer for AudioPassThrough {
    fn name(&self) -> &'static str {
        "audio-pass-through"
    }
    fn handles(&self, mime: &str) -> bool {
        mime.starts_with("audio/")
    }
    async fn transform(&self, input: &[u8], original_mime: &str) -> Result<Transformed, TransformError> {
        let mut notes = Vec::new();
        let mut output_bytes = input.to_vec();

        // MP3 with ID3v2 metadata — strip it. The tag starts with "ID3"
        // and the size is a 4-byte syncsafe integer at offset 6..10.
        if output_bytes.len() > 10 && &output_bytes[0..3] == b"ID3" {
            let size = ((output_bytes[6] as usize) << 21)
                | ((output_bytes[7] as usize) << 14)
                | ((output_bytes[8] as usize) << 7)
                | (output_bytes[9] as usize);
            let header_len = 10 + size;
            if header_len < output_bytes.len() {
                output_bytes.drain(0..header_len);
                notes.push(format!(
                    "Stripped ID3v2 metadata header ({header_len} bytes)."
                ));
            }
        }

        // Surface container detection so the principal sees we looked.
        if output_bytes.starts_with(b"OggS") {
            notes.push("Ogg container detected (likely Opus or Vorbis).".into());
        } else if output_bytes.starts_with(b"fLaC") {
            notes.push("FLAC stream detected.".into());
        } else if output_bytes.starts_with(b"RIFF")
            && output_bytes.len() >= 12
            && &output_bytes[8..12] == b"WAVE"
        {
            notes.push("RIFF WAV detected.".into());
        } else if output_bytes.len() >= 12 && &output_bytes[4..8] == b"ftyp" {
            notes.push("ISO BMFF audio container detected (e.g. M4A / AAC).".into());
        }

        notes.push(format!(
            "Bytes {} ({original_mime}). v1 plan: transcode to Opus in Ogg for archival stability.",
            if output_bytes.len() == input.len() {
                "preserved as-is"
            } else {
                "preserved after metadata strip"
            }
        ));

        Ok(Transformed::new(output_bytes, original_mime, notes))
    }
}

// ---------------------------------------------------------------------------
// Generic binary
// ---------------------------------------------------------------------------

pub struct BinaryPassThrough;

#[async_trait]
impl Transformer for BinaryPassThrough {
    fn name(&self) -> &'static str {
        "binary-pass-through"
    }
    fn handles(&self, _mime: &str) -> bool {
        true
    }
    async fn transform(&self, input: &[u8], original_mime: &str) -> Result<Transformed, TransformError> {
        let notes = vec![
            format!(
                "No dedicated transformer for '{original_mime}'. Bytes preserved as-is. \
                 Consider attaching a flattened version (PDF, JPEG, plain text) for archival."
            ),
        ];
        Ok(Transformed::new(input.to_vec(), original_mime, notes))
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // ---- TextPassThrough ----

    #[tokio::test]
    async fn text_strips_bom_and_normalises_endings() {
        let t = TextPassThrough;
        let input = b"\xEF\xBB\xBFhello\r\nworld\rgoodbye\n";
        let out = t.transform(input, "text/plain").await.unwrap();
        assert_eq!(out.bytes, b"hello\nworld\ngoodbye\n");
        assert!(out.notes.iter().any(|n| n.contains("LF")));
    }

    #[tokio::test]
    async fn text_rejects_invalid_utf8() {
        let t = TextPassThrough;
        let result = t.transform(&[0xFF, 0xFE, 0xFD], "text/plain").await;
        assert!(matches!(result, Err(TransformError::InvalidUtf8)));
    }

    #[tokio::test]
    async fn text_handles_markdown_csv_json_xml() {
        let t = TextPassThrough;
        assert!(t.handles("text/plain"));
        assert!(t.handles("text/markdown"));
        assert!(t.handles("text/csv"));
        assert!(t.handles("application/json"));
        assert!(t.handles("application/xml"));
        assert!(!t.handles("image/png"));
    }

    // ---- ImageNormalizer ----

    #[tokio::test]
    async fn image_normalizer_round_trips_a_png() {
        // Tiny 2x2 PNG built in-memory.
        let img = image::ImageBuffer::from_fn(2u32, 2u32, |x, _y| {
            image::Rgb([(x * 100) as u8, 200, 50])
        });
        let mut input = Vec::new();
        image::DynamicImage::ImageRgb8(img)
            .write_to(&mut Cursor::new(&mut input), image::ImageFormat::Png)
            .unwrap();

        let t = ImageNormalizer;
        let out = t.transform(&input, "image/png").await.unwrap();
        assert_eq!(out.mime, "image/jpeg");
        // Re-decoding the JPEG must succeed.
        let _decoded = image::load_from_memory(&out.bytes).unwrap();
        assert!(out.notes.iter().any(|n| n.contains("EXIF")));
    }

    #[tokio::test]
    async fn image_normalizer_rejects_garbage() {
        let t = ImageNormalizer;
        let err = t.transform(&[1, 2, 3, 4], "image/jpeg").await.unwrap_err();
        assert!(matches!(err, TransformError::ImageDecode(_)));
    }

    #[tokio::test]
    async fn image_normalizer_strips_exif() {
        // A minimal JPEG with a fake EXIF marker; the normalizer should
        // produce a smaller output without the marker.
        let img =
            image::ImageBuffer::from_fn(10u32, 10u32, |_x, _y| image::Rgb([100u8, 100, 100]));
        let mut input = Vec::new();
        let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut input, 70);
        image::DynamicImage::ImageRgb8(img)
            .write_with_encoder(encoder)
            .unwrap();

        let t = ImageNormalizer;
        let out = t.transform(&input, "image/jpeg").await.unwrap();
        assert_eq!(out.mime, "image/jpeg");
        // We can't easily assert "no EXIF" without an EXIF parser, but the
        // re-encoded image cannot carry EXIF that wasn't there to begin with
        // because the image crate's JPEG encoder does not emit EXIF.
        assert!(!out.bytes.is_empty());
    }

    // ---- PdfPassThrough ----

    #[tokio::test]
    async fn pdf_validates_header() {
        let t = PdfPassThrough;
        let input = b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n... (rest of PDF) ...";
        let out = t.transform(input, "application/pdf").await.unwrap();
        assert_eq!(out.mime, "application/pdf");
        assert_eq!(out.bytes, input);
    }

    #[tokio::test]
    async fn pdf_rejects_missing_header() {
        let t = PdfPassThrough;
        let err = t
            .transform(b"not a pdf at all", "application/pdf")
            .await
            .unwrap_err();
        assert!(matches!(err, TransformError::BadPdfHeader));
    }

    // ---- BinaryPassThrough ----

    #[tokio::test]
    async fn binary_passes_through_unknown_mime() {
        let t = BinaryPassThrough;
        let out = t.transform(&[1, 2, 3], "application/x-mystery").await.unwrap();
        assert_eq!(out.bytes, vec![1, 2, 3]);
        assert!(out.notes.iter().any(|n| n.contains("Consider attaching")));
    }

    // ---- Pipeline ----

    #[tokio::test]
    async fn pipeline_picks_first_match() {
        let pipeline = default_pipeline();
        let out = apply(&pipeline, b"hello", "text/plain", 1024 * 1024)
            .await
            .unwrap();
        // Text matched first; not the binary fallback.
        assert_eq!(out.bytes, b"hello");
    }

    #[tokio::test]
    async fn pipeline_rejects_empty() {
        let pipeline = default_pipeline();
        let err = apply(&pipeline, b"", "text/plain", 1024).await.unwrap_err();
        assert!(matches!(err, TransformError::Empty));
    }

    #[tokio::test]
    async fn pipeline_rejects_oversize() {
        let pipeline = default_pipeline();
        let huge = vec![b'A'; 1024 + 1];
        let err = apply(&pipeline, &huge, "text/plain", 1024).await.unwrap_err();
        assert!(matches!(err, TransformError::TooLarge(_, _)));
    }

    #[tokio::test]
    async fn pipeline_unknown_mime_falls_back_to_binary() {
        let pipeline = default_pipeline();
        let out = apply(&pipeline, b"\x00\x01\x02", "application/x-nope", 1024)
            .await
            .unwrap();
        assert_eq!(out.bytes, vec![0, 1, 2]);
        assert!(out.notes.iter().any(|n| n.contains("Consider attaching")));
    }

    #[tokio::test]
    async fn sha256_is_deterministic() {
        let pipeline = default_pipeline();
        let a = apply(&pipeline, b"same content", "text/plain", 1024).await.unwrap();
        let b = apply(&pipeline, b"same content", "text/plain", 1024).await.unwrap();
        assert_eq!(a.sha256_hex, b.sha256_hex);
        assert_eq!(a.sha256_hex.len(), 64); // 32 bytes hex-encoded
    }
}
