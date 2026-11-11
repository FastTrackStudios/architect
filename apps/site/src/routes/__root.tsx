/// <reference types="vite/client" />

import {
	createRootRoute,
	HeadContent,
	Scripts,
	useRouterState,
} from "@tanstack/react-router";

import { Bar } from "#/components/layout/bar";
import { Colophon } from "#/components/layout/colophon";
import { NotFound } from "#/components/layout/not-found";
import { BAR } from "#/content/landing";
import { SITE } from "#/content/site";
import { pageHead } from "#/lib/seo";
import { THEME_SCRIPT } from "#/lib/theme";
import appCss from "#/styles/index.css?url";

/**
 * Root route — owns the whole document (full-document SSR, prerendered at
 * build time). Per-route `head()` merges into the tags below, so a child
 * route overrides the title and description without re-declaring the rest.
 */
export const Route = createRootRoute({
	head: () => {
		const home = pageHead();
		return {
			meta: [
				{ charSet: "utf-8" },
				{ name: "viewport", content: "width=device-width, initial-scale=1" },
				{ name: "color-scheme", content: "light dark" },
				{
					name: "theme-color",
					content: SITE.themeColor.plot,
					media: "(prefers-color-scheme: light)",
				},
				// Its dark twin is in RootDocument: HeadContent keeps one meta per name.
				{ property: "og:type", content: "website" },
				{ property: "og:site_name", content: SITE.name },
				{ property: "og:locale", content: SITE.locale },
				// Defaults, so a page that forgets `pageHead()` is still titled.
				...home.meta,
			],
			links: [
				{ rel: "icon", href: "/mark.svg", type: "image/svg+xml" },
				{
					rel: "preload",
					href: "/fonts/archivo.woff2",
					as: "font",
					type: "font/woff2",
					crossOrigin: "anonymous",
				},
				{
					rel: "preload",
					href: "/fonts/osifont.woff2",
					as: "font",
					type: "font/woff2",
					crossOrigin: "anonymous",
				},
				{ rel: "stylesheet", href: appCss },
			],
			// Before first paint: the `js` class and the kept Plot/Model choice.
			scripts: [{ children: THEME_SCRIPT }],
		};
	},
	notFoundComponent: NotFound,
	shellComponent: RootDocument,
});

function RootDocument({ children }: { children: React.ReactNode }) {
	const home = useRouterState({ select: (s) => s.location.pathname === "/" });
	return (
		// The head script sets `class` and `data-theme` before React hydrates.
		<html lang="en" suppressHydrationWarning>
			<head>
				<HeadContent />
				<meta
					name="theme-color"
					content={SITE.themeColor.model}
					media="(prefers-color-scheme: dark)"
				/>
			</head>
			<body>
				<a className="skip" href="#main">
					{BAR.skip}
				</a>
				<Bar sheets={home} />
				<main id="main">{children}</main>
				<Colophon />
				<svg
					width="0"
					height="0"
					className="defs"
					aria-hidden="true"
					focusable="false"
				>
					<symbol id="hex" viewBox="-12 -11 24 22">
						<path d="M-6 -10 H6 L11.5 0 L6 10 H-6 L-11.5 0 Z" />
					</symbol>
				</svg>
				<Scripts />
			</body>
		</html>
	);
}
