import { tanstackStart } from "@tanstack/react-start/plugin/vite";
import viteReact from "@vitejs/plugin-react";
import { nitro } from "nitro/vite";
import { defineConfig } from "vite";

import { PAGES, SKIP_PRERENDER } from "./src/content/pages.ts";

/**
 * Every page is prerendered: `.output/public` is the root of the web
 * bundle (scripts/web-bundle.sh), plain static files, so no page needs a
 * server at request time.
 * The Nitro server build exists because the hub's toolchain makes one;
 * it is used only to render the pages at build time and is never deployed.
 */
const config = defineConfig({
	resolve: { tsconfigPaths: true },
	plugins: [
		nitro(),
		tanstackStart({
			prerender: {
				enabled: true,
				crawlLinks: true,
				// /download.html, not /download/index.html: the bundle's server
				// tries P, then P.html, then P/index.html (scripts/serve.ts).
				autoSubfolderIndex: false,
				failOnError: true,
				// Only PAGES and what they link to: no /extensions/ twin of /extensions.
				autoStaticPathsDiscovery: false,
				// Links that leave this app (the docs, the node) are not crawled.
				filter: ({ path }) => !SKIP_PRERENDER.some((p) => path.startsWith(p)),
			},
			pages: PAGES.map((path) => ({ path })),
		}),
		viteReact(),
	],
});

export default config;
