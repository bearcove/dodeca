#!/usr/bin/env -S node --experimental-strip-types --no-warnings=ExperimentalWarning

/**
 * Content-Addressed Storage batch upload for CI artifacts.
 *
 * Usage: cas-upload-batch.ts <manifest-key> <file1> [file2] ...
 *
 * This script:
 * 1. Computes SHA256 hashes in parallel
 * 2. Uploads to CAS in parallel (atomically via SSH)
 * 3. Writes a single manifest file with all files and their hashes
 *
 * Environment variables required:
 * - CAS_SSH_KEY: Private SSH key for cas@golem.bearcove.cloud
 */

import { basename } from "node:path";
import { stat } from "node:fs/promises";
import { createCasClient, type ManifestEntry, type PutResult } from "./cas-client.ts";

const startTime = performance.now();
let lastStep = startTime;

function logStep(name: string) {
  const now = performance.now();
  const elapsed = Math.round(now - lastStep);
  console.log(`  ⏱ ${name}: ${elapsed}ms`);
  lastStep = now;
}

function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes}B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)}KB`;
  return `${(bytes / 1024 / 1024).toFixed(1)}MB`;
}

async function main() {
  const [manifestKey, ...files] = process.argv.slice(2);

  if (!manifestKey || files.length === 0) {
    console.error("Usage: cas-upload-batch.ts <manifest-key> <file1> [file2] ...");
    console.error("Example: cas-upload-batch.ts ci/3410/cells-linux-x64 dist/ddc-cell-*");
    process.exit(1);
  }

  // Get file sizes
  console.log(`Files to upload (${files.length}):`);
  const fileSizes = await Promise.all(
    files.map(async (f) => {
      const s = await stat(f);
      return { file: f, size: s.size };
    })
  );
  let totalSize = 0;
  for (const { file, size } of fileSizes) {
    console.log(`  ${basename(file)}: ${formatSize(size)}`);
    totalSize += size;
  }
  console.log(`  Total: ${formatSize(totalSize)}`);

  // Initialize CAS client
  const knownHostsFile = new URL("./cas-known-hosts", import.meta.url).pathname;
  const client = await createCasClient(knownHostsFile);

  try {
    // Upload files in parallel
    console.log("Uploading to CAS...");
    const results = await Promise.all(
      files.map(async (file) => {
        const result = await client.put(file);
        return { file, ...result };
      })
    );
    logStep(`uploading ${formatSize(totalSize)}`);

    // Report results
    const uploaded = results.filter((r) => r.status === "uploaded");
    const existing = results.filter((r) => r.status === "exists");

    if (uploaded.length > 0) {
      console.log(`Uploaded ${uploaded.length} new files:`);
      for (const { file, hash, bytes, durationMs } of uploaded) {
        console.log(`  ${basename(file)}: ${formatSize(bytes)} in ${durationMs}ms`);
      }
    }

    if (existing.length > 0) {
      console.log(`Cache hits (${existing.length} files already in CAS):`);
      for (const { file, hash } of existing) {
        console.log(`  ${basename(file)}: ${hash.slice(0, 12)}...`);
      }
    }

    // Write manifest file
    console.log("Writing manifest...");
    const manifest: ManifestEntry[] = results.map(({ file, hash }) => ({
      name: basename(file),
      hash,
    }));

    await client.writeManifest(manifestKey, manifest);
    logStep("manifest");

    console.log(`Manifest written to ${manifestKey}`);

    // Summary
    const totalTime = Math.round(performance.now() - startTime);
    const cacheHitRate = ((existing.length / files.length) * 100).toFixed(1);
    console.log(`\nDone: ${files.length} files (${formatSize(totalSize)}) in ${totalTime}ms`);
    console.log(`Cache hit rate: ${cacheHitRate}% (${existing.length}/${files.length})`);
  } finally {
    await client.cleanup();
  }
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
