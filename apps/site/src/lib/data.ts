/**
 * The build-time data (src/data/*.json, written by scripts/data.ts and
 * committed as the last good copy), typed. Imported, never fetched: a page
 * reads what the build wrote.
 */
import changelogJson from "#/data/changelog.json";
import extensionsJson from "#/data/extensions.json";
import releaseJson from "#/data/release.json";

export type Asset = {
	version: string;
	url: string;
	sha256: string;
	file: string;
};
export type Release = {
	tag: string;
	version: string;
	url: string;
	publishedAt: string | null;
	linux: Asset | null;
	macos: Asset | null;
	node: { url: string; file: string } | null;
	sums: string | null;
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
export const CHANGELOG_RELEASES = changelogJson.releases as ReleaseNotes[];
export const STORE_INDEX = extensionsJson as Store;

export function findExtension(id: string) {
	return STORE_INDEX.entries.find((e) => e.id === id);
}

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
