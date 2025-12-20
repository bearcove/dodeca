#!/usr/bin/env -S node --experimental-strip-types --no-warnings=ExperimentalWarning

/**
 * CAS Client Library - Content-Addressed Storage over SSH
 *
 * Provides put/get/has operations for a content-addressed store
 * backed by SSH + rsync to cas@golem.bearcove.cloud.
 *
 * Environment variables required:
 * - CAS_SSH_KEY: Private SSH key (ed25519 PEM/OpenSSH format)
 *
 * Server layout:
 * - /srv/cas/cas/sha256/<hash[0:2]>/<hash> - CAS objects
 * - /srv/cas/cas/pointers/<key> - Pointer files
 * - /srv/cas/cas/tmp/ - Temporary upload staging
 */

import { createHash, randomBytes } from "node:crypto";
import { readFile, writeFile, stat, unlink, chmod, mkdir } from "node:fs/promises";
import { spawn } from "node:child_process";
import { tmpdir } from "node:os";
import { join } from "node:path";

const CAS_HOST = "cas@golem.bearcove.cloud";
const CAS_ROOT = "/srv/cas/cas";

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
   * Clean up ephemeral key file and control socket
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

    // Close control socket if it exists
    if (this.controlPath) {
      try {
        await this.run("ssh", [
          "-o", `ControlPath=${this.controlPath}`,
          "-O", "exit",
          CAS_HOST,
        ]);
      } catch {
        // Ignore cleanup errors
      }
      this.controlPath = null;
    }
  }

  /**
   * Run SSH command
   */
  private async ssh(args: string[]): Promise<{ code: number; stdout: string; stderr: string }> {
    if (!this.keyFile || !this.controlPath) {
      throw new Error("CasClient not initialized - call init() first");
    }

    const fullArgs = [
      "-i", this.keyFile,
      "-o", "IdentitiesOnly=yes",
      "-o", "StrictHostKeyChecking=yes",
      "-o", `UserKnownHostsFile=${this.knownHostsFile}`,
      "-o", "ControlMaster=auto",
      "-o", `ControlPath=${this.controlPath}`,
      "-o", "ControlPersist=60",
      CAS_HOST,
      ...args,
    ];

    return this.run("ssh", fullArgs);
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

    // Generate unique temp path
    const tmpPath = `tmp/${hash}.${randomBytes(4).toString("hex")}.${randomBytes(4).toString("hex")}.part`;
    const remoteTmpPath = `${CAS_ROOT}/${tmpPath}`;

    // Upload to temp location
    const uploadResult = await this.rsync([
      "-a",
      "--chmod=Fu=rw,Fgo=r",
      filePath,
      `${CAS_HOST}:${remoteTmpPath}`,
    ]);

    if (uploadResult.code !== 0) {
      throw new Error(`rsync upload failed: ${uploadResult.stderr}`);
    }

    // Atomic install
    const installResult = await this.ssh([`cas-install ${tmpPath} ${hash}`]);

    if (installResult.code !== 0) {
      throw new Error(`cas-install failed: ${installResult.stderr}`);
    }

    const status = installResult.stdout.trim() === "EXISTS" ? "exists" : "uploaded";

    // Integrity check: verify remote file size matches local
    if (status === "uploaded") {
      const finalPath = `${CAS_ROOT}/sha256/${hashPrefix}/${hash}`;
      const statResult = await this.ssh([`stat -c %s ${finalPath}`]);

      if (statResult.code !== 0) {
        throw new Error(`Failed to verify uploaded file: ${statResult.stderr}`);
      }

      const remoteSize = parseInt(statResult.stdout.trim(), 10);
      if (remoteSize !== bytes) {
        throw new Error(`Size mismatch: local=${bytes} remote=${remoteSize}`);
      }
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
    const finalPath = `${CAS_ROOT}/sha256/${hashPrefix}/${hash}`;

    const result = await this.ssh([`test -f ${finalPath}`]);
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
    const finalPath = `${CAS_ROOT}/sha256/${hashPrefix}/${hash}`;
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
      const remotePath = `${CAS_ROOT}/pointers/${pointerKey}`;
      const remoteDir = remotePath.substring(0, remotePath.lastIndexOf("/"));

      // Ensure remote directory exists
      await this.ssh([`mkdir -p ${remoteDir}`]);

      // Upload pointer file
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
      const remotePath = `${CAS_ROOT}/pointers/${manifestKey}`;
      const remoteDir = remotePath.substring(0, remotePath.lastIndexOf("/"));

      // Ensure remote directory exists
      await this.ssh([`mkdir -p ${remoteDir}`]);

      // Upload manifest file
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
    const remotePath = `${CAS_ROOT}/pointers/${pointerKey}`;
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
    const remotePath = `${CAS_ROOT}/pointers/${manifestKey}`;
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
