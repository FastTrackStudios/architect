/**
 * Who this site is, and where everything it links to lives. Isomorphic —
 * safe to import from client and server.
 *
 * Every page's shell (bar, footer, SEO defaults) reads from here, so a
 * redesign changes markup without hunting for hardcoded strings.
 */

export const SITE = {
	name: "Architect",
	domain: "architect.fasttrackstudio.app",
	url: "https://architect.fasttrackstudio.app",
	/** The home page's own title (the landing page's, verbatim). */
	homeTitle: "Architect: build the systems that work for you",
	description:
		"Architect is a modular, local-first, collaborative foundation: one open-source app where every feature is an extension, written in Rust, Lua or anything that compiles to WASM. For Linux and macOS.",
	/** og:description on the home page. */
	ogDescription:
		"A modular, local-first, collaborative foundation to build the systems that work for you.",
	/** Painted as <meta name="theme-color">: the grounds in src/styles/tokens.css. */
	themeColor: { light: "#ffffff", dark: "#0e0e10" },
	/** Where the light/dark choice is kept (the old page's key; it kept "plot"/"model"). */
	themeKey: "architect-plot",
	locale: "en_US",
	license: "MIT or Apache-2.0",
	repo: "https://github.com/FastTrackStudios/architect",
	org: "https://github.com/FastTrackStudios",
} as const;

/**
 * OPEN DECISION (#534): the node's public hostname once this site owns
 * `architect.fasttrackstudio.app`. The web app (`/app/…`) and share links
 * (`/s/…`) are served by the node, never through Vercel; this site only
 * REDIRECTS them here, path and query kept (scripts/assemble.sh reads this
 * constant). A placeholder until the hostname and its DNS are decided.
 */
export const NODE_ORIGIN = "https://app.architect.fasttrackstudio.app";

/** The paths the node owns: redirected there, never rendered here. */
export const NODE_PATHS = ["/app", "/s"] as const;

/** The docs: `docs/content` built by `architect site build`, copied in at /docs/. */
export const DOCS_PATH = "/docs/";

/** The release that `latest` means on GitHub. */
export const RELEASES_LATEST = `${SITE.repo}/releases/latest`;

/** The header's nav, on every page. */
export const NAV_LINKS = [
	{ label: "Download", to: "/download" },
	{ label: "Extensions", to: "/extensions" },
	{ label: "Changelog", to: "/changelog" },
] as const;

/** The Nix one-liner (the flake's default package is the release's AppImage). */
export const NIX_RUN = "nix run github:FastTrackStudios/architect";

/** The words of the shell: header, footer, theme switch. */
export const SHELL = {
	skip: "Skip to the content",
	home: "Architect, home",
	nav: "Site",
	docs: { label: "Docs", href: DOCS_PATH },
	theme: {
		toLight: "Switch to the light theme",
		toDark: "Switch to the dark theme",
	},
	footer: {
		by: {
			pre: "By",
			org: "FastTrackStudio",
			href: "https://fasttrackstudio.app",
		},
		tail: `${SITE.license}. Pre-1.0.`,
		links: [
			{ label: "Source", href: SITE.repo },
			{ label: "Releases", href: `${SITE.repo}/releases` },
			{ label: "Docs", href: DOCS_PATH },
		],
	},
} as const;
