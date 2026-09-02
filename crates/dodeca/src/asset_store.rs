//! Remote, content-addressed store for derived assets.
//!
//! A derived asset (an image variant, a video copied as-is) is a pure function
//! of its input's content hash, the pipeline version and the derivation
//! parameters, so it can live in an S3-compatible bucket under a key made of
//! exactly those — `d/<hash>-<width>w.webp`, `p/<hash>.mp4` — and never needs
//! to be produced twice by anyone.
//!
//! The bucket also holds a small *index* (`index/inputs.v1.bin`): the set of
//! input hashes whose derivations have all been uploaded. A build fetches it
//! once, derives and uploads only inputs that aren't in it, then writes the
//! index back (uploads first, index last, so the index never over-reports).
//! A lost race between two builds can only make the index under-report, which
//! costs a redundant re-derive and nothing else.
//!
//! Only `ddc build` talks to the store, and only with write credentials from
//! the environment; `ddc serve` derives locally and never touches it.

use crate::cas::{IMAGE_PIPELINE_VERSION, InputHash};
use crate::image::OutputFormat;
use dodeca_config::AssetStoreConfig;
use eyre::{Context as _, eyre};
use rusty_s3::{Bucket, Credentials, S3Action, UrlStyle};
use std::collections::HashSet;
use std::time::Duration;

/// Key of the input index object.
pub const INDEX_KEY: &str = "index/inputs.v2.bin";

pub const ACCESS_KEY_ENV: &str = "DODECA_ASSET_STORE_ACCESS_KEY_ID";
pub const SECRET_KEY_ENV: &str = "DODECA_ASSET_STORE_SECRET_ACCESS_KEY";

/// Static-file extensions stored remotely as-is when the site doesn't list
/// its own `passthrough_extensions`.
pub const DEFAULT_PASSTHROUGH_EXTENSIONS: &[&str] =
    &["mp4", "webm", "mov", "m4a", "mp3", "flac", "ogg", "wav"];

/// Image metadata remembered alongside an indexed input, so a build on a
/// machine without a local cache can reference an image's variants without
/// decoding it.
#[derive(Debug, Clone, facet::Facet)]
pub struct IndexedImage {
    pub input_hash: InputHash,
    pub metadata: crate::image::ImageMetadata,
}

/// The set of inputs whose derivations are known to be in the bucket.
#[derive(Debug, Clone, Default, facet::Facet)]
pub struct AssetIndex {
    /// `IMAGE_PIPELINE_VERSION` the entries were derived with. Input hashes
    /// already mix the version in, so this is a belt-and-braces check.
    pub pipeline_version: u64,
    pub inputs: Vec<InputHash>,
    /// Metadata for the image inputs among `inputs`.
    pub images: Vec<IndexedImage>,
}

impl AssetIndex {
    pub fn from_bytes(bytes: &[u8]) -> eyre::Result<Self> {
        let index: AssetIndex =
            facet_postcard::from_slice(bytes).map_err(|e| eyre!("decoding asset index: {e}"))?;
        if index.pipeline_version != IMAGE_PIPELINE_VERSION {
            tracing::warn!(
                index_version = index.pipeline_version,
                current = IMAGE_PIPELINE_VERSION,
                "asset index was built with another pipeline version; ignoring it"
            );
            return Ok(Self::default());
        }
        // Seed the local metadata cache so indexed images are never decoded here.
        for image in &index.images {
            crate::cas::put_cached_metadata(&image.input_hash, &image.metadata);
        }
        Ok(index)
    }

    /// Record an input as fully uploaded (with its image metadata, if it is one).
    pub fn add_input(&mut self, input_hash: InputHash) {
        self.inputs.push(input_hash);
        if let Some(metadata) = crate::cas::get_cached_metadata(&input_hash) {
            self.images.push(IndexedImage {
                input_hash,
                metadata,
            });
        }
    }

    pub fn to_bytes(&self) -> eyre::Result<Vec<u8>> {
        let mut index = self.clone();
        index.pipeline_version = IMAGE_PIPELINE_VERSION;
        index.inputs.sort_by(|a, b| a.0.cmp(&b.0));
        index.inputs.dedup();
        index
            .images
            .sort_by(|a, b| a.input_hash.0.cmp(&b.input_hash.0));
        index.images.dedup_by(|a, b| a.input_hash == b.input_hash);
        facet_postcard::to_vec(&index).map_err(|e| eyre!("encoding asset index: {e}"))
    }

    pub fn set(&self) -> HashSet<InputHash> {
        self.inputs.iter().copied().collect()
    }
}

/// Short hex form of an input hash used in object keys: 128 bits is plenty to
/// keep keys unique and still readable.
pub fn short_hex(hash: &InputHash) -> String {
    let mut s = String::with_capacity(32);
    for b in &hash.0[..16] {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

/// Key of an image variant: `d/<hash>-<width>w.<ext>`.
pub fn variant_key(input_hash: &InputHash, format: OutputFormat, width: u32) -> String {
    format!(
        "d/{}-{}w.{}",
        short_hex(input_hash),
        width,
        format.extension()
    )
}

/// Key of a file stored as-is: `p/<hash>.<ext>`.
pub fn passthrough_key(input_hash: &InputHash, ext: &str) -> String {
    format!("p/{}.{}", short_hex(input_hash), ext)
}

/// Public URL of an object.
pub fn public_url(public_base_url: &str, key: &str) -> String {
    format!("{}/{}", public_base_url.trim_end_matches('/'), key)
}

/// Whether a static file's path has one of the passthrough extensions.
pub fn is_passthrough(path: &str, extensions: &[String]) -> bool {
    let lower = path.to_ascii_lowercase();
    extensions
        .iter()
        .any(|ext| lower.ends_with(&format!(".{}", ext.trim_start_matches('.'))))
}

pub fn content_type_for(key: &str) -> &'static str {
    let ext = key.rsplit('.').next().unwrap_or("");
    match ext {
        "jxl" => "image/jxl",
        "webp" => "image/webp",
        "avif" => "image/avif",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        "mov" => "video/quicktime",
        "m4a" => "audio/mp4",
        "mp3" => "audio/mpeg",
        "flac" => "audio/flac",
        "ogg" => "audio/ogg",
        "wav" => "audio/wav",
        "bin" => "application/octet-stream",
        _ => "application/octet-stream",
    }
}

/// Write-capable client for the store. Built from the site config plus
/// credentials in the environment.
pub struct AssetStoreClient {
    bucket: Bucket,
    credentials: Credentials,
    http: reqwest::Client,
}

impl AssetStoreClient {
    /// `Ok(None)` when no credentials are present in the environment.
    pub fn from_config(cfg: &AssetStoreConfig) -> eyre::Result<Option<Self>> {
        let (Ok(key), Ok(secret)) = (std::env::var(ACCESS_KEY_ENV), std::env::var(SECRET_KEY_ENV))
        else {
            return Ok(None);
        };
        let endpoint: url::Url = cfg
            .endpoint
            .parse()
            .wrap_err_with(|| format!("asset_store.endpoint `{}` is not a URL", cfg.endpoint))?;
        let region = cfg.region.clone().unwrap_or_else(|| "auto".to_string());
        let bucket = Bucket::new(endpoint, UrlStyle::Path, cfg.bucket.clone(), region)
            .map_err(|e| eyre!("asset_store bucket: {e}"))?;
        Ok(Some(Self {
            bucket,
            credentials: Credentials::new(key, secret),
            http: reqwest::Client::new(),
        }))
    }

    fn sign_ttl() -> Duration {
        Duration::from_secs(600)
    }

    pub async fn fetch_index(&self) -> eyre::Result<AssetIndex> {
        let url = self
            .bucket
            .get_object(Some(&self.credentials), INDEX_KEY)
            .sign(Self::sign_ttl());
        let res = self.http.get(url).send().await?;
        match res.status().as_u16() {
            200 => {
                let bytes = res.bytes().await?;
                // An unreadable index (older format, truncated upload) only means
                // we re-derive: the objects themselves are untouched and idempotent.
                let index = match AssetIndex::from_bytes(&bytes) {
                    Ok(index) => index,
                    Err(e) => {
                        tracing::warn!(error = %e, "asset index unreadable; starting empty");
                        AssetIndex::default()
                    }
                };
                tracing::info!(inputs = index.inputs.len(), "fetched asset index");
                Ok(index)
            }
            404 => {
                tracing::info!("no asset index in the bucket yet; starting empty");
                Ok(AssetIndex::default())
            }
            status => {
                let body = res.text().await.unwrap_or_default();
                Err(eyre!("fetching asset index: HTTP {status}: {body}"))
            }
        }
    }

    /// Upload an object. Existing objects are left untouched (content-addressed
    /// keys never change meaning), so a 412 from `If-None-Match: *` is success.
    /// Transient failures (connection resets, 5xx, 429) are retried a few times
    /// with backoff; a persistent failure leaves the input unindexed so the
    /// next build retries it.
    pub async fn put_object(
        &self,
        key: &str,
        content_type: &str,
        bytes: Vec<u8>,
    ) -> eyre::Result<()> {
        let mut last_err = None;
        for attempt in 0..4u32 {
            if attempt > 0 {
                tokio::time::sleep(Duration::from_millis(250 * 2u64.pow(attempt))).await;
            }
            let mut action = self.bucket.put_object(Some(&self.credentials), key);
            action
                .headers_mut()
                .insert("content-type", content_type.to_string());
            let url = action.sign(Self::sign_ttl());
            let res = self
                .http
                .put(url)
                .header("content-type", content_type)
                .header("if-none-match", "*")
                .body(bytes.clone())
                .send()
                .await;
            match res {
                Ok(res) => match res.status().as_u16() {
                    200 | 201 | 204 | 412 => return Ok(()),
                    status @ (429 | 500..=599) => {
                        let body = res.text().await.unwrap_or_default();
                        last_err = Some(eyre!("uploading {key}: HTTP {status}: {body}"));
                    }
                    status => {
                        let body = res.text().await.unwrap_or_default();
                        return Err(eyre!("uploading {key}: HTTP {status}: {body}"));
                    }
                },
                Err(e) => last_err = Some(eyre!("uploading {key}: {e}")),
            }
            tracing::warn!(key, attempt, error = %last_err.as_ref().unwrap(), "upload failed; retrying");
        }
        Err(last_err.unwrap())
    }

    /// Overwrite the index (always, unconditionally: it only ever grows).
    pub async fn put_index(&self, index: &AssetIndex) -> eyre::Result<()> {
        let bytes = index.to_bytes()?;
        let mut action = self.bucket.put_object(Some(&self.credentials), INDEX_KEY);
        action
            .headers_mut()
            .insert("content-type", "application/octet-stream".to_string());
        let url = action.sign(Self::sign_ttl());
        let res = self
            .http
            .put(url)
            .header("content-type", "application/octet-stream")
            .body(bytes)
            .send()
            .await?;
        if !res.status().is_success() {
            let status = res.status();
            let body = res.text().await.unwrap_or_default();
            return Err(eyre!("writing asset index: HTTP {status}: {body}"));
        }
        tracing::info!(inputs = index.inputs.len(), "wrote asset index");
        Ok(())
    }
}
