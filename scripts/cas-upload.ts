#!/usr/bin/env -S node --experimental-strip-types --no-warnings=ExperimentalWarning

/**
 * Content-Addressed Storage (CAS) upload script for CI artifacts.
 *
 * Usage: cas-upload.ts <local-path> <pointer-key>
 *
 * Uploads a file to S3 using content-addressed storage:
 * 1. Computes SHA256 hash of the file
 * 2. Uploads to cas/<hash> if not already present
 * 3. Writes pointer file at ci/<run_id>/<pointer-key> containing the hash
 *
 * Environment variables required:
 * - S3_ENDPOINT: S3-compatible endpoint URL
 * - S3_BUCKET: Bucket name
 * - AWS_ACCESS_KEY_ID, AWS_SECRET_ACCESS_KEY: Credentials
 */

import { createHash } from "node:crypto";
import { readFile, stat } from "node:fs/promises";
import { spawn } from "node:child_process";
import { basename } from "node:path";

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

async function computeHash(filePath: string): Promise<string> {
  const content = await readFile(filePath);
  return createHash("sha256").update(content).digest("hex");
}

async function existsInCas(hash: string): Promise<boolean> {
  const result = await s3("ls", `s3://${S3_BUCKET}/cas/${hash}`);
  return result.code === 0;
}

async function uploadToCas(filePath: string, hash: string): Promise<void> {
  const result = await s3("cp", filePath, `s3://${S3_BUCKET}/cas/${hash}`);
  if (result.code !== 0) {
    console.error(`Failed to upload to CAS: ${result.stderr}`);
    process.exit(1);
  }
}

async function writePointer(pointerKey: string, hash: string): Promise<void> {
  // Use stdin to write the hash
  const proc = spawn("aws", [
    "s3", "--endpoint-url", S3_ENDPOINT!,
    "cp", "-", `s3://${S3_BUCKET}/${pointerKey}`
  ], { stdio: ["pipe", "pipe", "pipe"] });

  proc.stdin.write(hash);
  proc.stdin.end();

  return new Promise((resolve, reject) => {
    proc.on("close", (code) => {
      if (code === 0) resolve();
      else reject(new Error(`Failed to write pointer (exit ${code})`));
    });
  });
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

  const hash = await computeHash(localPath);
  const filename = basename(localPath);

  if (await existsInCas(hash)) {
    console.log(`${filename}: already in CAS (${hash.slice(0, 12)}...)`);
  } else {
    await uploadToCas(localPath, hash);
    console.log(`${filename}: uploaded to CAS (${hash.slice(0, 12)}...)`);
  }

  await writePointer(pointerKey, hash);
  console.log(`${filename}: pointer written to ${pointerKey}`);
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
