/* tslint:disable */
/* eslint-disable */

/**
 * Load the search manifest. Must be awaited once before [`search`] is called;
 * `/search/search.js` does this on page load. Named `load_index` rather than
 * `init` so it never shadows the wasm-bindgen module loader (also `init`).
 *
 * `meta_url` is the site-absolute path of the manifest, normally
 * `/search/meta`.
 */
export function load_index(meta_url: string): Promise<void>;

/**
 * Run a query and return a JSON array of results (`{url, title, excerpt,
 * score}`), best first. `/search/search.js` `JSON.parse`s the return value.
 *
 * Rejects only on a genuine fault (network, malformed index); an empty query
 * or a query with no matches resolves to `"[]"`.
 */
export function search(query: string): Promise<any>;

export type InitInput = RequestInfo | URL | Response | BufferSource | WebAssembly.Module;

export interface InitOutput {
    readonly memory: WebAssembly.Memory;
    readonly load_index: (a: number, b: number) => any;
    readonly search: (a: number, b: number) => any;
    readonly wasm_bindgen__closure__destroy__hb94b6ce7014a2afa: (a: number, b: number) => void;
    readonly wasm_bindgen__convert__closures_____invoke__h4a5af0810d5d0b41: (a: number, b: number, c: any, d: any) => void;
    readonly wasm_bindgen__convert__closures_____invoke__h9edff6fafb66268e: (a: number, b: number, c: any) => void;
    readonly __wbindgen_malloc: (a: number, b: number) => number;
    readonly __wbindgen_realloc: (a: number, b: number, c: number, d: number) => number;
    readonly __wbindgen_exn_store: (a: number) => void;
    readonly __externref_table_alloc: () => number;
    readonly __wbindgen_externrefs: WebAssembly.Table;
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
