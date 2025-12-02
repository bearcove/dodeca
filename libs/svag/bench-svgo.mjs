#!/usr/bin/env node
// Parallel batch SVG optimization with svgo using worker threads
// Outputs JSON with timing and size info for fair comparison

import { readdirSync, readFileSync, statSync } from 'fs';
import { join, relative } from 'path';
import { cpus } from 'os';
import { Worker, isMainThread, parentPort, workerData } from 'worker_threads';
import { fileURLToPath } from 'url';

const __filename = fileURLToPath(import.meta.url);

// Recursively collect all .svg files
function walkDir(dir) {
  const files = [];
  try {
    for (const entry of readdirSync(dir, { withFileTypes: true })) {
      const path = join(dir, entry.name);
      if (entry.isDirectory()) {
        files.push(...walkDir(path));
      } else if (entry.isFile() && entry.name.endsWith('.svg')) {
        files.push(path);
      }
    }
  } catch (e) {
    // Skip unreadable directories
  }
  return files;
}

if (!isMainThread) {
  // Worker thread: process a batch of files
  const { optimize } = await import('svgo');
  const { files, corpusDir } = workerData;

  const results = [];
  let totalOriginal = 0;
  let totalMinified = 0;

  for (const file of files) {
    const name = relative(corpusDir, file);

    let svg;
    try {
      svg = readFileSync(file, 'utf8');
    } catch (e) {
      continue;
    }

    const originalSize = Buffer.byteLength(svg, 'utf8');

    try {
      const result = optimize(svg, { path: file });
      const minifiedSize = Buffer.byteLength(result.data, 'utf8');

      totalOriginal += originalSize;
      totalMinified += minifiedSize;

      results.push({ name, original: originalSize, minified: minifiedSize });
    } catch (e) {
      // svgo failed on this file - skip it
      continue;
    }
  }

  parentPort.postMessage({ results, totalOriginal, totalMinified });
} else {
  // Main thread
  const corpusDir = process.argv[2] || 'tests/corpus';
  const numWorkers = cpus().length;

  const allFiles = walkDir(corpusDir).sort();
  const filesPerWorker = Math.ceil(allFiles.length / numWorkers);

  // Split files into batches for workers
  const batches = [];
  for (let i = 0; i < allFiles.length; i += filesPerWorker) {
    batches.push(allFiles.slice(i, i + filesPerWorker));
  }

  const startAll = performance.now();

  const workerPromises = batches.map(files => {
    return new Promise((resolve, reject) => {
      const worker = new Worker(__filename, {
        workerData: { files, corpusDir }
      });
      worker.on('message', resolve);
      worker.on('error', reject);
      worker.on('exit', code => {
        if (code !== 0) reject(new Error(`Worker exited with code ${code}`));
      });
    });
  });

  try {
    const results = await Promise.all(workerPromises);
    const totalTime = performance.now() - startAll;

    // Aggregate results
    let allResults = [];
    let totalOriginal = 0;
    let totalMinified = 0;

    for (const r of results) {
      allResults.push(...r.results);
      totalOriginal += r.totalOriginal;
      totalMinified += r.totalMinified;
    }

    console.log(JSON.stringify({
      files: allResults,
      total: {
        original: totalOriginal,
        minified: totalMinified,
        saved: totalOriginal - totalMinified,
        time_ms: totalTime,
      }
    }));
  } catch (e) {
    console.error('Worker error:', e);
    process.exit(1);
  }
}
