import extensionsJson from "#/data/extensions.json";
import type { Store } from "./data";

/** The store's index (src/data/extensions.json). */
export const STORE_INDEX = extensionsJson as Store;

export function findExtension(id: string) {
	return STORE_INDEX.entries.find((e) => e.id === id);
}
