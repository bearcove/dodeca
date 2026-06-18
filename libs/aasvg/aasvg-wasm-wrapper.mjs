// aasvg-wasm-wrapper.mjs - Runtime wrapper for aasvg.wasm
// Usage: import { render } from './aasvg-wasm-wrapper.mjs'

import { readFileSync } from 'fs';
import { fileURLToPath } from 'url';
import { dirname, join } from 'path';

const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);

const TYPES = {
    undefined: 0x00,
    number: 0x01,
    bytestring: 0xC3,
};

let instance = null;
let memory = null;

async function init() {
    if (instance) return;

    const wasmPath = join(__dirname, 'aasvg.wasm');
    const wasmBuffer = readFileSync(wasmPath);
    const module = await WebAssembly.compile(wasmBuffer);
    instance = await WebAssembly.instantiate(module);
    memory = instance.exports.$;

    // Ensure we have enough memory (at least 1MB)
    const currentPages = memory.buffer.byteLength / 65536;
    if (currentPages < 16) {
        memory.grow(16 - currentPages);
    }
}

function writeString(str, ptr) {
    const dv = new DataView(memory.buffer);
    dv.setUint32(ptr, str.length, true);
    const arr = new Uint8Array(memory.buffer, ptr + 4, str.length);
    for (let i = 0; i < str.length; i++) {
        arr[i] = str.charCodeAt(i);
    }
    return ptr;
}

function readString(ptr) {
    const dv = new DataView(memory.buffer);
    const length = dv.getUint32(ptr, true);
    const arr = new Uint8Array(memory.buffer, ptr + 4, length);
    let result = '';
    for (let i = 0; i < length; i++) {
        result += String.fromCharCode(arr[i]);
    }
    return result;
}

/**
 * Render an ASCII art diagram to SVG
 * @param {string} diagram - The ASCII art diagram
 * @returns {Promise<string>} - The SVG output
 */
export async function render(diagram) {
    await init();

    // Write input string to memory
    // Use a high address to avoid conflicts with wasm's internal allocations
    const inputPtr = 65536;  // Start of second page
    writeString(diagram, inputPtr);

    // Call wasm: render(internal1, internal2, internal3, internal4, strPtr, strType)
    // The first 4 params are internal Porffor context (we pass 0s)
    const [resultPtr, resultType] = instance.exports.render(
        0, 0,  // First internal pair
        0, 0,  // Second internal pair
        inputPtr, TYPES.bytestring  // The actual string parameter
    );

    // Read result string
    return readString(resultPtr);
}

// Synchronous version (call init() first)
export function renderSync(diagram) {
    if (!instance) {
        throw new Error('Call init() first or use render() async');
    }

    const inputPtr = 65536;
    writeString(diagram, inputPtr);

    const [resultPtr, resultType] = instance.exports.render(
        0, 0, 0, 0,
        inputPtr, TYPES.bytestring
    );

    return readString(resultPtr);
}

export { init };
