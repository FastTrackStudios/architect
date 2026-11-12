/**
 * The pages prerendered at build time (vite.config.ts) besides what the
 * crawl finds from them. Every extension's page is listed too, so a page
 * no link reaches is still built.
 */
import extensions from "../data/extensions.json" with { type: "json" };

export const PAGES = [
	"/",
	"/download",
	"/changelog",
	"/extensions",
	"/404",
	...extensions.entries.map((e) => `/extensions/${e.id}`),
];

/**
 * Links that leave this app: the docs (a separate build, copied in by
 * scripts/web-bundle.sh), the node's paths (the web app, share links), and /guides/ (linked by the
 * landing page, served by nothing today). Never crawled.
 */
export const SKIP_PRERENDER = ["/docs", "/app", "/s/", "/guides"];
