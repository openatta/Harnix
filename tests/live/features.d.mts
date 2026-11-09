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
}

export declare const FEATURES: Feature[];

/** Methods a client may call that no suite calls, each with the reason. */
export declare const UNCALLED: Record<string, string>;
