/**
 * The browser's half of /download's links (src/lib/release-assets.ts):
 * one call to GitHub's API for the page's own tag, kept in sessionStorage
 * so moving between pages, or reloading, asks once. Never the assets
 * themselves: their URLs redirect to a host that sends no CORS headers.
 */
import { useEffect, useState } from "react";

import {
	conventionAssets,
	type Lookup,
	lookupFrom,
	type ReleaseAssets,
	reconcile,
	releaseApiUrl,
} from "./release-assets";

/** How long a lookup is reused in one tab (a release's Mac job, or its sums, may land meanwhile). */
export const CACHE_MS = 10 * 60 * 1000;

const cacheKey = (tag: string) => `architect-release:${tag}`;

type Cached = { at: number; lookup: Lookup };

export type LookupDeps = {
	fetch: typeof fetch;
	storage: Pick<Storage, "getItem" | "setItem"> | null;
	now: () => number;
};

/**
 * The release `tag` per GitHub, from this tab's cache when it is fresh.
 * A release or a rate limit is kept (asking again within the hour would
 * only spend the limit); an error is not.
 */
export async function lookupRelease(
	tag: string,
	deps: LookupDeps,
): Promise<Lookup> {
	try {
		const raw = deps.storage?.getItem(cacheKey(tag));
		if (raw) {
			const cached = JSON.parse(raw) as Cached;
			if (deps.now() - cached.at < CACHE_MS) return cached.lookup;
		}
	} catch {
		// Blocked storage or a bad entry: ask.
	}
	let lookup: Lookup;
	try {
		const res = await deps.fetch(releaseApiUrl(tag), {
			headers: { accept: "application/vnd.github+json" },
		});
		const body = res.status === 200 ? await res.json() : null;
		lookup = lookupFrom(
			tag,
			res.status,
			res.headers.get("x-ratelimit-remaining"),
			body,
		);
	} catch (e) {
		lookup = { kind: "error", detail: String(e) };
	}
	if (lookup.kind !== "error") {
		try {
			deps.storage?.setItem(
				cacheKey(tag),
				JSON.stringify({ at: deps.now(), lookup } satisfies Cached),
			);
		} catch {
			// Full or blocked: the next view asks again.
		}
	}
	return lookup;
}

/** `pending` until the lookup is back (and with no script: the HTML's). */
export type CheckState = "pending" | Lookup["kind"];

/**
 * The release's assets for a page: the convention's while rendering (so
 * the prerendered HTML and the first client render agree), then
 * reconciled with the API once the page is up.
 */
export function useReleaseAssets(tag: string): {
	assets: ReleaseAssets;
	check: CheckState;
} {
	const [state, setState] = useState<{
		assets: ReleaseAssets;
		check: CheckState;
	}>(() => ({ assets: conventionAssets(tag), check: "pending" }));
	useEffect(() => {
		let live = true;
		let storage: Storage | null = null;
		try {
			storage = window.sessionStorage;
		} catch {
			// Blocked: no cache.
		}
		lookupRelease(tag, {
			fetch: window.fetch.bind(window),
			storage,
			now: Date.now,
		}).then((lookup) => {
			if (live)
				setState({ assets: reconcile(tag, lookup), check: lookup.kind });
		});
		return () => {
			live = false;
		};
	}, [tag]);
	return state;
}
