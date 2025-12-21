#!/usr/bin/env -S node --experimental-strip-types --no-warnings=ExperimentalWarning

/**
 * Content-Addressed Storage (CAS) upload script for CI artifacts.
 *
 * Usage: cas-upload.ts <local-path> <pointer-key>
 *
 * Uploads a file to CAS using content-addressed storage:
 * 1. Computes SHA256 hash of the file
 * 2. Uploads to CAS via SSH if not already present
 * 3. Writes pointer file containing the hash
 *
 * Environment variables required:
 * - CAS_SSH_KEY: Private SSH key for cas@golem.bearcove.cloud
 */

import { stat } from "node:fs/promises";
import { basename } from "node:path";
import { createCasClient } from "./cas-client.ts";

function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes}B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)}KB`;
  return `${(bytes / 1024 / 1024).toFixed(1)}MB`;
}

async function main() {
  const [localPath, pointerKey] = process.argv.slice(2);

  if (!localPath || !pointerKey) {
    console.error("Usage: cas-upload.ts <local-path> <pointer-key>");
    console.error("Example: cas-upload.ts target/release/ddc ci/12345/ddc-linux-x64");
    process.exit(1);
  }

  // Verify file exists
  try {
    await stat(localPath);
  } catch {
    console.error(`Error: File not found: ${localPath}`);
    process.exit(1);
  }

  const filename = basename(localPath);

  // Initialize CAS client
  const knownHostsFile = new URL("./cas-known-hosts", import.meta.url).pathname;
  const client = await createCasClient(knownHostsFile);

  try {
    // Upload to CAS
    const { hash, status, bytes, durationMs } = await client.put(localPath);

    if (status === "uploaded") {
      console.log(`${filename}: uploaded to CAS (${formatSize(bytes)} in ${durationMs}ms)`);
    } else {
      console.log(`${filename}: already in CAS (${hash.slice(0, 12)}...)`);
    }

    // Write pointer file
    await client.writePointer(pointerKey, hash);
    console.log(`${filename}: pointer written to ${pointerKey}`);
  } finally {
    await client.cleanup();
  }
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
