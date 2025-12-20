#!/usr/bin/env -S node --experimental-strip-types --no-warnings=ExperimentalWarning

/**
 * Content-Addressed Storage batch download for CI artifacts.
 *
 * Usage: cas-download-batch.ts <pointer-prefix> <local-dir>
 *
 * This script:
 * 1. Lists all pointer files under prefix
 * 2. Reads pointer hashes in parallel
 * 3. Downloads unique CAS objects in parallel
 */

import { spawn } from "node:child_process";
import { mkdir, chmod, stat, writeFile } from "node:fs/promises";
import { join, basename } from "node:path";

const S3_ENDPOINT = process.env.S3_ENDPOINT;
const S3_BUCKET = process.env.S3_BUCKET;

if (!S3_ENDPOINT || !S3_BUCKET) {
  console.error("Error: S3_ENDPOINT and S3_BUCKET must be set");
  process.exit(1);
}

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

async function run(cmd: string, args: string[]): Promise<{ code: number; stdout: string; stderr: string }> {
  return new Promise((resolve) => {
    const proc = spawn(cmd, args, { stdio: ["pipe", "pipe", "pipe"] });
    let stdout = "";
    let stderr = "";
    proc.stdout.on("data", (d) => (stdout += d));
    proc.stderr.on("data", (d) => (stderr += d));
    proc.on("close", (code) => resolve({ code: code ?? 1, stdout, stderr }));
  });
}

async function s3(...args: string[]): Promise<{ code: number; stdout: string; stderr: string }> {
  return run("aws", ["s3", "--endpoint-url", S3_ENDPOINT!, ...args]);
}

async function listPointers(prefix: string): Promise<string[]> {
  const result = await s3("ls", `s3://${S3_BUCKET}/${prefix}/`);
  if (result.code !== 0) return [];

  const files: string[] = [];
  for (const line of result.stdout.split("\n")) {
    const parts = line.trim().split(/\s+/);
    if (parts.length >= 4) {
      files.push(parts[3]);
    }
  }
  return files;
}

async function readPointer(pointerKey: string): Promise<string> {
  const result = await s3("cp", `s3://${S3_BUCKET}/${pointerKey}`, "-");
  if (result.code !== 0) {
    throw new Error(`Failed to read pointer ${pointerKey}: ${result.stderr}`);
  }
  return result.stdout.trim();
}

async function downloadFromCas(hash: string, localPath: string): Promise<void> {
  const result = await s3("cp", `s3://${S3_BUCKET}/cas/${hash}`, localPath);
  if (result.code !== 0) {
    throw new Error(`Failed to download from CAS: ${result.stderr}`);
  }
}

async function main() {
  const [pointerPrefix, localDir] = process.argv.slice(2);

  if (!pointerPrefix || !localDir) {
    console.error("Usage: cas-download-batch.ts <pointer-prefix> <local-dir>");
    process.exit(1);
  }

  await mkdir(localDir, { recursive: true });

  // List pointer files
  console.log(`Fetching pointer list from ${pointerPrefix}...`);
  const pointerFiles = await listPointers(pointerPrefix);
  logStep("list pointers");

  if (pointerFiles.length === 0) {
    console.error(`Warning: No pointer files found under ${pointerPrefix}`);
    return;
  }

  // Read all pointer hashes in parallel
  console.log(`Reading ${pointerFiles.length} pointer files...`);
  const pointerData = await Promise.all(
    pointerFiles.map(async (filename) => ({
      filename,
      hash: await readPointer(`${pointerPrefix}/${filename}`),
    }))
  );
  logStep("read pointers");

  // Group by unique hash
  const hashToFiles = new Map<string, string[]>();
  for (const { filename, hash } of pointerData) {
    const existing = hashToFiles.get(hash) || [];
    existing.push(filename);
    hashToFiles.set(hash, existing);
  }

  console.log(`Found ${pointerFiles.length} pointers → ${hashToFiles.size} unique CAS objects`);

  // Download unique CAS objects in parallel
  console.log("Downloading from CAS...");
  const downloads = await Promise.all(
    Array.from(hashToFiles.entries()).map(async ([hash, filenames]) => {
      const destPath = join(localDir, filenames[0]);
      await downloadFromCas(hash, destPath);
      return { hash, filenames, destPath };
    })
  );
  logStep("download CAS objects");

  // Copy for duplicates and set permissions
  let totalSize = 0;
  for (const { hash, filenames, destPath } of downloads) {
    const s = await stat(destPath);
    totalSize += s.size * filenames.length;

    // Make executable if it's a binary
    const first = filenames[0];
    if (first === "ddc" || first.startsWith("ddc-cell-")) {
      await chmod(destPath, 0o755);
    }

    // Copy to other filenames if duplicates exist
    for (let i = 1; i < filenames.length; i++) {
      const copyPath = join(localDir, filenames[i]);
      await run("cp", [destPath, copyPath]);
      if (filenames[i] === "ddc" || filenames[i].startsWith("ddc-cell-")) {
        await chmod(copyPath, 0o755);
      }
    }
  }
  logStep("finalize");

  const totalTime = Math.round(performance.now() - startTime);
  console.log(`Done: ${pointerFiles.length} files (${formatSize(totalSize)}) in ${totalTime}ms`);
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
