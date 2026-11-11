/**
 * Every internal link and asset in the built site resolves.
 *
 *   bun scripts/check-links.ts <static dir>
 *
 * Reads each page this app built (every .html outside docs/) and each CSS
 * file, and checks every href, src, poster and url() that stays on the
 * site: the file exists (with the deploy's clean URLs: /download is
 * download.html, a folder is its index.html), and a #fragment names an id
 * on the page it points at. Not checked: other hosts; the node's paths
 * (/app, /s/: redirects, scripts/vercel-config.ts); /docs/… when the
 * folder has no docs/ (a PR build has no `architect` CLI to build them);
 * and the docs' own pages (their builder checks those). KNOWN_DEAD lists
 * links the ported landing page already had that resolve nowhere today;
 * they are reported, not failed.
 */
import { existsSync, readdirSync, readFileSync, statSync } from "node:fs";
import { join, relative } from "node:path";

import { NODE_PATHS } from "../src/content/site.ts";

/** /guides/ is linked by apps/web/landing too, and 404s on the live site. */
const KNOWN_DEAD = ["/guides/"];

const root = process.argv[2];
if (!root || !existsSync(root)) {
	console.error("usage: bun scripts/check-links.ts <static dir>");
	process.exit(2);
}
const hasDocs = existsSync(join(root, "docs", "index.html"));

function walk(dir: string): string[] {
	return readdirSync(dir).flatMap((n) => {
		const p = join(dir, n);
		return statSync(p).isDirectory() ? walk(p) : [p];
	});
}

/** The file a site path is served from, or null. */
function fileFor(path: string): string | null {
	const p = decodeURIComponent(path).replace(/\/+$/, "");
	const candidates = [
		join(root, p),
		join(root, `${p}.html`),
		join(root, p, "index.html"),
	];
	return candidates.find((c) => existsSync(c) && statSync(c).isFile()) ?? null;
}

const ids = new Map<string, Set<string>>();
function idsOf(file: string) {
	let s = ids.get(file);
	if (!s) {
		s = new Set(
			[...readFileSync(file, "latin1").matchAll(/\sid="([^"]+)"/g)].map(
				(m) => m[1],
			),
		);
		ids.set(file, s);
	}
	return s;
}

const files = walk(root);
const pages = files.filter(
	(f) => f.endsWith(".html") && !relative(root, f).startsWith("docs/"),
);
const sheets = files.filter(
	(f) => f.endsWith(".css") && !relative(root, f).startsWith("docs/"),
);
const problems: string[] = [];
const dead = new Set<string>();
let checked = 0;

function check(from: string, raw: string, base: string) {
	if (/^(https?:|mailto:|data:|javascript:|\/\/)/.test(raw)) return;
	const url = new URL(raw, `https://site.invalid${base}`);
	const path = url.pathname;
	if (NODE_PATHS.some((p) => path === p || path.startsWith(`${p}/`))) return;
	if (KNOWN_DEAD.includes(path)) {
		dead.add(path);
		return;
	}
	if ((path === "/docs" || path.startsWith("/docs/")) && !hasDocs) return;
	checked++;
	const target = fileFor(path);
	if (!target) {
		problems.push(`${relative(root, from)}: ${raw} -> nothing at ${path}`);
		return;
	}
	const frag = decodeURIComponent(url.hash.slice(1));
	if (frag && target.endsWith(".html") && !idsOf(target).has(frag)) {
		problems.push(
			`${relative(root, from)}: ${raw} -> no id "${frag}" on ${relative(root, target)}`,
		);
	}
}

for (const page of pages) {
	const html = readFileSync(page, "latin1");
	const rel = `/${relative(root, page)}`;
	// A page is served at its clean URL; relative links resolve from there.
	const base = rel.endsWith("/index.html")
		? rel.slice(0, -10)
		: rel.replace(/\.html$/, "");
	for (const m of html.matchAll(/\s(?:href|src|poster)="([^"]*)"/g)) {
		check(page, m[1].replaceAll("&amp;", "&"), base);
	}
}
for (const sheet of sheets) {
	const css = readFileSync(sheet, "latin1");
	for (const m of css.matchAll(/url\(\s*["']?([^"')]+)["']?\s*\)/g)) {
		if (m[1].startsWith("#") || m[1].startsWith("data:")) continue;
		check(sheet, m[1], `/${relative(root, sheet)}`);
	}
}

for (const d of dead)
	console.warn(`known dead (linked by the ported landing page): ${d}`);
if (problems.length) {
	for (const p of problems) console.error(`broken: ${p}`);
	console.error(`check-links: ${problems.length} broken of ${checked} checked`);
	process.exit(1);
}
console.log(
	`check-links: ${checked} links and assets resolve (${pages.length} pages, ${sheets.length} stylesheets${hasDocs ? ", docs included" : ", /docs/ not built here, skipped"})`,
);
