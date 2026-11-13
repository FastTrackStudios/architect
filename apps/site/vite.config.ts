import { tanstackStart } from "@tanstack/react-start/plugin/vite";
import viteReact from "@vitejs/plugin-react";
import { nitro } from "nitro/vite";
import { defineConfig, type Plugin } from "vite";

import { PAGES, SKIP_PRERENDER } from "./src/content/pages.ts";

/**
 * Every page is prerendered: `.output/public` is the root of the web
 * bundle (scripts/web-bundle.sh), plain static files, so no page needs a
 * server at request time.
 * The Nitro server build exists because the hub's toolchain makes one;
 * it is used only to render the pages at build time and is never deployed.
 */
/**
 * On the local stack (`devenv up web site`, docs/content/guide/local-stack.md)
 * the web app has a dev server of its own (`dx serve`), named by
 * ARCHITECT_WEB_DEV: this one sends `/app/…` there, so the Demo and
 * "Open the web app" links lead to it as they do in the bundle, where
 * both are one origin. Dev server only; unset, nothing changes.
 */
const webApp = (): Plugin => ({
	name: "architect-web-dev",
	apply: "serve",
	configureServer(server) {
		const app = process.env.ARCHITECT_WEB_DEV?.replace(/\/$/, "");
		if (!app) return;
		server.middlewares.use((req, res, next) => {
			const url = req.url ?? "";
			if (
				url !== "/app" &&
				!url.startsWith("/app/") &&
				!url.startsWith("/app?")
			)
				return next();
			res.statusCode = 302;
			res.setHeader("Location", app + url);
			res.end();
		});
	},
});

const config = defineConfig({
	resolve: { tsconfigPaths: true },
	plugins: [
		webApp(),
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
