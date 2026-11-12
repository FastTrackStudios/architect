/**
 * The site's build-time data, written to src/data/*.json (committed):
 *
 *   release.json     the release the site is built for: its tag, page and
 *                    date, for /download and the landing page. Nothing
 *                    about its files: their names follow from the tag and
 *                    the browser checks them against GitHub's API
 *                    (src/lib/release-assets.ts), so no other release job
 *                    has to run first
 *   changelog.json   the last releases' notes (the GitHub release bodies,
 *                    which are scripts/release-notes.sh's output), for /changelog
 *   extensions.json  the extension store's index (extensions/index.toml at
 *                    this commit), for /extensions
 *
 *   bun scripts/data.ts            # all three
 *   bun scripts/data.ts release    # one or more by name
 *
 * FAIL-SOFT by default: a source that cannot be read (GitHub unreachable,
 * rate limited) keeps the committed file and prints a warning (a GitHub
 * Actions `::warning::` in CI); a local or PR build never fails because
 * GitHub was unreachable.
 *
 * STRICT for a release: with SITE_RELEASE_TAG=vX.Y.Z (scripts/web-bundle.sh
 * passes ARCHITECT_RELEASE_TAG through, release.yml's `web` job sets it)
 * /download describes THAT release (not GitHub's `latest`, which a hotfix
 * of an older line is not) and every failure exits 1: the release must
 * exist and /changelog must list it with its notes. Both are made by
 * release.yml's `release` job, the only one the bundle waits for; the
 * binaries are not asked for (they may still be building).
 *
 * GITHUB_TOKEN (or GH_TOKEN), when set, is sent to the API: unauthenticated
 * calls share a 60-an-hour limit per address. Drafts are never listed.
 */
import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { marked } from "marked";

const REPO = "FastTrackStudios/architect";
const CHANGELOG_RELEASES = 20;

const here = dirname(fileURLToPath(import.meta.url));
const site = join(here, "..");
const repoRoot = join(site, "..", "..");
const dataDir = join(site, "src", "data");

const inCi = process.env.GITHUB_ACTIONS === "true";
/** The release this build is for (strict), or "" (fail-soft, `latest`). */
const releaseTag = process.env.SITE_RELEASE_TAG ?? "";
function warn(message: string) {
	if (releaseTag) {
		console.error(
			inCi
				? `::error::site data (${releaseTag}): ${message}`
				: `error: site data (${releaseTag}): ${message}`,
		);
		process.exit(1);
	}
	console.warn(
		inCi ? `::warning::site data: ${message}` : `warning: ${message}`,
	);
}

const token = process.env.GITHUB_TOKEN || process.env.GH_TOKEN || "";

async function getJson<T>(url: string, api = false): Promise<T> {
	const headers: Record<string, string> = {
		"user-agent": "architect-site-build",
	};
	if (api) {
		headers.accept = "application/vnd.github+json";
		if (token) headers.authorization = `Bearer ${token}`;
	}
	const res = await fetch(url, {
		headers,
		redirect: "follow",
		signal: AbortSignal.timeout(20_000),
	});
	if (!res.ok) throw new Error(`${url}: HTTP ${res.status}`);
	return (await res.json()) as T;
}

function write(name: string, value: unknown) {
	writeFileSync(join(dataDir, name), `${JSON.stringify(value, null, "\t")}\n`);
	console.log(`site data: wrote src/data/${name}`);
}

type GhRelease = {
	tag_name: string;
	name: string | null;
	html_url: string;
	published_at: string | null;
	created_at: string;
	draft: boolean;
	prerelease: boolean;
	body: string | null;
};

/** A release by tag, drafts included: page through the list (a draft has no
 * tag yet, so `releases/tags/<tag>` cannot find it). */
async function findRelease(tag: string): Promise<GhRelease> {
	for (let page = 1; page <= 10; page++) {
		const all = await getJson<GhRelease[]>(
			`https://api.github.com/repos/${REPO}/releases?per_page=100&page=${page}`,
			true,
		);
		const r = all.find((x) => x.tag_name === tag);
		if (r) return r;
		if (all.length < 100) break;
	}
	throw new Error(`${tag} is not among the releases`);
}

/** /download and the landing page: the release (SITE_RELEASE_TAG's, or `latest`). */
async function release() {
	let found: GhRelease;
	try {
		// The release being built is still a DRAFT (release.yml publishes it
		// once `sums` has signed every asset), and `releases/tags/<tag>` does
		// not return drafts: find it in the list, which shows drafts to the
		// release job's token.
		found = releaseTag
			? await findRelease(releaseTag)
			: await getJson<GhRelease>(
					`https://api.github.com/repos/${REPO}/releases/latest`,
					true,
				);
	} catch (e) {
		warn(`the release could not be read (${e}); keeping src/data/release.json`);
		return;
	}
	if (releaseTag && found.tag_name !== releaseTag)
		warn(`GitHub answered ${found.tag_name} for ${releaseTag}`);
	write("release.json", {
		tag: found.tag_name,
		version: found.tag_name.replace(/^v/, ""),
		// A draft's html_url is an `untagged-…` page: link the tag's page,
		// which is the release once it is published.
		url: found.draft
			? `https://github.com/${REPO}/releases/tag/${found.tag_name}`
			: found.html_url,
		publishedAt: found.published_at ?? found.created_at,
	});
}

/** /changelog: the last releases' notes, rendered to HTML here, at build time. */
async function changelog() {
	let releases: GhRelease[];
	try {
		releases = await getJson<GhRelease[]>(
			`https://api.github.com/repos/${REPO}/releases?per_page=${CHANGELOG_RELEASES + 10}`,
			true,
		);
	} catch (e) {
		warn(
			`the releases could not be read (${e}); keeping src/data/changelog.json`,
		);
		return;
	}
	const listed = releases
		// Published ones, and this build's own release while it is a draft.
		.filter(
			(r) =>
				(!r.draft && r.published_at) ||
				(releaseTag && r.tag_name === releaseTag),
		)
		.slice(0, CHANGELOG_RELEASES)
		.map((r) => ({
			tag: r.tag_name,
			name: r.name || r.tag_name,
			url: r.draft
				? `https://github.com/${REPO}/releases/tag/${r.tag_name}`
				: r.html_url,
			publishedAt: r.published_at ?? r.created_at,
			prerelease: r.prerelease,
			html: marked.parse(r.body ?? "", { async: false, gfm: true }),
		}));
	if (releaseTag) {
		const own = listed.find((r) => r.tag === releaseTag);
		if (!own) warn(`${releaseTag} is not among the releases listed`);
		else if (!releases.find((r) => r.tag_name === releaseTag)?.body?.trim())
			warn(`${releaseTag}'s release has no notes`);
	}
	write("changelog.json", { repo: REPO, releases: listed });
}

type IndexEntry = {
	id: string;
	name?: string;
	version: string;
	kind: "data" | "luau" | "wasm" | "process";
	description?: string;
	category?: string;
	path?: string;
	git?: string;
	rev?: string;
	archive?: string;
	sha256?: string;
	platforms?: string[];
	changelog?: string;
};
type Index = { name?: string; extensions?: IndexEntry[] };

function semverCmp(a: string, b: string) {
	const [ca, pa = ""] = a.split("+")[0].split("-", 2);
	const [cb, pb = ""] = b.split("+")[0].split("-", 2);
	const na = ca.split(".").map(Number);
	const nb = cb.split(".").map(Number);
	for (let i = 0; i < 3; i++) if (na[i] !== nb[i]) return na[i] - nb[i];
	if (pa === pb) return 0;
	if (!pa) return 1;
	if (!pb) return -1;
	return pa < pb ? -1 : 1;
}

/**
 * /extensions: the store's index (apps/distribution `STORE_REGISTRY`,
 * `STORE_DIR`: extensions/index.toml in this repository), read from this
 * checkout so the site lists what the commit it is built from lists. One
 * `[[extensions]]` entry per version (examples/registry/index.schema.json);
 * the page shows each id's newest. While the store lists nothing, the
 * page is built from the example registry (examples/registry/index.toml),
 * marked as a fixture.
 */
function extensions() {
	const store = join(repoRoot, "extensions", "index.toml");
	const example = join(repoRoot, "examples", "registry", "index.toml");
	if (!existsSync(store)) {
		warn(`${store} is not there; keeping src/data/extensions.json`);
		return;
	}
	let index: Index;
	let source: "store" | "fixture" = "store";
	let file = "extensions/index.toml";
	try {
		index = Bun.TOML.parse(readFileSync(store, "utf8")) as Index;
		if (!index.extensions?.length) {
			index = Bun.TOML.parse(readFileSync(example, "utf8")) as Index;
			source = "fixture";
			file = "examples/registry/index.toml";
		}
	} catch (e) {
		warn(
			`the extension index could not be read (${e}); keeping src/data/extensions.json`,
		);
		return;
	}
	const byId = new Map<string, IndexEntry[]>();
	for (const e of index.extensions ?? []) {
		byId.set(e.id, [...(byId.get(e.id) ?? []), e]);
	}
	const entries = [...byId.values()]
		.map((versions) => {
			versions.sort((a, b) => semverCmp(b.version, a.version));
			const e = versions[0];
			return {
				id: e.id,
				name: e.name ?? e.id,
				version: e.version,
				kind: e.kind,
				description: e.description ?? "",
				category: e.category ?? "",
				// The schema has no publisher field: an id is reverse-DNS
				// (`org.name`), so its first part names who publishes it.
				publisher: e.id.split(".")[0],
				source: e.archive
					? { type: "archive", archive: e.archive, sha256: e.sha256 }
					: e.git
						? { type: "git", git: e.git, rev: e.rev }
						: { type: "path", path: e.path },
				platforms: e.platforms ?? [],
				changelog: e.changelog ?? "",
				versions: versions.map((v) => ({
					version: v.version,
					changelog: v.changelog ?? "",
				})),
			};
		})
		.sort((a, b) => a.name.localeCompare(b.name));
	write("extensions.json", {
		registry: index.name ?? "FastTrackStudios",
		source,
		file,
		entries,
	});
}

const jobs: Record<string, () => unknown> = { release, changelog, extensions };
const wanted = process.argv.slice(2);
for (const name of wanted.length ? wanted : Object.keys(jobs)) {
	const job = jobs[name];
	if (!job) {
		console.error(
			`site data: no source named ${name} (${Object.keys(jobs).join(", ")})`,
		);
		process.exit(2);
	}
	await job();
}
