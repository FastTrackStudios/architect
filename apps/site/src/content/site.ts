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
	homeTitle: "Architect: your notes, projects and files on every device",
	description:
		"Architect is a local-first app for desktop, phone and the web, and the Rust framework it is built on. Vaults of markdown, projects and files held at home and pinned where you work, synced peer to peer.",
	/** og:description on the home page, as the landing page had it. */
	ogDescription:
		"Your notes, projects and files on every device, local-first and in sync.",
	/** Painted as <meta name="theme-color">: the plot (light) and the model (dark). */
	themeColor: { plot: "#f3f5f2", model: "#0c0e10" },
	/** Where the Plot/Model choice is kept (the landing page's key, unchanged). */
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

/** The pages' own nav, beside the landing page's sheet index. */
export const NAV_LINKS = [
	{ label: "Download", to: "/download" },
	{ label: "Extensions", to: "/extensions" },
	{ label: "Changelog", to: "/changelog" },
] as const;
