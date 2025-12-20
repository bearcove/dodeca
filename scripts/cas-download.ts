#!/usr/bin/env -S node --experimental-strip-types --no-warnings=ExperimentalWarning

/**
 * Content-Addressed Storage (CAS) download script for CI artifacts.
 *
 * Usage: cas-download.ts <pointer-key> <local-path>
 *    or: cas-download.ts --prefix <pointer-prefix> <local-dir>
 *
 * Downloads a file from S3 using content-addressed storage:
 * 1. Reads the pointer file to get the hash
 * 2. Downloads the actual content from cas/<hash>
 *
 * With --prefix, downloads all files under a pointer prefix.
 *
 * Environment variables required:
 * - S3_ENDPOINT: S3-compatible endpoint URL
 * - S3_BUCKET: Bucket name
 * - AWS_ACCESS_KEY_ID, AWS_SECRET_ACCESS_KEY: Credentials
 */

import { spawn } from "node:child_process";
import { mkdir, chmod } from "node:fs/promises";
import { dirname, basename, join } from "node:path";

const S3_ENDPOINT = process.env.S3_ENDPOINT;
const S3_BUCKET = process.env.S3_BUCKET;

if (!S3_ENDPOINT || !S3_BUCKET) {
  console.error("Error: S3_ENDPOINT and S3_BUCKET must be set");
  process.exit(1);
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

async function readPointer(pointerKey: string): Promise<string> {
  const result = await s3("cp", `s3://${S3_BUCKET}/${pointerKey}`, "-");
  if (result.code !== 0) {
    throw new Error(`Failed to read pointer ${pointerKey}: ${result.stderr}`);
  }
  return result.stdout.trim();
}

async function downloadFromCas(hash: string, localPath: string): Promise<void> {
  await mkdir(dirname(localPath), { recursive: true });
  const result = await s3("cp", `s3://${S3_BUCKET}/cas/${hash}`, localPath);
  if (result.code !== 0) {
    throw new Error(`Failed to download from CAS: ${result.stderr}`);
  }
}

async function listPointers(prefix: string): Promise<string[]> {
  const result = await s3("ls", `s3://${S3_BUCKET}/${prefix}/`);
  if (result.code !== 0) {
    // Empty prefix is okay
    return [];
  }

  // Parse aws s3 ls output: "2024-01-01 12:00:00         64 filename"
  const files: string[] = [];
  for (const line of result.stdout.split("\n")) {
    const parts = line.trim().split(/\s+/);
    if (parts.length >= 4) {
      files.push(parts[3]);
    }
  }
  return files;
}

async function downloadSingle(pointerKey: string, localPath: string): Promise<void> {
  const hash = await readPointer(pointerKey);
  await downloadFromCas(hash, localPath);

  // Make executable if it looks like a binary
  const filename = basename(localPath);
  if (filename === "ddc" || filename.startsWith("ddc-cell-")) {
    await chmod(localPath, 0o755);
  }

  console.log(`${basename(localPath)}: downloaded from CAS (${hash.slice(0, 12)}...)`);
}

async function downloadPrefix(pointerPrefix: string, localDir: string): Promise<void> {
  await mkdir(localDir, { recursive: true });

  const files = await listPointers(pointerPrefix);
  if (files.length === 0) {
    console.error(`Warning: No files found under ${pointerPrefix}`);
    return;
  }

  for (const filename of files) {
    const pointerKey = `${pointerPrefix}/${filename}`;
    const localPath = join(localDir, filename);
    await downloadSingle(pointerKey, localPath);
  }

  console.log(`Downloaded ${files.length} files from ${pointerPrefix}`);
}

async function main() {
  const args = process.argv.slice(2);

  if (args[0] === "--prefix") {
    const [, pointerPrefix, localDir] = args;
    if (!pointerPrefix || !localDir) {
      console.error("Usage: cas-download.ts --prefix <pointer-prefix> <local-dir>");
      console.error("Example: cas-download.ts --prefix ci/12345/cells-linux-x64 dist/");
      process.exit(1);
    }
    await downloadPrefix(pointerPrefix, localDir);
  } else {
    const [pointerKey, localPath] = args;
    if (!pointerKey || !localPath) {
      console.error("Usage: cas-download.ts <pointer-key> <local-path>");
      console.error("   or: cas-download.ts --prefix <pointer-prefix> <local-dir>");
      console.error("Example: cas-download.ts ci/12345/ddc-linux-x64 dist/ddc");
      process.exit(1);
    }
    await downloadSingle(pointerKey, localPath);
  }
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
