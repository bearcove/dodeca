/* tslint:disable */
/* eslint-disable */

/**
 * Log a message to the browser console (for debugging)
 */
export function log(msg: string): void;

/**
 * Mount the devtools overlay into the page
 */
export function mount_devtools(): void;

export type InitInput = RequestInfo | URL | Response | BufferSource | WebAssembly.Module;

export interface InitOutput {
    readonly memory: WebAssembly.Memory;
    readonly mount_devtools: () => void;
    readonly log: (a: number, b: number) => void;
    readonly wasm_bindgen__closure__destroy__h6dbba33256e47057: (a: number, b: number) => void;
    readonly wasm_bindgen__closure__destroy__h60ad5ebb8d5cc79e: (a: number, b: number) => void;
    readonly wasm_bindgen__closure__destroy__h464d001e56ef0524: (a: number, b: number) => void;
    readonly wasm_bindgen__convert__closures_____invoke__h68990f7aea37f8d9: (a: number, b: number, c: any) => void;
    readonly wasm_bindgen__convert__closures_____invoke__h6a5204b65e7613db: (a: number, b: number, c: any) => void;
    readonly wasm_bindgen__convert__closures_____invoke__h76510d7a7a68c6c6: (a: number, b: number) => void;
    readonly __wbindgen_malloc: (a: number, b: number) => number;
    readonly __wbindgen_realloc: (a: number, b: number, c: number, d: number) => number;
    readonly __wbindgen_externrefs: WebAssembly.Table;
    readonly __wbindgen_exn_store: (a: number) => void;
    readonly __externref_table_alloc: () => number;
    readonly __wbindgen_free: (a: number, b: number, c: number) => void;
    readonly __wbindgen_start: () => void;
}

export type SyncInitInput = BufferSource | WebAssembly.Module;

/**
 * Instantiates the given `module`, which can either be bytes or
 * a precompiled `WebAssembly.Module`.
 *
 * @param {{ module: SyncInitInput }} module - Passing `SyncInitInput` directly is deprecated.
 *
 * @returns {InitOutput}
 */
export function initSync(module: { module: SyncInitInput } | SyncInitInput): InitOutput;

/**
 * If `module_or_path` is {RequestInfo} or {URL}, makes a request and
 * for everything else, calls `WebAssembly.instantiate` directly.
 *
 * @param {{ module_or_path: InitInput | Promise<InitInput> }} module_or_path - Passing `InitInput` directly is deprecated.
 *
 * @returns {Promise<InitOutput>}
 */
export default function __wbg_init (module_or_path?: { module_or_path: InitInput | Promise<InitInput> } | InitInput | Promise<InitInput>): Promise<InitOutput>;
