+++
title = "Asset store"
weight = 20
+++

Image variants and large media can live in a **content-addressed bucket**
instead of the output directory. Pages reference them from the bucket's public
URL, `public/` shrinks to HTML/CSS/JS, and — the important part — **no image is
ever derived twice**, on any machine.

## How it works

A derived asset is a pure function of its input's content hash, the image
pipeline version and the derivation parameters. Its key in the bucket is made of
exactly those:

```text
d/<input-hash>-<width>w.webp    image variant
d/<input-hash>-<width>w.jxl
p/<input-hash>.mp4              file stored as-is ("passthrough")
```

The bucket also holds a small **index** (`index/inputs.v1.bin`): the set of input
hashes whose derivations have all been uploaded, plus each image's dimensions
and thumbhash. A build:

1. fetches the index (one request),
2. references indexed inputs' variants without decoding or encoding anything,
3. derives the inputs that aren't indexed, uploads their objects, and only then
   adds them to the index (uploads first, index last — the index never claims
   an object that isn't there).

Two builds racing on the index can only make it *under*-report, which costs a
redundant re-derive and nothing else. Existing objects are never overwritten
(`If-None-Match: *`), so the store is append-only in practice.

Only `ddc build` talks to the bucket. `ddc serve` ignores the store and derives
locally, so laptops need no credentials and can't write anything.

## Configuration

```styx
site {
    asset_store {
        // pages reference derived assets under this URL
        public_base_url "https://assets.example.com"
        // S3 API endpoint (Cloudflare R2 shown)
        endpoint "https://<account-id>.r2.cloudflarestorage.com"
        bucket my-site-assets
        // request-signing region; R2 uses `auto` (the default)
        region auto
        // static files stored as-is; defaults to common video/audio containers
        passthrough_extensions (mp4 webm mov m4a mp3 flac ogg wav)
    }
}
```

Credentials come from the environment, and `ddc build` refuses to run without
them when `asset_store` is configured:

```bash
export DODECA_ASSET_STORE_ACCESS_KEY_ID=...
export DODECA_ASSET_STORE_SECRET_ACCESS_KEY=...
ddc build
```

Any S3-compatible store works: R2, S3, MinIO, or `rclone serve s3` for local
experiments:

```bash
rclone serve s3 --addr 127.0.0.1:9000 --auth-key key,secret /tmp/store
```

## Invalidation

Bump `IMAGE_PIPELINE_VERSION` (in `dodeca::cas`) when encoder settings change:
input hashes mix it in, so every key changes and the old objects simply go
cold. An index written with another pipeline version is ignored.

## Serving

Put the bucket behind a CDN with a custom domain and long cache lifetimes —
keys are content-addressed, so `Cache-Control: public, max-age=31536000,
immutable` is safe.
