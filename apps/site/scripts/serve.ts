/**
 * Serves a `.vercel/output` folder locally the way its config.json says,
 * for checking a deploy before it is one:
 *
 *   bun scripts/serve.ts .vercel/output [port]
 *
 * It applies config.json's routes before any `handle` (redirects: status
 * + Location, the request's query carried over as Vercel does), then
 * serves static/: a file by its name, a page by its `overrides` path, a
 * folder by its index.html; anything else gets 404.html and a 404. Not a Vercel emulator: enough to
 * read every page and see every redirect.
 */
import { existsSync, statSync } from "node:fs";
import { join } from "node:path";

const dir = process.argv[2] ?? ".vercel/output";
const port = Number(process.argv[3] ?? 4173);
type Route = {
	src?: string;
	status?: number;
	headers?: Record<string, string>;
	handle?: string;
};
const config = (await Bun.file(join(dir, "config.json")).json()) as {
	routes: Route[];
	overrides?: Record<string, { path?: string }>;
};
const before: Route[] = [];
for (const r of config.routes) {
	if (r.handle) break;
	before.push(r);
}
/** `overrides`: the URL path (no leading slash) each file is served at. */
const byPath = new Map(
	Object.entries(config.overrides ?? {}).flatMap(([f, o]) =>
		o.path ? [[o.path, f]] : [],
	),
);
const staticDir = join(dir, "static");

function file(path: string) {
	const rel = decodeURIComponent(path).replace(/^\//, "");
	const over = byPath.get(rel);
	if (over) return join(staticDir, over);
	if (byPath.size && [...byPath.values()].includes(rel)) return null; // served at its clean path only
	const p = join(staticDir, rel);
	for (const c of [p, join(p, "index.html")]) {
		if (existsSync(c) && statSync(c).isFile()) return c;
	}
	return null;
}

const server = Bun.serve({
	port,
	hostname: "127.0.0.1",
	fetch(req) {
		const url = new URL(req.url);
		for (const r of before) {
			if (!r.src || !r.status || !r.headers?.Location) continue;
			const m = new RegExp(r.src).exec(url.pathname);
			if (!m) continue;
			let loc = r.headers.Location.replace(
				/\$(\d+)/g,
				(_, i) => m[Number(i)] ?? "",
			);
			for (const [k, v] of Object.entries(m.groups ?? {})) {
				loc = loc.replaceAll(`$${k}`, v ?? "");
			}
			if (url.search)
				loc += (loc.includes("?") ? "&" : "?") + url.search.slice(1);
			return new Response(null, {
				status: r.status,
				headers: { Location: loc },
			});
		}
		const f = file(url.pathname);
		if (f) return new Response(Bun.file(f));
		return new Response(Bun.file(join(staticDir, "404.html")), {
			status: 404,
			headers: { "content-type": "text/html; charset=utf-8" },
		});
	},
});
console.log(`serve: http://127.0.0.1:${server.port}/ (${dir})`);
