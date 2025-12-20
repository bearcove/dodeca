#!/usr/bin/env -S node --experimental-strip-types --no-warnings=ExperimentalWarning

/**
 * Content-Addressed Storage (CAS) download script for CI artifacts.
 *
 * Usage: cas-download.ts <pointer-key> <local-path>
 *
 * Downloads a file from CAS using content-addressed storage:
 * 1. Reads the pointer file to get the hash
 * 2. Downloads the actual content from CAS
 * 3. Verifies hash integrity
 *
 * Environment variables required:
 * - CAS_SSH_KEY: Private SSH key for cas@golem.bearcove.cloud
 */

import { mkdir, chmod } from "node:fs/promises";
import { dirname, basename } from "node:path";
import { createCasClient } from "./cas-client.ts";

function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes}B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)}KB`;
  return `${(bytes / 1024 / 1024).toFixed(1)}MB`;
}

async function main() {
  const [pointerKey, localPath] = process.argv.slice(2);

  if (!pointerKey || !localPath) {
    console.error("Usage: cas-download.ts <pointer-key> <local-path>");
    console.error("Example: cas-download.ts ci/12345/ddc-linux-x64 dist/ddc");
    process.exit(1);
  }

  // Ensure parent directory exists
  await mkdir(dirname(localPath), { recursive: true });

  const filename = basename(localPath);

  // Initialize CAS client
  const knownHostsFile = new URL("./cas-known-hosts", import.meta.url).pathname;
  const client = await createCasClient(knownHostsFile);

  try {
    // Read pointer to get hash
    const hash = await client.readPointer(pointerKey);

    // Download from CAS
    const { bytes, durationMs } = await client.get(hash, localPath);

    // Make executable if it looks like a binary
    if (filename === "ddc" || filename.startsWith("ddc-cell-")) {
      await chmod(localPath, 0o755);
    }

    console.log(`${filename}: downloaded from CAS (${formatSize(bytes)} in ${durationMs}ms)`);
  } finally {
    await client.cleanup();
  }
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
