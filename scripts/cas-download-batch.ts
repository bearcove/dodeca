#!/usr/bin/env -S node --experimental-strip-types --no-warnings=ExperimentalWarning

/**
 * Content-Addressed Storage batch download for CI artifacts.
 *
 * Usage: cas-download-batch.ts <manifest-key> <output-dir>
 *
 * This script:
 * 1. Reads the manifest file to get the list of files and their hashes
 * 2. Downloads all files from CAS in parallel
 * 3. Writes files to the output directory with proper permissions
 *
 * Environment variables required:
 * - CAS_SSH_KEY: Private SSH key for cas@golem.bearcove.cloud
 */

import { mkdir, chmod } from "node:fs/promises";
import { join, basename } from "node:path";
import { createCasClient } from "./cas-client.ts";

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
  const [manifestKey, outputDir] = process.argv.slice(2);

  if (!manifestKey || !outputDir) {
    console.error("Usage: cas-download-batch.ts <manifest-key> <output-dir>");
    console.error("Example: cas-download-batch.ts ci/3410/cells-linux-x64 dist/");
    process.exit(1);
  }

  // Ensure output directory exists
  await mkdir(outputDir, { recursive: true });

  // Initialize CAS client
  const knownHostsFile = new URL("./cas-known-hosts", import.meta.url).pathname;
  const client = await createCasClient(knownHostsFile);

  try {
    // Read manifest
    console.log(`Reading manifest from ${manifestKey}...`);
    const manifest = await client.readManifest(manifestKey);
    logStep("read manifest");

    if (manifest.length === 0) {
      console.error("Error: Manifest is empty");
      process.exit(1);
    }

    console.log(`Found ${manifest.length} files in manifest`);

    // Download files in parallel
    console.log("Downloading from CAS...");
    const results = await Promise.all(
      manifest.map(async ({ name, hash }) => {
        const destPath = join(outputDir, name);
        const { bytes, durationMs } = await client.get(hash, destPath);

        // Make executable if it looks like a binary
        if (name === "ddc" || name.startsWith("ddc-cell-")) {
          await chmod(destPath, 0o755);
        }

        return { name, hash, bytes, durationMs };
      })
    );
    logStep("downloading files");

    // Report results
    let totalBytes = 0;
    console.log("Downloaded files:");
    for (const { name, hash, bytes, durationMs } of results) {
      console.log(`  ${name}: ${formatSize(bytes)} in ${durationMs}ms`);
      totalBytes += bytes;
    }

    // Summary
    const totalTime = Math.round(performance.now() - startTime);
    console.log(`\nDone: ${manifest.length} files (${formatSize(totalBytes)}) in ${totalTime}ms`);
  } catch (err) {
    if (err instanceof Error) {
      console.error(`Error: ${err.message}`);
    } else {
      console.error(err);
    }
    process.exit(1);
  } finally {
    await client.cleanup();
  }
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
