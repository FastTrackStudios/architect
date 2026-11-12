/**
 * Serves a static folder with exactly the rules the web bundle needs
 * (scripts/web-bundle.sh, its ROUTES.md): the reference for what a
 * production server must do, used to look at a bundle locally and by
 * scripts/check-web-bundle.sh.
 *
 *   bun scripts/serve.ts <dir> [port]     # default .output/public, 4173
 *
 * For a request path P (percent-decoded, never above the root):
 *   1. P itself, if it is a file
 *   2. P.html, if it is a file              (/download -> download.html)
 *   3. P/index.html, if it is a file        (/, /docs/, /app/)
 *   4. else 404.html, with status 404
 * The query string is ignored. No redirects, no rewrites, no headers: the
 * node's paths (/app's sign-in callback, /s/, /mcp, …) are the production
 * server's business, before this.
 */
import { existsSync, statSync } from "node:fs";
import { join, normalize, resolve } from "node:path";

const dir = resolve(process.argv[2] ?? ".output/public");
const port = Number(process.argv[3] ?? 4173);

const isFile = (p: string) => existsSync(p) && statSync(p).isFile();

/** The file a request path is served from, or null (404). */
export function resolvePath(root: string, path: string): string | null {
	let rel: string;
	try {
		rel = decodeURIComponent(path);
	} catch {
		return null;
	}
	const p = normalize(join(root, rel));
	if (p !== root && !p.startsWith(`${root}/`)) return null;
	// /download/ is not /download: only a folder's URL ends in a slash.
	const html = rel.endsWith("/") ? [] : [`${p}.html`];
	return [p, ...html, join(p, "index.html")].find(isFile) ?? null;
}

if (import.meta.main) {
	if (!isFile(join(dir, "404.html"))) {
		console.error(`serve: ${dir}/404.html is missing`);
		process.exit(2);
	}
	const server = Bun.serve({
		port,
		hostname: "127.0.0.1",
		fetch(req) {
			const f = resolvePath(dir, new URL(req.url).pathname);
			if (f) return new Response(Bun.file(f));
			return new Response(Bun.file(join(dir, "404.html")), {
				status: 404,
				headers: { "content-type": "text/html; charset=utf-8" },
			});
		},
	});
	console.log(`serve: http://127.0.0.1:${server.port}/ (${dir})`);
}
