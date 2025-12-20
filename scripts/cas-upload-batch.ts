#!/usr/bin/env -S node --experimental-strip-types --no-warnings=ExperimentalWarning

/**
 * Content-Addressed Storage batch upload for CI artifacts.
 *
 * Usage: cas-upload-batch.ts <pointer-prefix> <file1> [file2] ...
 *
 * This script:
 * 1. Computes SHA256 hashes in parallel
 * 2. Uploads to CAS in parallel (skipping existing)
 * 3. Writes pointer files in parallel
 */

import { createHash } from "node:crypto";
import { readFile, stat, mkdir, rm, link, copyFile } from "node:fs/promises";
import { spawn } from "node:child_process";
import { basename, join } from "node:path";
import { tmpdir } from "node:os";

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

async function run(cmd: string, args: string[], stdin?: string): Promise<{ code: number; stdout: string; stderr: string }> {
  return new Promise((resolve) => {
    const proc = spawn(cmd, args, { stdio: ["pipe", "pipe", "pipe"] });
    let stdout = "";
    let stderr = "";
    proc.stdout.on("data", (d) => (stdout += d));
    proc.stderr.on("data", (d) => (stderr += d));
    if (stdin) {
      proc.stdin.write(stdin);
      proc.stdin.end();
    }
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

async function main() {
  const [pointerPrefix, ...files] = process.argv.slice(2);

  if (!pointerPrefix || files.length === 0) {
    console.error("Usage: cas-upload-batch.ts <pointer-prefix> <file1> [file2] ...");
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

  // Compute hashes in parallel
  console.log("Computing SHA256 hashes...");
  const hashResults = await Promise.all(
    files.map(async (file) => ({
      file,
      hash: await computeHash(file),
    }))
  );
  logStep(`hashing ${formatSize(totalSize)}`);

  // Check which hashes exist in CAS (in parallel)
  console.log("Checking CAS...");
  const existsResults = await Promise.all(
    hashResults.map(async ({ file, hash }) => ({
      file,
      hash,
      exists: await existsInCas(hash),
    }))
  );
  logStep("checking CAS");

  // Upload missing files to CAS (in parallel)
  const toUpload = existsResults.filter((r) => !r.exists);
  if (toUpload.length > 0) {
    console.log(`Uploading ${toUpload.length} new files to CAS...`);
    await Promise.all(
      toUpload.map(async ({ file, hash }) => {
        const result = await s3("cp", file, `s3://${S3_BUCKET}/cas/${hash}`);
        if (result.code !== 0) {
          console.error(`Failed to upload ${file}: ${result.stderr}`);
          process.exit(1);
        }
      })
    );
    logStep(`uploading ${toUpload.length} files`);
  } else {
    console.log("All files already in CAS");
    logStep("(no uploads needed)");
  }

  // Write pointer files in parallel
  console.log("Writing pointer files...");
  await Promise.all(
    hashResults.map(async ({ file, hash }) => {
      const filename = basename(file);
      const pointerKey = `${pointerPrefix}/${filename}`;
      const result = await run(
        "aws",
        ["s3", "--endpoint-url", S3_ENDPOINT!, "cp", "-", `s3://${S3_BUCKET}/${pointerKey}`],
        hash
      );
      if (result.code !== 0) {
        console.error(`Failed to write pointer for ${filename}: ${result.stderr}`);
        process.exit(1);
      }
    })
  );
  logStep("pointer files");

  const totalTime = Math.round(performance.now() - startTime);
  console.log(`Done: ${files.length} files (${formatSize(totalSize)}) in ${totalTime}ms`);
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
