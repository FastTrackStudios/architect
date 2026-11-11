/**
 * Writes the deploy's `.vercel/output/config.json` (Vercel's Build Output
 * API v3: https://vercel.com/docs/build-output-api/configuration) for the
 * static files already in `<output>/static`.
 *
 *   bun scripts/vercel-config.ts <output dir>
 *
 * - routes, in order:
 *   1. the node's paths (NODE_PATHS, NODE_ORIGIN in src/content/site.ts):
 *      `/app`, `/app/…`, `/s`, `/s/…` answer 307 with a Location on the
 *      node, the whole path kept (trailing slash included). The query
 *      string is not in `src` (Vercel matches the pathname only) and
 *      Vercel carries it over to a redirect's Location, as it does for
 *      vercel.json `redirects`, which compile to this same route shape.
 *      The app and share links are never proxied through Vercel (#534).
 *   2. `handle: error`, then every 404 gets the prerendered /404.html with
 *      its 404 status (the route Vercel's own static builder emits).
 * - overrides: each page this app prerendered as `<name>.html` is served
 *   at `/<name>` (the documented clean-URL override). Folders' index.html
 *   files (`/`, every docs page) are served at their folder's URL as they
 *   are; the docs' own links all end in `/`.
 */
import { readdirSync, statSync, writeFileSync } from "node:fs";
import { join, relative } from "node:path";

import { NODE_ORIGIN, NODE_PATHS } from "../src/content/site.ts";

const out = process.argv[2];
if (!out) {
	console.error("usage: bun scripts/vercel-config.ts <.vercel/output>");
	process.exit(2);
}
const staticDir = join(out, "static");

function walk(dir: string): string[] {
	return readdirSync(dir).flatMap((n) => {
		const p = join(dir, n);
		return statSync(p).isDirectory() ? walk(p) : [p];
	});
}

const reEscape = (s: string) => s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");

const routes: object[] = [
	...NODE_PATHS.map((p) => ({
		src: `^${reEscape(p)}(/.*)?$`,
		headers: { Location: `${NODE_ORIGIN}${p}$1` },
		status: 307,
	})),
	{ handle: "error" },
	{ status: 404, src: "^(?!/api).*$", dest: "/404.html" },
];

const overrides: Record<string, { path: string }> = {};
for (const f of walk(staticDir)) {
	const rel = relative(staticDir, f);
	if (!rel.endsWith(".html") || rel.endsWith("index.html")) continue;
	if (rel.startsWith("docs/") || rel === "404.html") continue;
	overrides[rel] = { path: rel.slice(0, -".html".length) };
}

const config = {
	version: 3,
	routes,
	overrides,
	framework: { slug: "tanstack-start", version: "1.168.49" },
};
writeFileSync(join(out, "config.json"), `${JSON.stringify(config, null, 2)}\n`);
console.log(
	`vercel-config: ${join(out, "config.json")} (${routes.length} routes, ${Object.keys(overrides).length} clean URLs)`,
);
