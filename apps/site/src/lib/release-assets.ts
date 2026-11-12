/**
 * A release's downloads, by name. Every asset's name follows from the tag
 * alone (.github/workflows/release.yml), so a page is prerendered with
 * its links and works with scripts off; on load, one call to GitHub's API
 * (`releaseApiUrl`) says which of them are really there, how big they are
 * and their sha256 (`reconcile`). Pure: no fetch, no storage, no DOM
 * (src/lib/release-check.ts does those), so it is unit-tested as is
 * (test/release-assets.test.ts).
 */

export const REPO = "FastTrackStudios/architect";

/** What the site offers from a release, each one asset. */
export type AssetKey = "linux" | "macos" | "node" | "web" | "sums" | "sig";

export const ASSET_KEYS: readonly AssetKey[] = [
	"linux",
	"macos",
	"node",
	"web",
	"sums",
	"sig",
];

/**
 * - `unchecked`: the link is the convention's; nothing has said yet
 *   whether the asset is there (no script, the call pending or failed).
 * - `listed`: the API listed it, uploaded: the link works.
 * - `missing`: the API answered and it is not there (not yet, or never:
 *   the Mac job is optional). The page shows no link.
 */
export type AssetState = "unchecked" | "listed" | "missing";

export type ReleaseAsset = {
	key: AssetKey;
	file: string;
	url: string;
	state: AssetState;
	/** Bytes, from the API. */
	size: number | null;
	/** Hex, from the API's `digest` ("sha256:…"). */
	sha256: string | null;
};

export type ReleaseAssets = Record<AssetKey, ReleaseAsset>;

/**
 * The file each asset has for `tag` (`vX.Y.Z`), as release.yml names it:
 * jobs `linux`, `macos`, `node` (the tag, with its `v`), `web` (the
 * version, without it) and `sums`.
 */
export function assetFiles(tag: string): Record<AssetKey, string> {
	const version = tag.replace(/^v/, "");
	return {
		linux: "Architect-linux-x86_64.AppImage",
		macos: "Architect-macos-arm64.dmg",
		node: `architect-node-${tag}-x86_64-linux.tar.gz`,
		web: `architect-web-${version}.tar.gz`,
		sums: "SHA256SUMS",
		sig: "SHA256SUMS.minisig",
	};
}

/** Where GitHub serves a release's asset (a redirect to its storage). */
export function downloadUrl(tag: string, file: string) {
	return `https://github.com/${REPO}/releases/download/${tag}/${file}`;
}

/** The one API call a page makes: CORS-enabled, unauthenticated. */
export function releaseApiUrl(tag: string) {
	return `https://api.github.com/repos/${REPO}/releases/tags/${tag}`;
}

/** Every asset by convention, none checked: what the HTML carries. */
export function conventionAssets(tag: string): ReleaseAssets {
	const files = assetFiles(tag);
	const out = {} as ReleaseAssets;
	for (const key of ASSET_KEYS) {
		out[key] = {
			key,
			file: files[key],
			url: downloadUrl(tag, files[key]),
			state: "unchecked",
			size: null,
			sha256: null,
		};
	}
	return out;
}

/** The part of the API's release the page reads. */
export type ApiAsset = {
	name: string;
	size?: number;
	/** "sha256:<hex>", on assets uploaded since mid-2025. */
	digest?: string | null;
	/** "uploaded" once complete; "open" while an upload runs. */
	state?: string;
};
export type ApiRelease = { tag_name: string; assets: ApiAsset[] };

/** How the call went. */
export type Lookup =
	| { kind: "release"; release: ApiRelease }
	| { kind: "rate-limited" }
	| { kind: "error"; detail: string };

/**
 * Classifies an API response: a 403 or 429 with no requests left is the
 * rate limit (60 an hour per address, unauthenticated); anything else that
 * is not a 200 with a release for this tag is an error.
 */
export function lookupFrom(
	tag: string,
	status: number,
	remaining: string | null,
	body: unknown,
): Lookup {
	if (status === 429 || (status === 403 && remaining === "0"))
		return { kind: "rate-limited" };
	if (status !== 200) return { kind: "error", detail: `HTTP ${status}` };
	const release = body as Partial<ApiRelease> | null;
	if (!release || release.tag_name !== tag || !Array.isArray(release.assets))
		return { kind: "error", detail: "not this tag's release" };
	return {
		kind: "release",
		release: {
			tag_name: release.tag_name,
			// Kept small: the result is cached in sessionStorage.
			assets: release.assets.map((a) => ({
				name: a.name,
				size: a.size,
				digest: a.digest ?? null,
				state: a.state,
			})),
		},
	};
}

/**
 * The convention's links, reconciled with the lookup. On a release, each
 * asset the API lists (uploaded) is `listed` with its size and sha256, and
 * every other one is `missing`; assets the page does not offer are
 * ignored. On a rate limit or an error, the links stay `unchecked`, as
 * the HTML has them.
 */
export function reconcile(tag: string, lookup: Lookup): ReleaseAssets {
	const assets = conventionAssets(tag);
	if (lookup.kind !== "release") return assets;
	const byName = new Map(lookup.release.assets.map((a) => [a.name, a]));
	for (const key of ASSET_KEYS) {
		const a = byName.get(assets[key].file);
		if (!a || (a.state !== undefined && a.state !== "uploaded")) {
			assets[key] = { ...assets[key], state: "missing" };
			continue;
		}
		const sha = /^sha256:([0-9a-f]{64})$/.exec(a.digest ?? "");
		assets[key] = {
			...assets[key],
			state: "listed",
			size: typeof a.size === "number" ? a.size : null,
			sha256: sha ? sha[1] : null,
		};
	}
	return assets;
}

/** "73.9 MB": decimal units. */
export function formatSize(bytes: number) {
	if (bytes < 1000) return `${bytes} B`;
	const units = ["kB", "MB", "GB"];
	let n = bytes / 1000;
	let i = 0;
	while (n >= 1000 && i < units.length - 1) {
		n /= 1000;
		i++;
	}
	return `${n.toFixed(n < 10 ? 2 : 1)} ${units[i]}`;
}
