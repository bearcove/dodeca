#!/usr/bin/env -S node --experimental-strip-types --no-warnings=ExperimentalWarning

/**
 * CAS Client Library - Content-Addressed Storage over SSH
 *
 * Provides put/get/has operations for a content-addressed store
 * backed by rsync over SSH to cas@golem.bearcove.cloud.
 *
 * Server uses rrsync (restricted rsync) to limit access to /srv/cas/cas.
 *
 * Environment variables required:
 * - CAS_SSH_KEY: Private SSH key (ed25519 PEM/OpenSSH format)
 *
 * Server layout:
 * - sha256/<hash[0:2]>/<hash> - CAS objects (relative to /srv/cas/cas)
 * - pointers/<key> - Pointer files
 *
 * Race-safe: Multiple concurrent uploads of same hash write identical bytes.
 */

import { createHash, randomBytes } from "node:crypto";
import { readFile, writeFile, stat, unlink, chmod, mkdir } from "node:fs/promises";
import { spawn } from "node:child_process";
import { tmpdir } from "node:os";
import { join } from "node:path";

const CAS_HOST = "cas@golem.bearcove.cloud";

export interface PutResult {
  hash: string;
  status: "uploaded" | "exists";
  bytes: number;
  durationMs: number;
}

export interface GetResult {
  bytes: number;
  durationMs: number;
}

export interface ManifestEntry {
  name: string;
  hash: string;
}

export class CasClient {
  private keyFile: string | null = null;
  private controlPath: string | null = null;
  private knownHostsFile: string;

  constructor(knownHostsFile: string) {
    this.knownHostsFile = knownHostsFile;
  }

  /**
   * Initialize the client by setting up the ephemeral SSH key file and control socket
   */
  async init(): Promise<void> {
    const sshKey = process.env.CAS_SSH_KEY;
    if (!sshKey) {
      throw new Error("CAS_SSH_KEY environment variable not set");
    }

    // Create ephemeral key file
    this.keyFile = join(tmpdir(), `cas-key-${randomBytes(8).toString("hex")}`);
    await writeFile(this.keyFile, sshKey + "\n", { mode: 0o600 });

    // Create control socket path for connection reuse
    this.controlPath = join(tmpdir(), `cas-ctrl-${randomBytes(8).toString("hex")}`);
  }

  /**
   * Clean up ephemeral key file (control socket will auto-expire after ControlPersist)
   */
  async cleanup(): Promise<void> {
    if (this.keyFile) {
      try {
        await unlink(this.keyFile);
      } catch {
        // Ignore cleanup errors
      }
      this.keyFile = null;
    }
    // Control socket will auto-cleanup after ControlPersist timeout (60s)
    this.controlPath = null;
  }

  /**
   * Run rsync command
   */
  private async rsync(args: string[]): Promise<{ code: number; stdout: string; stderr: string }> {
    if (!this.keyFile || !this.controlPath) {
      throw new Error("CasClient not initialized - call init() first");
    }

    const sshArgs = [
      "-i", this.keyFile,
      "-o", "IdentitiesOnly=yes",
      "-o", "StrictHostKeyChecking=yes",
      "-o", `UserKnownHostsFile=${this.knownHostsFile}`,
      "-o", "ControlMaster=auto",
      "-o", `ControlPath=${this.controlPath}`,
      "-o", "ControlPersist=60",
    ].join(" ");

    const fullArgs = ["-e", `ssh ${sshArgs}`, ...args];

    return this.run("rsync", fullArgs);
  }

  /**
   * Run a command and capture output
   */
  private async run(cmd: string, args: string[]): Promise<{ code: number; stdout: string; stderr: string }> {
    return new Promise((resolve) => {
      const proc = spawn(cmd, args, { stdio: ["pipe", "pipe", "pipe"] });
      let stdout = "";
      let stderr = "";
      proc.stdout.on("data", (d) => (stdout += d));
      proc.stderr.on("data", (d) => (stderr += d));
      proc.on("close", (code) => resolve({ code: code ?? 1, stdout, stderr }));
    });
  }

  /**
   * Compute SHA256 hash of a file
   */
  private async computeHash(filePath: string): Promise<string> {
    const content = await readFile(filePath);
    return createHash("sha256").update(content).digest("hex");
  }

  /**
   * Upload a file to CAS
   * Returns the hash and whether it was uploaded or already existed
   */
  async put(filePath: string): Promise<PutResult> {
    const startTime = performance.now();

    // Compute hash
    const hash = await this.computeHash(filePath);
    const hashPrefix = hash.slice(0, 2);

    // Get file size
    const { size: bytes } = await stat(filePath);

    // Check if already exists (optional optimization)
    const finalPath = `sha256/${hashPrefix}/${hash}`;
    const exists = await this.has(hash);

    let status: "uploaded" | "exists" = "uploaded";

    if (!exists) {
      // Upload directly to final location
      // Race condition is fine - content-addressed means identical bytes
      // Use --mkpath to create parent directories (rsync 3.2.3+)
      const uploadResult = await this.rsync([
        "-a",
        "--mkpath",
        "--chmod=Fu=rw,Fgo=r",
        filePath,
        `${CAS_HOST}:${finalPath}`,
      ]);

      if (uploadResult.code !== 0) {
        throw new Error(`rsync upload failed: ${uploadResult.stderr}`);
      }
    } else {
      status = "exists";
    }

    const durationMs = Math.round(performance.now() - startTime);

    return { hash, status, bytes, durationMs };
  }

  /**
   * Check if a hash exists in CAS
   */
  async has(hash: string): Promise<boolean> {
    if (!/^[a-f0-9]{64}$/.test(hash)) {
      throw new Error(`Invalid hash: ${hash}`);
    }

    const hashPrefix = hash.slice(0, 2);
    const finalPath = `sha256/${hashPrefix}/${hash}`;

    // Use rsync --list-only to check if file exists (no gateway needed)
    const result = await this.rsync(["--list-only", `${CAS_HOST}:${finalPath}`]);
    return result.code === 0;
  }

  /**
   * Download a file from CAS
   */
  async get(hash: string, destPath: string): Promise<GetResult> {
    if (!/^[a-f0-9]{64}$/.test(hash)) {
      throw new Error(`Invalid hash: ${hash}`);
    }

    const startTime = performance.now();

    const hashPrefix = hash.slice(0, 2);
    const finalPath = `sha256/${hashPrefix}/${hash}`;
    const tmpDest = `${destPath}.tmp`;

    // Download to temp location
    const downloadResult = await this.rsync([
      "-a",
      `${CAS_HOST}:${finalPath}`,
      tmpDest,
    ]);

    if (downloadResult.code !== 0) {
      throw new Error(`rsync download failed: ${downloadResult.stderr}`);
    }

    // Verify hash
    const actualHash = await this.computeHash(tmpDest);
    if (actualHash !== hash) {
      await unlink(tmpDest);
      throw new Error(`Hash mismatch: expected ${hash}, got ${actualHash}`);
    }

    // Atomic rename to final destination
    const { size: bytes } = await stat(tmpDest);
    await this.run("mv", [tmpDest, destPath]);

    const durationMs = Math.round(performance.now() - startTime);

    return { bytes, durationMs };
  }

  /**
   * Write a single pointer file
   */
  async writePointer(pointerKey: string, hash: string): Promise<void> {
    if (!/^[a-f0-9]{64}$/.test(hash)) {
      throw new Error(`Invalid hash: ${hash}`);
    }

    // Create temp file with hash
    const tmpFile = join(tmpdir(), `pointer-${randomBytes(8).toString("hex")}`);
    await writeFile(tmpFile, hash + "\n");

    try {
      const remotePath = `pointers/${pointerKey}`;

      // Upload pointer file (rrsync will create parent dirs)
      const result = await this.rsync(["-a", tmpFile, `${CAS_HOST}:${remotePath}`]);

      if (result.code !== 0) {
        throw new Error(`Failed to write pointer: ${result.stderr}`);
      }
    } finally {
      await unlink(tmpFile);
    }
  }

  /**
   * Write a manifest file (grouped pointers)
   * Format: one line per file: "filename hash"
   */
  async writeManifest(manifestKey: string, entries: ManifestEntry[]): Promise<void> {
    // Validate all hashes
    for (const { hash } of entries) {
      if (!/^[a-f0-9]{64}$/.test(hash)) {
        throw new Error(`Invalid hash: ${hash}`);
      }
    }

    // Create temp file with manifest
    const manifestContent = entries.map(({ name, hash }) => `${name} ${hash}`).join("\n") + "\n";
    const tmpFile = join(tmpdir(), `manifest-${randomBytes(8).toString("hex")}`);
    await writeFile(tmpFile, manifestContent);

    try {
      const remotePath = `pointers/${manifestKey}`;

      // Upload manifest file (rrsync will create parent dirs)
      const result = await this.rsync(["-a", tmpFile, `${CAS_HOST}:${remotePath}`]);

      if (result.code !== 0) {
        throw new Error(`Failed to write manifest: ${result.stderr}`);
      }
    } finally {
      await unlink(tmpFile);
    }
  }

  /**
   * Read a single pointer file
   */
  async readPointer(pointerKey: string): Promise<string> {
    const remotePath = `pointers/${pointerKey}`;
    const tmpFile = join(tmpdir(), `pointer-${randomBytes(8).toString("hex")}`);

    try {
      const result = await this.rsync(["-a", `${CAS_HOST}:${remotePath}`, tmpFile]);

      if (result.code !== 0) {
        throw new Error(`Failed to read pointer: ${result.stderr}`);
      }

      const content = await readFile(tmpFile, "utf-8");
      const hash = content.trim();

      if (!/^[a-f0-9]{64}$/.test(hash)) {
        throw new Error(`Invalid hash in pointer: ${hash}`);
      }

      return hash;
    } finally {
      try {
        await unlink(tmpFile);
      } catch {
        // Ignore cleanup errors
      }
    }
  }

  /**
   * Read a manifest file (grouped pointers)
   * Returns array of { name, hash } entries
   */
  async readManifest(manifestKey: string): Promise<ManifestEntry[]> {
    const remotePath = `pointers/${manifestKey}`;
    const tmpFile = join(tmpdir(), `manifest-${randomBytes(8).toString("hex")}`);

    try {
      const result = await this.rsync(["-a", `${CAS_HOST}:${remotePath}`, tmpFile]);

      if (result.code !== 0) {
        throw new Error(`Failed to read manifest: ${result.stderr}`);
      }

      const content = await readFile(tmpFile, "utf-8");
      const entries: ManifestEntry[] = [];

      for (const line of content.split("\n")) {
        const trimmed = line.trim();
        if (!trimmed) continue;

        const [name, hash] = trimmed.split(/\s+/);
        if (!name || !hash || !/^[a-f0-9]{64}$/.test(hash)) {
          throw new Error(`Invalid manifest line: ${line}`);
        }

        entries.push({ name, hash });
      }

      return entries;
    } finally {
      try {
        await unlink(tmpFile);
      } catch {
        // Ignore cleanup errors
      }
    }
  }
}

/**
 * Helper: Create and initialize a CAS client
 */
export async function createCasClient(knownHostsFile: string): Promise<CasClient> {
  const client = new CasClient(knownHostsFile);
  await client.init();
  return client;
}
