/**
 * The build-time data (src/data/*.json, written by scripts/data.ts and
 * committed as the last good copy), typed. Imported, never fetched: a page
 * reads what the build wrote. One module per file (this one, changelog.ts,
 * store.ts), so a page's script carries only the data that page shows.
 */
import releaseJson from "#/data/release.json";

/**
 * The release a build is for: its tag and page, nothing about its files.
 * Its downloads are named by convention from the tag and checked against
 * GitHub's API in the browser (src/lib/release-assets.ts), so the bundle
 * is built without waiting for the release's other jobs.
 */
export type Release = {
	tag: string;
	version: string;
	url: string;
	publishedAt: string | null;
};

export type ReleaseNotes = {
	tag: string;
	name: string;
	url: string;
	publishedAt: string;
	prerelease: boolean;
	/** The notes' markdown, rendered to HTML by scripts/data.ts. */
	html: string;
};

export type Extension = {
	id: string;
	name: string;
	version: string;
	kind: string;
	description: string;
	category: string;
	publisher: string;
	source:
		| { type: "archive"; archive: string; sha256?: string }
		| { type: "git"; git: string; rev?: string }
		| { type: "path"; path?: string };
	platforms: string[];
	changelog: string;
	versions: { version: string; changelog: string }[];
};

export type Store = {
	registry: string;
	source: "store" | "fixture";
	file: string;
	entries: Extension[];
};

export const RELEASE = releaseJson as Release;

/** "11 November 2026", fixed to UTC so the build and the browser agree. */
export function day(iso: string | null) {
	if (!iso) return "";
	return new Date(iso).toLocaleDateString("en-GB", {
		day: "numeric",
		month: "long",
		year: "numeric",
		timeZone: "UTC",
	});
}
