#!/usr/bin/env node
// Batch SVG optimization with svgo - loads once, processes all files
// Outputs JSON with timing and size info for fair comparison

import { readFileSync, readdirSync, statSync } from 'fs';
import { join, basename } from 'path';
import { optimize } from 'svgo';

const corpusDir = process.argv[2] || 'tests/corpus';

// Collect all .svg files (non-recursive, top-level only for README benchmarks)
const files = readdirSync(corpusDir)
  .filter(f => f.endsWith('.svg'))
  .map(f => join(corpusDir, f))
  .filter(f => statSync(f).isFile())
  .sort();

const results = [];
let totalOriginal = 0;
let totalMinified = 0;

const startAll = performance.now();

for (const file of files) {
  const name = basename(file, '.svg');
  const svg = readFileSync(file, 'utf8');
  const originalSize = Buffer.byteLength(svg, 'utf8');

  const start = performance.now();
  const result = optimize(svg, { path: file });
  const elapsed = performance.now() - start;

  const minifiedSize = Buffer.byteLength(result.data, 'utf8');

  totalOriginal += originalSize;
  totalMinified += minifiedSize;

  results.push({
    name,
    original: originalSize,
    minified: minifiedSize,
    time_ms: elapsed,
  });
}

const totalTime = performance.now() - startAll;

console.log(JSON.stringify({
  files: results,
  total: {
    original: totalOriginal,
    minified: totalMinified,
    saved: totalOriginal - totalMinified,
    time_ms: totalTime,
  }
}, null, 2));
