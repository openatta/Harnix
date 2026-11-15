// What `features.mjs` exports, for the TypeScript that imports it (`teardown.ts`). It stays
// JavaScript because `coverage.mjs` runs under plain `node`.

/** What one layer owes a feature that has no test there: why not (`na`), or what brings it (`pending`). */
export interface Owed {
  na?: string;
  pending?: string;
}

export interface Feature {
  id: string;
  name: string;
  api?: Owed;
  ui?: Owed;
  /** Its tests in `CARRIER_SUITES` also run on the process carrier, and are counted there. */
  carrier?: boolean;
}

export declare const FEATURES: Feature[];

/** The suites that run once per carrier, as globs under `tests/live`. */
export declare const CARRIER_SUITES: string[];
/** Of those, the ones that pin their carrier in their own profile. */
export declare const CARRIER_PINNED: string[];

/** Methods a client may call that no suite calls, each with the reason. */
export declare const UNCALLED: Record<string, string>;
